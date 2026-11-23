use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};
use postio_session::{STORE_KEY_ENTRY, store_key};

fn entry() -> AccountKey {
    AccountKey::new(STORE_KEY_ENTRY)
}

#[tokio::test]
async fn a_first_run_mints_a_key_and_keeps_it() {
    let keyring = MemorySecretStore::new();
    assert!(keyring.is_empty(), "nothing has ever been stored");

    let key = store_key(&keyring).await.expect("a first run mints one");

    let kept = keyring
        .retrieve(&entry())
        .await
        .expect("and writes it down");
    assert_eq!(
        kept.expose(),
        key.to_hex().as_str(),
        "the key handed back is the key that was kept — anything else and the \
         store is encrypted under something nobody can read again"
    );
}

#[tokio::test]
async fn a_second_open_reads_the_key_back_rather_than_minting_another() {
    let keyring = MemorySecretStore::new();

    let first = store_key(&keyring).await.expect("first open");
    let second = store_key(&keyring.reopen()).await.expect("second open");

    assert_eq!(first.to_hex(), second.to_hex());
    assert_eq!(keyring.len(), 1, "and one entry, not two");
}

#[tokio::test]
async fn a_locked_keyring_refuses_and_says_how_to_unlock() {
    let keyring = MemorySecretStore::locked();

    let refused = store_key(&keyring)
        .await
        .expect_err("a locked keyring cannot answer");

    assert!(
        matches!(refused, SecretError::Locked { .. }),
        "the variant has to survive, because it is what routes to the unlock \
         surface rather than to onboarding: {refused:?}"
    );
    let said = postio_session::key_refusal(&refused).sentence;
    assert!(
        said.contains("locked") && said.contains("unlock it"),
        "and it has to tell the user what to do: {said}"
    );
    assert!(
        !said.contains("local store encryption key") && !said.contains("cannot read"),
        "in a person's words, not the error's: {said}"
    );
    assert!(
        keyring.is_empty(),
        "nothing was written to a locked keyring"
    );
}

#[tokio::test]
async fn an_entry_that_is_not_a_key_is_refused_rather_than_replaced() {
    let keyring = MemorySecretStore::new();
    keyring
        .store(&entry(), &Password::new("this is not a key"))
        .await
        .expect("the fixture writes");

    let refused = store_key(&keyring).await.expect_err("that will not parse");

    assert!(
        !matches!(refused, SecretError::NotFound { .. }),
        "a corrupt entry is not a missing one: {refused:?}"
    );
    assert_eq!(
        keyring
            .retrieve(&entry())
            .await
            .expect("still there")
            .expose(),
        "this is not a key",
        "the entry that could not be parsed must still be there afterwards"
    );
}

#[tokio::test]
async fn an_empty_entry_is_a_failed_write_and_is_minted_over() {
    let keyring = MemorySecretStore::new();
    keyring
        .store(&entry(), &Password::new(""))
        .await
        .expect("the fixture writes");

    let key = store_key(&keyring).await.expect("a fresh key");

    assert_eq!(
        keyring.retrieve(&entry()).await.expect("kept").expose(),
        key.to_hex().as_str()
    );
}

#[test]
fn the_key_can_be_read_with_no_runtime_running() {
    let keyring = MemorySecretStore::new();

    let first = postio_session::store_key_blocking(&keyring).expect("a first run");
    let second = postio_session::store_key_blocking(&keyring.reopen()).expect("a second run");

    assert_eq!(first.to_hex(), second.to_hex());
}

#[test]
fn a_locked_keyring_means_there_is_no_store_to_open() {
    let refused = postio_session::store_key_blocking(&MemorySecretStore::locked())
        .expect_err("a locked keyring cannot answer");

    assert!(matches!(refused, SecretError::Locked { .. }));
    assert!(
        refused.to_string().contains("unlock"),
        "and the sentence that reaches the user says what to do: {refused}"
    );
}

use postio_test_support::logs::Captured;

#[tokio::test]
async fn no_key_material_reaches_the_log_at_any_level() {
    if postio_test_support::alone!("no_key_material_reaches_the_log_at_any_level") {
        return;
    }
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();

    let minted = {
        let _guard = tracing::subscriber::set_default(subscriber);
        let keyring = MemorySecretStore::new();
        let minted = store_key(&keyring).await.expect("first run");
        store_key(&keyring.reopen()).await.expect("second run");
        let _ = store_key(&MemorySecretStore::locked()).await;

        let corrupt = MemorySecretStore::new();
        corrupt
            .store(&entry(), &Password::new("not-a-key"))
            .await
            .expect("the fixture writes");
        let _ = store_key(&corrupt).await;
        minted
    };

    let log = captured.text();
    let hex = minted.to_hex();
    assert!(!log.contains(hex.as_str()), "the key is in the log: {log}");
    for window in hex.as_bytes().windows(16) {
        let fragment = std::str::from_utf8(window).expect("hex is ascii");
        assert!(
            !log.contains(fragment),
            "a fragment of the key is in the log ({fragment}): {log}"
        );
    }
}

#[tokio::test]
async fn a_store_this_build_cannot_read_refuses_with_a_sentence() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("postio.db");
    std::fs::write(&path, b"this is not a database").expect("write the file");

    let key = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);
    let said = postio_session::open_store_at(&path, &key)
        .await
        .expect_err("a file that is not a store must not open");

    assert!(
        !said.is_empty() && said.chars().any(char::is_alphabetic),
        "the refusal must say something: {said:?}"
    );
}

#[tokio::test]
async fn a_store_this_build_wrote_opens_again_with_its_mail() {
    use postio_model::Message;
    use postio_storage::repository::MessageRepository;

    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("postio.db");
    let key = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);

    {
        let (database, _blobs) = postio_session::open_store_at(&path, &key)
            .await
            .expect("a fresh store opens");
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, chrono::Utc::now());
        message.subject = Some("Zarquon".to_owned());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create");
    }

    let (database, _blobs) = postio_session::open_store_at(&path, &key)
        .await
        .expect("and it opens again");
    let connection = database.connect().await.expect("checkout");
    let subject: String =
        postio_storage::sql::one(&connection, "SELECT subject FROM messages", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("the message survived the reopen");
    assert_eq!(subject, "Zarquon");
}

#[tokio::test]
async fn a_store_under_another_key_is_refused_in_one_sentence() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("postio.db");
    let written = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);
    drop(
        postio_session::open_store_at(&path, &written)
            .await
            .expect("a fresh store opens"),
    );

    let other = postio_storage::key::StoreKey::from_bytes([0x17; 32]);
    let said = postio_session::open_store_at(&path, &other)
        .await
        .expect_err("a store must not open under another key");

    assert!(
        said.contains("another installation"),
        "the refusal names the cause: {said:?}"
    );
    let restarts = said
        .split(". ")
        .skip(1)
        .filter(|next| next.chars().next().is_some_and(char::is_lowercase))
        .count();
    assert_eq!(
        restarts, 0,
        "two openings were stacked, leaving a sentence that starts lowercase: {said:?}"
    );
}

#[tokio::test]
async fn a_store_under_another_key_is_started_over_not_retried() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("postio.db");
    let written = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);
    drop(
        postio_session::open_store_at(&path, &written)
            .await
            .expect("a fresh store opens"),
    );

    let ours = postio_storage::key::StoreKey::from_bytes([0x17; 32]);
    let Err(refusal) = postio_session::open_store_at_reporting(&path, &ours, &|_| {}).await else {
        panic!("a store must not open under another key");
    };
    assert_eq!(
        refusal.remedy,
        postio_session::Remedy::StartOver {
            store: path.clone(),
            because: postio_session::Unreadable::AnotherKey,
        },
        "retrying meets the same key every time: {refusal:?}"
    );

    let started = postio_session::start_over::start_over_at(&path, &ours)
        .await
        .expect("a store under another key can be started over");
    assert!(
        started.set_aside.join("postio.db").exists(),
        "the old store is set aside, not deleted: {}",
        started.set_aside.display()
    );
    assert_eq!(
        started.accounts, 0,
        "nothing comes across from a store this key cannot read"
    );
    postio_session::open_store_at(&path, &ours)
        .await
        .expect("the fresh store opens under this installation's key");
}

#[test]
fn a_locked_keyring_is_only_ever_tried_again() {
    let locked = SecretError::Locked {
        keyring: "login".to_owned(),
        account: STORE_KEY_ENTRY.to_owned(),
    };
    assert_eq!(
        postio_session::key_refusal(&locked).remedy,
        postio_session::Remedy::TryAgain
    );
}
