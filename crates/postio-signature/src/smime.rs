use std::ops::Range;

use base64::Engine;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::{OctetString, SetOfVec};
use der::oid::ObjectIdentifier;
use der::oid::db::rfc5280::{ID_CE_EXT_KEY_USAGE, ID_CE_SUBJECT_ALT_NAME};
use der::oid::db::rfc5911::{ID_MESSAGE_DIGEST, ID_SIGNING_TIME};
use der::{Decode, Encode};
use p256::ecdsa::signature::Verifier;
use pki_types::{
    AlgorithmIdentifier, CertificateDer, InvalidSignature, SignatureVerificationAlgorithm,
    UnixTime, alg_id,
};
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use sha2::{Digest, Sha256, Sha384, Sha512};
use x509_cert::Certificate;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, SubjectAltName};

use crate::policy::{Hash, Reason, Status};
use crate::scope::SignedScope;
use crate::verify::{KeySet, Verdict};

const EMAIL_PROTECTION: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x04];
const SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const MD5: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.5");
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");

pub fn anchors_in(bytes: &[u8]) -> Vec<Vec<u8>> {
    if bytes.first() == Some(&0x30) && Certificate::from_der(bytes).is_ok() {
        return vec![bytes.to_vec()];
    }
    let text = String::from_utf8_lossy(bytes);
    let mut found = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
        let after = &rest[start..];
        let Some(end) = after.find("-----END CERTIFICATE-----") else {
            break;
        };
        let block = &after[..end + "-----END CERTIFICATE-----".len()];
        if let Ok((_, der)) = der::pem::decode_vec(block.as_bytes())
            && Certificate::from_der(&der).is_ok()
        {
            found.push(der);
        }
        rest = &after[block.len()..];
    }
    found
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerCertificate {
    pub email: Option<String>,
    pub chain: Vec<String>,
}

pub fn unchecked_certificate(raw: &[u8]) -> Option<SignerCertificate> {
    let (SignedScope::WholeMessage {
        format: crate::scope::Format::Smime,
        signature,
        ..
    }
    | SignedScope::Part {
        format: crate::scope::Format::Smime,
        signature,
        ..
    }) = crate::scope::signed_scope(raw)?
    else {
        return None;
    };
    let (signed_data, signer) = signer_info(raw.get(signature)?)?;
    let SignerIdentifier::IssuerAndSerialNumber(id) = &signer.sid else {
        return None;
    };
    let certificates: Vec<Certificate> = signed_data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            cms::cert::CertificateChoices::Certificate(certificate) => Some(certificate.clone()),
            _ => None,
        })
        .collect();
    let leaf = certificates.iter().find(|certificate| {
        certificate.tbs_certificate.serial_number == id.serial_number
            && certificate.tbs_certificate.issuer == id.issuer
    })?;
    let email = extension::<SubjectAltName>(leaf, ID_CE_SUBJECT_ALT_NAME).and_then(|names| {
        names.0.into_iter().find_map(|name| match name {
            GeneralName::Rfc822Name(address) => Some(address.to_string()),
            _ => None,
        })
    });
    let mut chain = vec![common_name(&leaf.tbs_certificate.subject)];
    let mut at = leaf;
    while chain.len() < 8 {
        let issuer = &at.tbs_certificate.issuer;
        if *issuer == at.tbs_certificate.subject {
            break;
        }
        chain.push(common_name(issuer));
        match certificates
            .iter()
            .find(|certificate| certificate.tbs_certificate.subject == *issuer)
        {
            Some(next) => at = next,
            None => break,
        }
    }
    Some(SignerCertificate { email, chain })
}

fn common_name(name: &x509_cert::name::Name) -> String {
    let shown = name.to_string();
    shown
        .split(',')
        .find_map(|part| part.trim().strip_prefix("CN="))
        .map_or(shown.clone(), str::to_owned)
}

pub(crate) fn check(
    scope: &SignedScope,
    raw: &[u8],
    part: Range<usize>,
    signature: Range<usize>,
    keys: &KeySet,
) -> Verdict {
    let verdict = |status, reason, signer: Option<String>, signed_at| Verdict {
        status,
        reason,
        scope: scope.clone(),
        signer,
        signed_at,
    };
    let Some((signed_data, signer)) = signer_info(&raw[signature]) else {
        return verdict(Status::CantCheck, Some(Reason::Malformed), None, None);
    };
    let certificates: Vec<Certificate> = signed_data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            cms::cert::CertificateChoices::Certificate(certificate) => Some(certificate.clone()),
            _ => None,
        })
        .collect();
    let SignerIdentifier::IssuerAndSerialNumber(id) = &signer.sid else {
        return verdict(Status::CantCheck, Some(Reason::Unsupported), None, None);
    };
    let Some(leaf) = certificates.iter().find(|certificate| {
        certificate.tbs_certificate.serial_number == id.serial_number
            && certificate.tbs_certificate.issuer == id.issuer
    }) else {
        return verdict(Status::CantCheck, Some(Reason::NoKey), None, None);
    };
    let Ok(leaf_der) = leaf.to_der() else {
        return verdict(Status::CantCheck, Some(Reason::Malformed), None, None);
    };
    let fingerprint = Some(hex::upper(&Sha256::digest(&leaf_der)));
    let signed_at = signing_time(&signer);
    let at = |status, reason| verdict(status, reason, fingerprint.clone(), signed_at);

    let digest = match signer.digest_alg.oid {
        SHA256 => Hash::Sha256,
        SHA384 => Hash::Sha384,
        SHA512 => Hash::Sha512,
        SHA1 => return at(Status::CantCheck, Some(Reason::WeakHash(Hash::Sha1))),
        MD5 => return at(Status::CantCheck, Some(Reason::WeakHash(Hash::Md5))),
        _ => return at(Status::CantCheck, Some(Reason::Unsupported)),
    };
    let spki = &leaf.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid == RSA
        && RsaPublicKey::from_pkcs1_der(spki.subject_public_key.raw_bytes())
            .is_ok_and(|key| key.size() * 8 < 2048)
    {
        return at(Status::CantCheck, Some(Reason::WeakKey));
    }

    let content = crate::verify::crlf(&raw[part]);
    let to_verify = match &signer.signed_attrs {
        Some(attributes) => {
            let stated = attributes
                .iter()
                .find(|attribute| attribute.oid == ID_MESSAGE_DIGEST)
                .and_then(|attribute| attribute.values.iter().next())
                .and_then(|value| value.decode_as::<OctetString>().ok());
            let Some(stated) = stated else {
                return at(Status::CantCheck, Some(Reason::Malformed));
            };
            if stated.as_bytes() != digest_of(digest, &content).as_slice() {
                return at(Status::Bad, Some(Reason::Tampered));
            }
            let set: SetOfVec<_> = attributes.clone();
            let Ok(der) = set.to_der() else {
                return at(Status::CantCheck, Some(Reason::Malformed));
            };
            der
        }
        None => content,
    };
    match signature_holds(&signer, spki, digest, &to_verify) {
        None => return at(Status::CantCheck, Some(Reason::Unsupported)),
        Some(false) => return at(Status::Bad, Some(Reason::Tampered)),
        Some(true) => {}
    }

    let anchors: Vec<CertificateDer<'_>> = keys
        .anchors()
        .iter()
        .map(|der| CertificateDer::from(der.as_slice()))
        .collect();
    let trusted: Vec<_> = anchors
        .iter()
        .filter_map(|anchor| webpki::anchor_from_trusted_cert(anchor).ok())
        .collect();
    let others: Vec<CertificateDer<'static>> = certificates
        .iter()
        .filter(|certificate| *certificate != leaf)
        .filter_map(|certificate| certificate.to_der().ok().map(CertificateDer::from))
        .collect();
    let leaf_cert = CertificateDer::from(leaf_der.as_slice());
    let Ok(end_entity) = webpki::EndEntityCert::try_from(&leaf_cert) else {
        return at(Status::CantCheck, Some(Reason::Malformed));
    };
    let Some(when) = signed_at.and_then(|at| u64::try_from(at).ok()) else {
        return at(Status::CantCheck, Some(Reason::Malformed));
    };
    // `required_if_present`: webpki asks every certificate on the path for the EKU, and a CA with
    // no EKU extension is the norm. The leaf itself must carry it, checked below.
    let path = end_entity.verify_for_usage(
        ALGORITHMS,
        &trusted,
        &others,
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(when)),
        webpki::KeyUsage::required_if_present(EMAIL_PROTECTION),
        None,
        None,
    );
    if let Err(error) = path {
        return match error {
            webpki::Error::CertExpired { .. } => at(Status::Bad, Some(Reason::Expired)),
            webpki::Error::CertNotValidYet { .. } => at(Status::Bad, Some(Reason::NotYetValid)),
            webpki::Error::RequiredEkuNotFoundContext(_) => {
                at(Status::CantCheck, Some(Reason::NotForEmail))
            }
            webpki::Error::UnsupportedSignatureAlgorithmContext(_)
            | webpki::Error::UnsupportedSignatureAlgorithmForPublicKeyContext(_) => {
                at(Status::CantCheck, Some(Reason::Unsupported))
            }
            _ => at(Status::CantCheck, Some(Reason::Untrusted)),
        };
    }
    if !extension::<ExtendedKeyUsage>(leaf, ID_CE_EXT_KEY_USAGE)
        .is_some_and(|usage| usage.0.iter().any(|oid| oid.as_bytes() == EMAIL_PROTECTION))
    {
        return at(Status::CantCheck, Some(Reason::NotForEmail));
    }
    let named: Vec<String> = extension::<SubjectAltName>(leaf, ID_CE_SUBJECT_ALT_NAME)
        .map(|names| {
            names
                .0
                .into_iter()
                .filter_map(|name| match name {
                    GeneralName::Rfc822Name(address) => Some(address.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let from = from_address(raw);
    if !from.as_deref().is_some_and(|from| {
        named
            .iter()
            .any(|address| address.eq_ignore_ascii_case(from))
    }) {
        return at(Status::Bad, Some(Reason::AddressMismatch));
    }
    at(Status::Good, None)
}

fn signer_info(body: &[u8]) -> Option<(SignedData, SignerInfo)> {
    let text: Vec<u8> = body
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()?;
    let info = ContentInfo::from_der(&der).ok()?;
    let signed_data: SignedData = info.content.decode_as().ok()?;
    let mut signers = signed_data.signer_infos.0.iter();
    let signer = signers.next()?.clone();
    Some((signed_data.clone(), signer))
}

fn signing_time(signer: &SignerInfo) -> Option<i64> {
    let value = signer
        .signed_attrs
        .as_ref()?
        .iter()
        .find(|attribute| attribute.oid == ID_SIGNING_TIME)?
        .values
        .iter()
        .next()?
        .to_der()
        .ok()?;
    let value = x509_cert::time::Time::from_der(&value).ok()?;
    i64::try_from(value.to_unix_duration().as_secs()).ok()
}

fn digest_of(hash: Hash, bytes: &[u8]) -> Vec<u8> {
    match hash {
        Hash::Sha384 => Sha384::digest(bytes).to_vec(),
        Hash::Sha512 => Sha512::digest(bytes).to_vec(),
        _ => Sha256::digest(bytes).to_vec(),
    }
}

fn signature_holds(
    signer: &SignerInfo,
    spki: &x509_cert::spki::SubjectPublicKeyInfoOwned,
    digest: Hash,
    message: &[u8],
) -> Option<bool> {
    let signature = signer.signature.as_bytes();
    let key = spki.subject_public_key.raw_bytes();
    match signer.signature_algorithm.oid {
        RSA | RSA_SHA256 | RSA_SHA384 | RSA_SHA512 => {
            let key = RsaPublicKey::from_pkcs1_der(key).ok()?;
            let hashed = digest_of(digest, message);
            let scheme = match digest {
                Hash::Sha384 => Pkcs1v15Sign::new::<Sha384>(),
                Hash::Sha512 => Pkcs1v15Sign::new::<Sha512>(),
                _ => Pkcs1v15Sign::new::<Sha256>(),
            };
            Some(key.verify(scheme, &hashed, signature).is_ok())
        }
        ECDSA_SHA256 => {
            let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key).ok()?;
            let signature = p256::ecdsa::Signature::from_der(signature).ok()?;
            Some(key.verify(message, &signature).is_ok())
        }
        ECDSA_SHA384 => {
            let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(key).ok()?;
            let signature = p384::ecdsa::Signature::from_der(signature).ok()?;
            Some(key.verify(message, &signature).is_ok())
        }
        _ => None,
    }
}

fn extension<T: for<'a> Decode<'a>>(certificate: &Certificate, oid: ObjectIdentifier) -> Option<T> {
    certificate
        .tbs_certificate
        .extensions
        .iter()
        .flatten()
        .find(|extension| extension.extn_id == oid)
        .and_then(|extension| T::from_der(extension.extn_value.as_bytes()).ok())
}

fn from_address(raw: &[u8]) -> Option<String> {
    let end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|window| window == b"\n\n"))
        .unwrap_or(raw.len());
    let headers = String::from_utf8_lossy(&raw[..end]);
    let mut from: Option<String> = None;
    for line in headers.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(value) = from.as_mut() {
                value.push_str(line);
            }
            continue;
        }
        if from.is_some() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("from")
        {
            from = Some(value.to_owned());
        }
    }
    let value = from?;
    let address = match (value.rfind('<'), value.rfind('>')) {
        (Some(open), Some(close)) if open < close => &value[open + 1..close],
        _ => value.trim(),
    };
    Some(address.trim().to_ascii_lowercase())
}

static ALGORITHMS: &[&dyn SignatureVerificationAlgorithm] = &[
    &RsaPkcs1 {
        signature: alg_id::RSA_PKCS1_SHA256,
        hash: Hash::Sha256,
    },
    &RsaPkcs1 {
        signature: alg_id::RSA_PKCS1_SHA384,
        hash: Hash::Sha384,
    },
    &RsaPkcs1 {
        signature: alg_id::RSA_PKCS1_SHA512,
        hash: Hash::Sha512,
    },
    &EcdsaP256,
    &EcdsaP384,
];

#[derive(Debug)]
struct RsaPkcs1 {
    signature: AlgorithmIdentifier,
    hash: Hash,
}

impl SignatureVerificationAlgorithm for RsaPkcs1 {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::RSA_ENCRYPTION
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        self.signature
    }

    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let key = RsaPublicKey::from_pkcs1_der(public_key).map_err(|_| InvalidSignature)?;
        let scheme = match self.hash {
            Hash::Sha384 => Pkcs1v15Sign::new::<Sha384>(),
            Hash::Sha512 => Pkcs1v15Sign::new::<Sha512>(),
            _ => Pkcs1v15Sign::new::<Sha256>(),
        };
        key.verify(scheme, &digest_of(self.hash, message), signature)
            .map_err(|_| InvalidSignature)
    }
}

#[derive(Debug)]
struct EcdsaP256;

impl SignatureVerificationAlgorithm for EcdsaP256 {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P256
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA256
    }

    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let key =
            p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| InvalidSignature)?;
        let signature =
            p256::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        key.verify(message, &signature)
            .map_err(|_| InvalidSignature)
    }
}

#[derive(Debug)]
struct EcdsaP384;

impl SignatureVerificationAlgorithm for EcdsaP384 {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P384
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA384
    }

    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let key =
            p384::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| InvalidSignature)?;
        let signature =
            p384::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        key.verify(message, &signature)
            .map_err(|_| InvalidSignature)
    }
}

mod hex {
    pub(super) fn upper(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }
}
