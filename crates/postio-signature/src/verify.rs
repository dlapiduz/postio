//! The signature check itself: the exact signed bytes, the signer's key from the keys handed in,
//! the mathematics from rPGP, and the meaning from `Policy`. Reads no file and no socket.

use std::io::Cursor;

use pgp::composed::{CleartextSignedMessage, Deserializable, DetachedSignature, SignedPublicKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::packet::{PublicKey, PublicSubkey, Signature};
use pgp::types::{KeyDetails, PublicParams, VerifyingKey};
use rsa::traits::PublicKeyParts;

use crate::policy::{Facts, Hash, KeyStrength, Policy, Reason, Status};
use crate::scope::{Format, SignedScope, signed_scope};

#[derive(Debug, Clone, Default)]
pub struct KeySet {
    keys: Vec<SignedPublicKey>,
    anchors: Vec<Vec<u8>>,
}

impl KeySet {
    pub fn new() -> KeySet {
        KeySet::default()
    }

    pub fn push(&mut self, key: SignedPublicKey) {
        self.keys.push(key);
    }

    pub fn keys(&self) -> &[SignedPublicKey] {
        &self.keys
    }

    pub fn push_anchor(&mut self, certificate: Vec<u8>) {
        self.anchors.push(certificate);
    }

    pub fn anchors(&self) -> &[Vec<u8>] {
        &self.anchors
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub status: Status,
    pub reason: Option<Reason>,
    pub scope: SignedScope,
    /// The signing key's fingerprint as upper-case hex, when the signature names it.
    pub signer: Option<String>,
    /// Seconds since the Unix epoch.
    pub signed_at: Option<i64>,
}

/// `None` when the message carries no signature Postio can find. Never panics.
pub fn verify(raw: &[u8], keys: &KeySet, policy: &Policy) -> Option<Verdict> {
    let scope = signed_scope(raw)?;
    let refused = |reason| Verdict {
        status: Status::CantCheck,
        reason: Some(reason),
        scope: scope.clone(),
        signer: None,
        signed_at: None,
    };
    let checked = std::panic::catch_unwind(|| match &scope {
        SignedScope::Encrypted => refused(Reason::Unsupported),
        SignedScope::WholeMessage {
            format: Format::Smime,
            part,
            signature,
        }
        | SignedScope::Part {
            format: Format::Smime,
            part,
            signature,
            ..
        } => crate::smime::check(&scope, raw, part.clone(), signature.clone(), keys),
        SignedScope::WholeMessage {
            part, signature, ..
        }
        | SignedScope::Part {
            part, signature, ..
        } => {
            let Ok((detached, _)) =
                DetachedSignature::from_armor_single(Cursor::new(&raw[signature.clone()]))
            else {
                return refused(Reason::Malformed);
            };
            let signed = crlf(&raw[part.clone()]);
            judge(&scope, &detached.signature, keys, policy, |key| match key {
                Key::Primary(key) => detached.verify(*key, &signed).is_ok(),
                Key::Sub(key) => detached.verify(*key, &signed).is_ok(),
            })
        }
        SignedScope::Inline { text, .. } => {
            let text = String::from_utf8_lossy(&raw[text.clone()]).into_owned();
            let end = text
                .find("-----END PGP SIGNATURE-----")
                .map_or(text.len(), |at| at + "-----END PGP SIGNATURE-----".len());
            let Ok((message, _)) = CleartextSignedMessage::from_string(&text[..end]) else {
                return refused(Reason::Malformed);
            };
            let Some(signature) = message.signatures().first().cloned() else {
                return refused(Reason::Malformed);
            };
            judge(&scope, &signature, keys, policy, |key| match key {
                Key::Primary(key) => message.verify(*key).is_ok(),
                Key::Sub(key) => message.verify(*key).is_ok(),
            })
        }
    });
    Some(checked.unwrap_or_else(|_| refused(Reason::Malformed)))
}

fn judge(
    scope: &SignedScope,
    signature: &Signature,
    keys: &KeySet,
    policy: &Policy,
    holds: impl Fn(&Key<'_>) -> bool,
) -> Verdict {
    let named = signature
        .issuer_fingerprint()
        .first()
        .map(|fingerprint| hex(&fingerprint.to_string()));
    let signed_at = signature.created().map(|at| i64::from(at.as_secs()));
    let Some(found) = signer(signature, keys) else {
        return Verdict {
            status: Status::CantCheck,
            reason: Some(Reason::NoKey),
            scope: scope.clone(),
            signer: named,
            signed_at,
        };
    };
    let facts = Facts {
        hash: hash(signature.hash_alg()),
        key: strength(found.key.get().public_params()),
        signed_at,
        key_created: i64::from(found.key.get().created_at().as_secs()),
        key_expires: found.expires,
        revoked: found.revoked,
    };
    let (status, reason) = policy.judge(&facts, holds(&found.key));
    Verdict {
        status,
        reason,
        scope: scope.clone(),
        signer: Some(hex(&found.key.get().fingerprint().to_string())),
        signed_at,
    }
}

enum Key<'a> {
    Primary(&'a PublicKey),
    Sub(&'a PublicSubkey),
}

impl Key<'_> {
    fn get(&self) -> &dyn KeyDetails {
        match self {
            Key::Primary(key) => *key,
            Key::Sub(key) => *key,
        }
    }
}

struct Found<'a> {
    key: Key<'a>,
    expires: Option<i64>,
    revoked: bool,
}

fn signer<'a>(signature: &Signature, keys: &'a KeySet) -> Option<Found<'a>> {
    let fingerprints = signature.issuer_fingerprint();
    let ids = signature.issuer_key_id();
    let names = |key: &dyn VerifyingKey| {
        fingerprints
            .iter()
            .any(|named| **named == key.fingerprint())
            || ids.iter().any(|named| **named == key.legacy_key_id())
    };
    for key in &keys.keys {
        let revoked = !key.details.revocation_signatures.is_empty();
        let created = i64::from(key.primary_key.created_at().as_secs());
        if names(&key.primary_key) {
            let expires = key
                .details
                .users
                .iter()
                .flat_map(|user| user.signatures.iter())
                .find_map(Signature::key_expiration_time)
                .map(|after| created + after.as_secs() as i64);
            return Some(Found {
                key: Key::Primary(&key.primary_key),
                expires,
                revoked,
            });
        }
        for subkey in &key.public_subkeys {
            if names(&subkey.key) {
                let created = i64::from(subkey.key.created_at().as_secs());
                let expires = subkey
                    .signatures
                    .iter()
                    .find_map(Signature::key_expiration_time)
                    .map(|after| created + after.as_secs() as i64);
                return Some(Found {
                    key: Key::Sub(&subkey.key),
                    expires,
                    revoked,
                });
            }
        }
    }
    None
}

/// RFC 3156 signs canonical text: bare LF becomes CRLF, and nothing else changes.
pub(crate) fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    let mut previous = 0u8;
    for &byte in bytes {
        if byte == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(byte);
        previous = byte;
    }
    out
}

fn hex(text: &str) -> String {
    text.to_ascii_uppercase()
}

fn hash(algorithm: Option<HashAlgorithm>) -> Hash {
    match algorithm {
        Some(HashAlgorithm::Md5) => Hash::Md5,
        Some(HashAlgorithm::Sha1) => Hash::Sha1,
        Some(HashAlgorithm::Ripemd160) => Hash::Ripemd160,
        Some(HashAlgorithm::Sha224) => Hash::Sha224,
        Some(HashAlgorithm::Sha256) => Hash::Sha256,
        Some(HashAlgorithm::Sha384) => Hash::Sha384,
        Some(HashAlgorithm::Sha512) => Hash::Sha512,
        Some(HashAlgorithm::Sha3_256 | HashAlgorithm::Sha3_512) => Hash::Sha3,
        _ => Hash::Other,
    }
}

fn strength(params: &PublicParams) -> KeyStrength {
    match params {
        PublicParams::RSA(rsa) => KeyStrength::Rsa {
            bits: u32::try_from(rsa.key.size() * 8).unwrap_or(0),
        },
        // Deprecated by RFC 9580 and rare in mail; treated as too weak rather than measured.
        PublicParams::DSA(_) => KeyStrength::Dsa { bits: 0 },
        PublicParams::EdDSALegacy(_) | PublicParams::Ed25519(_) | PublicParams::Ed448(_) => {
            KeyStrength::EdDsa
        }
        PublicParams::ECDSA(_) => KeyStrength::Ecdsa,
        _ => KeyStrength::Other,
    }
}

#[cfg(test)]
mod tests {
    use postio_model::test_corpus;

    use super::*;

    fn key(name: &str) -> SignedPublicKey {
        let path = format!(
            "{}/../postio-model/tests/corpus/keys/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).expect("a corpus key");
        SignedPublicKey::from_string(&text).expect("it parses").0
    }

    fn keys(names: &[&str]) -> KeySet {
        let mut set = KeySet::new();
        for name in names {
            set.push(key(name));
        }
        set
    }

    fn status(fixture: &str, with: &[&str]) -> (Status, Option<Reason>) {
        let verdict = verify(test_corpus::load(fixture).bytes(), &keys(with), &Policy)
            .unwrap_or_else(|| panic!("{fixture} has a signature"));
        (verdict.status, verdict.reason)
    }

    const BOTH: &[&str] = &["ada-a.asc", "ada-b.asc"];

    #[test]
    fn each_fixture_reads_as_its_signature_is() {
        for (fixture, expected) in [
            ("pgp-signed-good", (Status::Good, None)),
            ("pgp-signed-qp", (Status::Good, None)),
            ("pgp-signed-other-key", (Status::Good, None)),
            ("pgp-signed-list-footer", (Status::Good, None)),
            ("pgp-inline-signed", (Status::Good, None)),
            (
                "pgp-signed-tampered-body",
                (Status::Bad, Some(Reason::Tampered)),
            ),
            (
                "pgp-signed-tampered-header",
                (Status::Bad, Some(Reason::Tampered)),
            ),
            (
                "pgp-signed-reencoded",
                (Status::Bad, Some(Reason::Tampered)),
            ),
            (
                "pgp-signed-sha1",
                (Status::CantCheck, Some(Reason::WeakHash(Hash::Sha1))),
            ),
            (
                "pgp-signed-no-key",
                (Status::CantCheck, Some(Reason::NoKey)),
            ),
            (
                "pgp-encrypted",
                (Status::CantCheck, Some(Reason::Unsupported)),
            ),
        ] {
            assert_eq!(status(fixture, BOTH), expected, "{fixture}");
        }
    }

    #[test]
    fn the_signer_is_named_by_fingerprint_and_a_missing_key_is_no_key() {
        let good = verify(
            test_corpus::load("pgp-signed-good").bytes(),
            &keys(BOTH),
            &Policy,
        )
        .expect("signed");
        let a = hex(&key("ada-a.asc").fingerprint().to_string());
        assert_eq!(good.signer.as_deref(), Some(a.as_str()));
        assert!(good.signed_at.is_some());
        assert_eq!(
            status("pgp-signed-good", &["ada-b.asc"]),
            (Status::CantCheck, Some(Reason::NoKey)),
            "another key of the same address is not this signature's"
        );
    }

    #[test]
    fn a_malformed_signature_is_uncheckable_never_bad() {
        let raw = String::from_utf8_lossy(test_corpus::load("pgp-signed-good").bytes()).replace(
            "-----BEGIN PGP SIGNATURE-----",
            "-----BEGIN PGP SIGNATURE-----\r\n!!!!",
        );
        let verdict = verify(raw.as_bytes(), &keys(BOTH), &Policy).expect("signed");
        assert_eq!(
            (verdict.status, verdict.reason),
            (Status::CantCheck, Some(Reason::Malformed))
        );
    }

    #[test]
    fn bare_line_feeds_are_signed_as_crlf() {
        assert_eq!(crlf(b"a\nb\r\nc\n"), b"a\r\nb\r\nc\r\n");
    }
}
