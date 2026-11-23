use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::{
    AppendMessage, Capability, FlagChange, MailBackend, MailboxFilter, MockBackend, MockMailbox,
    SelectMode, UidSet, VecSink,
};
use postio_account::cancel::CancelToken;
use postio_account::imap::{ConnectionPool, ImapBackend, PoolConfig, RustlsConnector};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{TestMailbox, TestServer};
use postio_model::{Flag, FlagSet, ModSeq, Uid, UidValidity};

const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

const GENERATION: u32 = 4_242;
const BASELINE: u64 = 900;

async fn conforms(backend: &dyn MailBackend) {
    let cancel = CancelToken::new();

    let capabilities = backend.connect().await.expect("connect");
    assert!(capabilities.supports_incremental_sync());
    assert!(capabilities.contains(Capability::UidPlus));
    assert_eq!(
        backend.capabilities().await.expect("cached capabilities"),
        capabilities,
        "a liveness check answers from session state, without a round trip"
    );

    let mailboxes = backend
        .list_mailboxes(&MailboxFilter::all())
        .await
        .expect("list");
    let paths: Vec<&str> = mailboxes
        .iter()
        .map(|mailbox| mailbox.path.as_str())
        .collect();
    assert_eq!(paths, ["INBOX", "Archive"], "{}", backend.describe());

    let inbox = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .expect("select");
    assert_eq!(inbox.exists, 3);
    assert_eq!(inbox.generation, postio_model::Generation::new(GENERATION));
    assert_eq!(inbox.uid_next, Uid::new(4));
    assert_eq!(inbox.highest_mod_seq, Some(ModSeq::new(BASELINE)));
    assert!(!inbox.read_only);

    let headers = backend
        .fetch_headers("INBOX", &UidSet::all(), None, &cancel)
        .await
        .expect("fetch headers");
    let uids: Vec<u32> = headers.iter().map(|message| message.uid.get()).collect();
    assert_eq!(uids, [1, 2, 3]);

    let first = &headers[0];
    assert_eq!(first.uid_validity, UidValidity::new(GENERATION));
    assert_eq!(
        first.size,
        postio_model::test_corpus::load("plain-text-simple")
            .bytes()
            .len() as u64
    );
    let envelope = first.envelope.as_ref().expect("an envelope");
    assert_eq!(
        envelope.subject.as_deref(),
        Some("Tuesday walkthrough notes")
    );
    assert_eq!(envelope.from[0].address, "ada.norwood@example.com");

    let mut sink = VecSink::new();
    let fetched = backend
        .fetch_body("INBOX", &rid(1), &mut sink, &cancel)
        .await
        .expect("fetch body");
    let raw = postio_model::test_corpus::load("plain-text-simple")
        .bytes()
        .to_vec();
    assert_eq!(sink.as_slice(), raw.as_slice());
    assert_eq!(fetched.bytes_written, raw.len() as u64);

    let updates = backend
        .store_flags(
            "INBOX",
            &ids([1]),
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("store flags");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].remote_id, rid(1));
    assert!(updates[0].flags.is_seen());
    let stamped = updates[0].mod_seq.expect("a CONDSTORE server stamps it");
    assert!(stamped > ModSeq::new(BASELINE));

    let changed = backend
        .fetch_headers(
            "INBOX",
            &UidSet::all(),
            Some(ModSeq::new(BASELINE)),
            &cancel,
        )
        .await
        .expect("changedsince fetch");
    let changed: Vec<u32> = changed.iter().map(|message| message.uid.get()).collect();
    assert_eq!(changed, [1], "only what moved since the baseline");

    let mapping = backend
        .move_messages("INBOX", &ids([2]), "Archive")
        .await
        .expect("move");
    assert_eq!(mapping.len(), 1, "UIDPLUS says where it landed");
    assert_eq!(mapping[0].source, Uid::new(2));
    assert_eq!(backend.status("INBOX").await.expect("status").exists, 2);
    assert_eq!(backend.status("Archive").await.expect("status").exists, 1);

    let draft = AppendMessage::new(b"Subject: a draft\r\n\r\nnot sent yet\r\n".to_vec())
        .with_flags(FlagSet::from_iter([Flag::Draft]));
    let landed = backend
        .append("Archive", &draft)
        .await
        .expect("append")
        .expect("UIDPLUS reports APPENDUID");
    assert_eq!(
        postio_model::Generation::new(landed.uid_validity.get()),
        backend.status("Archive").await.unwrap().generation
    );
    assert_eq!(backend.status("Archive").await.expect("status").exists, 2);

    // The returned UIDs are not compared: a real server reports sequence numbers, so `ImapBackend`
    // returns an empty list.
    // Both must agree on what is left.
    backend
        .store_flags(
            "INBOX",
            &ids([3]),
            &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend.expunge("INBOX", None).await.expect("expunge");
    assert_eq!(backend.status("INBOX").await.expect("status").exists, 1);

    // The mock queues every change made through it, a real server only what the connection did not
    // do (see `MailBackend::idle`).
    // So drain first; with nothing new, a watch must end empty.
    let _backlog = backend
        .idle("INBOX", Duration::from_millis(100), &cancel)
        .await
        .expect("idle");

    let quiet = backend
        .idle("INBOX", Duration::from_millis(100), &cancel)
        .await
        .expect("idle");
    assert!(quiet.is_empty(), "nothing happened, which is not a failure");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let stopped = backend
        .idle("INBOX", Duration::from_secs(30), &cancelled)
        .await
        .expect("a cancelled idle is not a failure either");
    assert!(stopped.is_empty());
}

fn ids(values: impl IntoIterator<Item = u32>) -> Vec<postio_model::RemoteId> {
    values.into_iter().map(rid).collect()
}

fn rid(uid: u32) -> postio_model::RemoteId {
    postio_model::RemoteId::new(format!("{GENERATION}:{uid}"))
}

#[tokio::test]
async fn the_mock_behaves_like_a_mail_server() {
    let backend = MockBackend::builder()
        .capabilities([
            "IMAP4rev1",
            "ENABLE",
            "CONDSTORE",
            "QRESYNC",
            "IDLE",
            "UIDPLUS",
            "MOVE",
        ])
        .mailbox(
            MockMailbox::new("INBOX")
                .uid_validity(UidValidity::new(GENERATION))
                .highest_mod_seq(ModSeq::new(BASELINE))
                .corpus(SEEDED),
        )
        .mailbox(MockMailbox::new("Archive"))
        .build();

    conforms(&backend).await;
}

#[tokio::test]
async fn the_imap_backend_behaves_the_same_way_over_a_socket() {
    let server = TestServer::builder()
        .mailbox(
            TestMailbox::new("INBOX")
                .uid_validity(UidValidity::new(GENERATION))
                .highest_mod_seq(ModSeq::new(BASELINE))
                .corpus(SEEDED),
        )
        .mailbox(TestMailbox::new("Archive"))
        .start()
        .await;

    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");

    let backend = ImapBackend::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    );

    conforms(&backend).await;
}

#[tokio::test]
async fn a_background_view_shares_the_pool_with_its_interactive_one() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        .start()
        .await;
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    let pool = Arc::new(ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    ));

    let interactive = ImapBackend::over(Arc::clone(&pool));
    let background = interactive.background();

    interactive.connect().await.expect("connect");
    background
        .list_mailboxes(&MailboxFilter::all())
        .await
        .expect("list");

    assert_eq!(
        pool.stats().opened,
        1,
        "the second view must reuse the first one's connection, not open its own"
    );
}

#[tokio::test]
async fn an_imap_backend_offers_a_background_view_through_the_trait() {
    // The engine holds a `dyn MailBackend`, so the background view must be reachable through the
    // trait.
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        .start()
        .await;
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    let pool = Arc::new(ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    ));

    let interactive: Arc<dyn MailBackend> = Arc::new(ImapBackend::over(Arc::clone(&pool)));
    let background = interactive
        .for_background()
        .expect("an IMAP backend has priorities to give");

    interactive.connect().await.expect("connect");
    background
        .list_mailboxes(&MailboxFilter::all())
        .await
        .expect("list");
    assert_eq!(
        pool.stats().opened,
        1,
        "the background view must be the same pool, not a second one"
    );
}
