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
    let outcome = Store::open(&path, &key).await;
    let error = outcome
        .err()
        .expect("a file this build cannot read must not open");
    println!("refused with: {error}");
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
