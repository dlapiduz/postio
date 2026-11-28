use chrono::Utc;
use postio_model::test_corpus;
use postio_model::{BodyState, EmailAddress, MessageId};
use postio_session::signature;
use postio_storage::repository::{KeyRepository, KeySource, MessageRepository, StoredBody};
use postio_storage::{BlobStore, Store, test_support};

struct Scene {
    database: Store,
    blobs: BlobStore,
    account: postio_model::Account,
    inbox: postio_model::MailboxId,
    _dir: tempfile::TempDir,
}

impl Scene {
    async fn new() -> Scene {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        drop(connection);
        let dir = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(dir.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Scene {
            database,
            blobs,
            account,
            inbox,
            _dir: dir,
        }
    }

    async fn arrive(&self, fixture: &str, keep_raw: bool) -> MessageId {
        let bytes = test_corpus::load(fixture).bytes();
        let mut message =
            postio_model::mime::parse(bytes).into_message(self.account.id, self.inbox, Utc::now());
        if keep_raw {
            message.raw_blob_id = Some(self.blobs.put(bytes).expect("kept"));
        }
        let connection = self.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let id = messages.create(&mut message).await.expect("a message");
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(String::from_utf8_lossy(bytes).into_owned()),
                    html: None,
                    headers: postio_model::headers::block_of(bytes).map(|block| block.text),
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("its body");
        id
    }

    async fn status(&self, message: MessageId) -> Option<String> {
        let connection = self.database.connect().await.expect("a connection");
        KeyRepository::new(&connection)
            .verdict(message)
            .await
            .expect("a read")
            .map(|row| row.status)
    }

    async fn check(&self, messages: &[MessageId]) {
        signature::check_messages(&self.database, &self.blobs, messages)
            .await
            .expect("the job runs");
    }
}

fn ada() -> EmailAddress {
    EmailAddress::new(None::<String>, "ada@example.com")
}

#[tokio::test]
async fn an_autocrypt_header_records_the_senders_key() {
    let scene = Scene::new().await;
    let message = scene.arrive("autocrypt-header", false).await;
    scene.check(&[message]).await;
    let connection = scene.database.connect().await.expect("a connection");
    let keys = KeyRepository::new(&connection);
    let held = keys.for_address(&ada()).await.expect("a read");
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(
        keys.sources(&held[0].fingerprint).await.expect("a read"),
        [KeySource::Autocrypt]
    );
    assert_eq!(
        scene.status(message).await,
        None,
        "an unsigned message has no verdict"
    );
}

#[tokio::test]
async fn a_signed_message_gets_its_verdict_row_off_the_reading_path() {
    let scene = Scene::new().await;
    let carrier = scene.arrive("autocrypt-header", false).await;
    let signed = scene.arrive("pgp-signed-good", true).await;
    let tampered = scene.arrive("pgp-signed-tampered-body", true).await;
    scene.check(&[carrier, signed, tampered]).await;
    assert_eq!(scene.status(signed).await.as_deref(), Some("good"));
    assert_eq!(scene.status(tampered).await.as_deref(), Some("bad"));
}

#[tokio::test]
async fn a_key_arriving_later_rechecks_that_senders_signed_mail() {
    let scene = Scene::new().await;
    let signed = scene.arrive("pgp-signed-good", true).await;
    scene.check(&[signed]).await;
    assert_eq!(scene.status(signed).await.as_deref(), Some("cant_check"));
    let carrier = scene.arrive("autocrypt-header", false).await;
    scene.check(&[carrier]).await;
    assert_eq!(
        scene.status(signed).await.as_deref(),
        Some("good"),
        "the new key re-verified the address's rows"
    );
}

#[tokio::test]
async fn evicted_bytes_are_not_here_yet_and_resolve_when_they_return() {
    let scene = Scene::new().await;
    let carrier = scene.arrive("autocrypt-header", false).await;
    let signed = scene.arrive("pgp-signed-good", false).await;
    scene.check(&[carrier, signed]).await;
    assert_eq!(
        scene.status(signed).await.as_deref(),
        Some("not_here_yet"),
        "never cant_check or bad for bytes that are only absent"
    );
    let connection = scene.database.connect().await.expect("a connection");
    let messages = MessageRepository::new(&connection);
    let mut row = messages.get(signed).await.expect("a read").expect("a row");
    row.raw_blob_id = Some(
        scene
            .blobs
            .put(test_corpus::load("pgp-signed-good").bytes())
            .expect("kept"),
    );
    messages.update(&mut row).await.expect("the fetch landed");
    drop(connection);
    scene.check(&[signed]).await;
    assert_eq!(scene.status(signed).await.as_deref(), Some("good"));
}

#[tokio::test]
async fn the_reading_says_signed_and_signed_accepted_once_the_key_is_accepted() {
    use postio_ui::reader::authenticity::Mark;
    let scene = Scene::new().await;
    let carrier = scene.arrive("autocrypt-header", false).await;
    let signed = scene.arrive("pgp-signed-good", true).await;
    scene.check(&[carrier, signed]).await;
    let connection = scene.database.connect().await.expect("a connection");
    let messages = MessageRepository::new(&connection);
    let row = messages.get(signed).await.expect("a read").expect("a row");
    let block = postio_model::headers::block_of(test_corpus::load("pgp-signed-good").bytes())
        .map(|block| block.text);
    let mark = async || {
        postio_session::reading::authenticity(&connection, &row, block.as_deref())
            .await
            .mark
    };
    assert_eq!(mark().await, Some(Mark::Signed));
    let keys = KeyRepository::new(&connection);
    let signer = keys
        .verdict(signed)
        .await
        .expect("a read")
        .expect("a verdict")
        .fingerprint;
    keys.accept(&ada(), &signer.expect("a signer"), 1)
        .await
        .expect("accepted");
    assert_eq!(mark().await, Some(Mark::SignedAccepted));
    keys.unaccept(&ada()).await.expect("taken back");
    assert_eq!(mark().await, Some(Mark::Signed));
}

#[tokio::test]
async fn an_encrypted_message_opens_on_a_sentence_not_a_blank_body() {
    let scene = Scene::new().await;
    let encrypted = scene.arrive("pgp-encrypted", false).await;
    let connection = scene.database.connect().await.expect("a connection");
    let (body, _) = postio_session::reading::load_with_row(&connection, encrypted, false).await;
    let postio_session::reading::Body::Ready { body, .. } = body else {
        panic!("the encrypted message has a body to show");
    };
    assert_eq!(
        body.text.as_deref(),
        Some(postio_ui::reader::document::ENCRYPTED)
    );
}
