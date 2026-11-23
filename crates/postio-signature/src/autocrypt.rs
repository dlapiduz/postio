//! A key from an `Autocrypt:` header (Level 1): kept only when its `addr` is the From address, and
//! trusted by nothing but the person's later decision.

use base64::Engine as _;
use pgp::composed::{Deserializable, SignedPublicKey};

pub fn autocrypt(header_value: &str, from: &str) -> Option<SignedPublicKey> {
    let mut addr = None;
    let mut keydata = None;
    for attribute in header_value.split(';') {
        let (name, value) = attribute.split_once('=')?;
        match name.trim().to_ascii_lowercase().as_str() {
            "addr" => addr = Some(value.trim()),
            "keydata" => keydata = Some(value),
            _ => {}
        }
    }
    if !addr?.eq_ignore_ascii_case(from.trim()) {
        return None;
    }
    let compact: String = keydata?.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .ok()?;
    std::panic::catch_unwind(|| SignedPublicKey::from_bytes(&bytes[..]).ok())
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use pgp::types::KeyDetails;
    use postio_model::{headers, test_corpus};

    use super::*;

    fn header() -> String {
        let block = headers::block_of(test_corpus::load("autocrypt-header").bytes())
            .expect("a header block");
        headers::parse_block(&block.text)
            .get("Autocrypt")
            .expect("the fixture carries one")
            .to_owned()
    }

    fn key_a() -> SignedPublicKey {
        let path = format!(
            "{}/../postio-model/tests/corpus/keys/ada-a.asc",
            env!("CARGO_MANIFEST_DIR")
        );
        SignedPublicKey::from_string(&std::fs::read_to_string(path).expect("key A"))
            .expect("parses")
            .0
    }

    #[test]
    fn the_header_carries_the_from_addresss_key() {
        let key = autocrypt(&header(), "Ada@Example.com").expect("a key");
        assert_eq!(key.fingerprint(), key_a().fingerprint());
    }

    #[test]
    fn an_addr_that_is_not_from_is_refused() {
        assert!(autocrypt(&header(), "grace@example.net").is_none());
    }

    #[test]
    fn broken_keydata_or_missing_attributes_are_nothing() {
        for value in [
            "addr=ada@example.com; keydata=!!!!",
            "addr=ada@example.com; keydata=AAAA",
            "addr=ada@example.com",
            "keydata=AAAA",
            "",
        ] {
            assert!(autocrypt(value, "ada@example.com").is_none(), "{value}");
        }
    }
}
