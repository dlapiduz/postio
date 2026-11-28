use pgp::composed::{Deserializable, SignedPublicKey};
use pgp::ser::Serialize as _;
use pgp::types::KeyDetails as _;

use crate::verify::KeySet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub fingerprint: String,
    pub keyblock: Vec<u8>,
    pub primary_uid: Option<String>,
    pub addresses: Vec<String>,
    pub revoked: bool,
}

impl KeyInfo {
    pub fn of(key: &SignedPublicKey) -> Option<KeyInfo> {
        let uids: Vec<String> = key
            .details
            .users
            .iter()
            .map(|user| String::from_utf8_lossy(user.id.id()).into_owned())
            .collect();
        let mut addresses: Vec<String> = uids.iter().filter_map(|uid| address_in(uid)).collect();
        addresses.sort();
        addresses.dedup();
        Some(KeyInfo {
            fingerprint: key.fingerprint().to_string().to_ascii_uppercase(),
            keyblock: key.to_bytes().ok()?,
            primary_uid: uids.first().cloned(),
            addresses,
            revoked: !key.details.revocation_signatures.is_empty(),
        })
    }
}

const BEGIN: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";
const END: &str = "-----END PGP PUBLIC KEY BLOCK-----";

pub fn keys_in(bytes: &[u8]) -> Vec<KeyInfo> {
    std::panic::catch_unwind(|| {
        let text = String::from_utf8_lossy(bytes);
        let parsed: Vec<SignedPublicKey> = if text.contains(BEGIN) {
            text.split(BEGIN)
                .skip(1)
                .filter_map(|block| {
                    let end = block.find(END)? + END.len();
                    let armoured = format!("{BEGIN}{}", &block[..end]);
                    SignedPublicKey::from_string(&armoured)
                        .ok()
                        .map(|(key, _)| key)
                })
                .collect()
        } else {
            SignedPublicKey::from_bytes_many(bytes)
                .map(|keys| keys.filter_map(Result::ok).collect())
                .unwrap_or_default()
        };
        parsed.iter().filter_map(KeyInfo::of).collect()
    })
    .unwrap_or_default()
}

impl KeySet {
    pub fn from_keyblocks<'a>(blocks: impl IntoIterator<Item = &'a [u8]>) -> KeySet {
        let mut set = KeySet::new();
        for block in blocks {
            if let Ok(Ok(key)) = std::panic::catch_unwind(|| SignedPublicKey::from_bytes(block)) {
                set.push(key);
            }
        }
        set
    }
}

fn address_in(uid: &str) -> Option<String> {
    let inner = match (uid.rfind('<'), uid.rfind('>')) {
        (Some(open), Some(close)) if open < close => &uid[open + 1..close],
        _ => uid.trim(),
    };
    (inner.contains('@') && !inner.contains(char::is_whitespace))
        .then(|| inner.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus_key(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/../postio-model/tests/corpus/keys/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("a corpus key")
    }

    #[test]
    fn a_key_file_names_its_fingerprint_user_id_and_address() {
        let keys = keys_in(&corpus_key("ada-a.asc"));
        assert_eq!(keys.len(), 1);
        let key = &keys[0];
        assert_eq!(key.addresses, ["ada@example.com"]);
        assert_eq!(
            key.primary_uid.as_deref(),
            Some("Ada Moreno (corpus key A) <ada@example.com>")
        );
        assert_eq!(key.fingerprint.len(), 40);
        assert!(!key.revoked);
        let rebuilt = KeySet::from_keyblocks([key.keyblock.as_slice()]);
        assert_eq!(rebuilt.keys().len(), 1, "the stored bytes parse back");
    }

    #[test]
    fn two_armoured_keys_in_one_file_are_both_read_and_garbage_is_none() {
        let mut both = corpus_key("ada-a.asc");
        both.extend(corpus_key("ada-b.asc"));
        assert_eq!(keys_in(&both).len(), 2);
        assert!(keys_in(b"not a key").is_empty());
        assert!(keys_in(b"-----BEGIN PGP PUBLIC KEY BLOCK-----\n\n!!!\n").is_empty());
    }
}
