//! A store this build cannot read is refused, and left exactly as it was.
//!
//! There is no migration from the SQLCipher format -- a store in it is rebuilt
//! by resyncing (ADR 0038). "Rebuilt" has to mean the old file is still there
//! to be moved aside, so the failure path may not truncate, may not create,
//! and may not half-write a header over somebody's mailbox.
//!
//! Its own test rather than a `storage_suite` module because it is about
//! `Store::open` refusing, which is the one thing the suite's shared fixtures
//! cannot set up: they all open successfully.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::{
    Store,
    key::{Purpose, StoreKey},
};
#[tokio::test]
async fn a_store_this_build_cannot_read_is_left_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    // A file shaped like the old store: a 16-byte random salt where SQLCipher
    // puts one, then pages of ciphertext. Nothing this build can read.
    let original: Vec<u8> = (0..32_768u32)
        .map(|n| (n.wrapping_mul(2654435761) >> 16) as u8)
        .collect();
    std::fs::write(&path, &original).unwrap();
    let before = std::fs::read(&path).unwrap();

    let key = StoreKey::from_bytes([0x2a; 32]).derive(Purpose::Database);
    let Err(error) = Store::open(&path, &key).await else {
        panic!("a file this build cannot read must not open");
    };
    assert!(
        matches!(error, postio_storage::Error::WrongStoreKey),
        "the refusal reached a screen as the engine's own words. Postio writes \
         one key and one format, so a store that will not open is the key or \
         the format and never a rotted disk -- and the sentence has to say \
         that, because the remedy (sync again) is not something a person can \
         infer from \"invalid page size in database header\". Got: {error}"
    );

    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after, "the refusal rewrote the file");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len() as usize,
        original.len(),
        "the refusal changed the file's length"
    );
}
