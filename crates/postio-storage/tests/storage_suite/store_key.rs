use postio_storage::key::{Purpose, StoreKey};

fn fixed() -> StoreKey {
    StoreKey::from_bytes([7u8; 32])
}

#[test]
fn a_generated_key_is_thirty_two_bytes_and_not_the_same_twice() {
    let first = StoreKey::generate();
    let second = StoreKey::generate();

    assert_ne!(
        first.to_hex(),
        second.to_hex(),
        "two calls to the OS RNG returning the same key is not a flake, it is \
         a broken RNG — and every store would share one key"
    );
    assert_eq!(first.to_hex().len(), 64, "32 bytes, hex");
}

#[test]
fn a_key_round_trips_through_the_text_a_keyring_can_hold() {
    // Hex, not base64: no case or padding questions, and a keyring browser cannot mistake it for
    // something typed.
    let key = StoreKey::generate();

    let restored = StoreKey::from_hex(&key.to_hex()).expect("its own hex parses");

    assert_eq!(restored.to_hex(), key.to_hex());
}

#[test]
fn text_that_is_not_a_key_is_refused_rather_than_padded() {
    // A corrupted keyring entry must not become a different valid key: that would look like data
    // loss.
    for wrong in [
        "",
        "07",
        &"7".repeat(63),
        &"7".repeat(65),
        &"z".repeat(64),
        &"  ".repeat(32),
    ] {
        assert!(
            StoreKey::from_hex(wrong).is_err(),
            "{wrong:?} should not parse as a key"
        );
    }
}

#[test]
fn subkey_derivation_is_deterministic() {
    let key = fixed();

    assert_eq!(
        key.derive(Purpose::Database).to_hex(),
        fixed().derive(Purpose::Database).to_hex()
    );
}

#[test]
fn the_three_purposes_are_cryptographically_separated() {
    let key = fixed();
    let derived: Vec<String> = [Purpose::Database, Purpose::BlobContent, Purpose::BlobId]
        .into_iter()
        .map(|purpose| key.derive(purpose).to_hex().to_string())
        .collect();

    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(
            derived[left], derived[right],
            "two purposes derived the same subkey"
        );
    }
    assert_ne!(
        derived[0],
        key.to_hex().to_string(),
        "and a subkey is never the master key itself"
    );
}

#[test]
fn the_derivation_contexts_are_the_ones_the_adr_named() {
    // Test vectors, pinned: these strings are part of the on-disk format, and changing one stops
    // every existing store opening. Computed from BLAKE3's `derive_key`, not pasted.
    let key = fixed();
    for (purpose, context) in [
        (Purpose::Database, "postio db"),
        (Purpose::BlobContent, "postio blob content"),
        (Purpose::BlobId, "postio blob id"),
    ] {
        let expected = blake3::derive_key(context, key.expose());
        assert_eq!(
            key.derive(purpose).to_hex().to_string(),
            hex(&expected),
            "the context for {purpose:?} must stay {context:?}"
        );
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn no_key_material_survives_a_debug_print() {
    // A `Debug` that dumps the key turns any `dbg!` or panic message into a full compromise of the
    // store.
    let key = fixed();
    let subkey = key.derive(Purpose::Database);

    let (key_hex, subkey_hex) = (key.to_hex().to_string(), subkey.to_hex().to_string());
    for rendered in [format!("{key:?}"), format!("{subkey:?}")] {
        assert!(
            !rendered.contains(&key_hex),
            "the master key is in a debug print: {rendered}"
        );
        assert!(
            !rendered.contains(&subkey_hex),
            "a subkey is in a debug print: {rendered}"
        );
        assert!(
            !rendered.contains("0707"),
            "and neither is any run of its bytes: {rendered}"
        );
    }
}
