//! The store's master key and the subkeys derived from it — ADR 0014 Q3.
//!
//! Nothing here touches a keyring: the key *service* (fetch it, or mint one
//! and keep it) is `postio-session`'s, because that is where the
//! `SecretStore` seam lives. This is the material itself, and what has to be
//! true of it whatever fetched it.

use postio_storage::key::{Purpose, StoreKey};

/// A key with known bytes, so a derivation can be asserted against a fixed
/// value rather than against itself.
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
    // The Secret Service seam stores a `Password`, which is a string. Hex
    // rather than base64 because the alphabet has no case or padding
    // questions and a keyring browser showing it cannot mistake it for
    // anything a person typed.
    let key = StoreKey::generate();

    let restored = StoreKey::from_hex(&key.to_hex()).expect("its own hex parses");

    assert_eq!(restored.to_hex(), key.to_hex());
}

#[test]
fn text_that_is_not_a_key_is_refused_rather_than_padded() {
    // A truncated or corrupted keyring entry must not silently become a
    // *different* valid key: that would open a store that decrypts to
    // nothing and look like data loss rather than a bad key.
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
    // The database subkey has to be the same on every open or the store
    // stops opening.
    let key = fixed();

    assert_eq!(
        key.derive(Purpose::Database).to_hex(),
        fixed().derive(Purpose::Database).to_hex()
    );
}

#[test]
fn the_three_purposes_are_cryptographically_separated() {
    // ADR 0014 Q3: the database, the blob contents and the blob ids are
    // separated without three keyring entries. Sharing one subkey between
    // them would mean a weakness in one reaching the others.
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
    // Test vectors, pinned. These strings are part of the on-disk format as
    // surely as the file layout is: change one and every existing store's
    // database subkey changes with it, which means it stops opening.
    //
    // Computed here from BLAKE3's own `derive_key` rather than pasted, so
    // the assertion is "the context string is this" and not "the output was
    // whatever it was on the day this was written".
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
    // The rule `secret.rs` already keeps for passwords, applied to the one
    // secret that opens everything. A `Debug` that dumps the key turns any
    // `dbg!`, any `?err`, any panic message into a full compromise of the
    // store — and it is the kind of thing that gets added by accident when
    // somebody derives `Debug` on a struct that happens to hold one.
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
