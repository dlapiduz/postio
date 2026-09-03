//! The real client stack against a real socket.
//!
//! Every other test in this crate replays a transcript ([`ImapScript`]) or
//! stops at the `MailBackend` seam ([`MockBackend`](postio_account::backend::MockBackend)).
//! Neither exercises `io-imap`: the script cannot answer a command nobody
//! wrote down, and the mock sits above the protocol entirely. `io-imap` is
//! pre-1.0 and shipped six minor releases in eleven weeks (ADR 0001), so it
//! is the layer most likely to regress under us and the one nothing was
//! watching.
//!
//! These tests drive `ConnectionPool` — real sockets, real `io-imap`, real
//! session and auth path — against [`TestServer`], an in-process IMAP server
//! on an ephemeral loopback port. Nothing here touches the network.

use std::sync::Arc;
use std::time::Duration;

use io_imap::client::ImapClientAsync;
use io_imap::types::core::Vec1;
use io_imap::types::extensions::enable::CapabilityEnable;
use postio_account::backend::{
    AppendMessage, BackendError, BodyPart, Capability, FlagChange, MailboxEvent, MailboxFilter,
    SelectMode, UidSet, VecSink,
};
use postio_account::cancel::CancelToken;
use postio_account::imap::{
    ConnectionPool, PoolConfig, Priority, RustlsConnector, append, copy_messages, expunge,
    fetch_headers, fetch_part, idle, list_mailboxes, move_messages, select, status, store_flags,
};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{Fault, Quirk, TestMailbox, TestMessage, TestServer};
use postio_model::{Flag, FlagSet, ModSeq, Uid, UidValidity};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

/// What a mainstream provider advertises after login.
const FULL: [&str; 12] = [
    "IMAP4rev1",
    "SASL-IR",
    "AUTH=PLAIN",
    "ENABLE",
    "CONDSTORE",
    "QRESYNC",
    "IDLE",
    "UIDPLUS",
    "MOVE",
    "NAMESPACE",
    "UNSELECT",
    "ID",
];

/// A server shaped like the account this project targets: a provider that
/// hides its extensions until you log in and names no special-use folders.
async fn server() -> TestServer {
    TestServer::builder()
        .mailbox(
            TestMailbox::new("INBOX")
                .uid_validity(UidValidity::new(4_242))
                .highest_mod_seq(ModSeq::new(900))
                .corpus(SEEDED),
        )
        .mailbox(TestMailbox::new("Archive"))
        .mailbox(TestMailbox::new("Sent Messages"))
        .start()
        .await
}

/// The same server, minus the named capabilities — how a fallback path is
/// put under test rather than assumed.
async fn without(dropped: &[&str]) -> TestServer {
    let kept: Vec<&str> = FULL
        .iter()
        .copied()
        .filter(|name| !dropped.contains(name))
        .collect();

    TestServer::builder()
        .capabilities(kept)
        .mailbox(
            TestMailbox::new("INBOX")
                .uid_validity(UidValidity::new(4_242))
                .highest_mod_seq(ModSeq::new(900))
                .corpus(SEEDED),
        )
        .mailbox(TestMailbox::new("Archive"))
        .start()
        .await
}

async fn pool_for(server: &TestServer) -> ConnectionPool {
    pool_with(server, PoolConfig::default()).await
}

async fn pool_with(server: &TestServer, config: PoolConfig) -> ConnectionPool {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");

    ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        config,
    )
}

/// How many times the server has been asked to select a mailbox.
fn selects(server: &TestServer) -> usize {
    server
        .commands()
        .iter()
        .filter(|command| command.to_ascii_uppercase().contains(" SELECT "))
        .count()
}

fn uids(values: impl IntoIterator<Item = u32>) -> Vec<postio_model::RemoteId> {
    // The server above pins its generation to 4242, so these are the ids
    // its adapter mints.
    values.into_iter().map(rid).collect()
}

/// The identity the pinned generation gives `uid`.
fn rid(uid: u32) -> postio_model::RemoteId {
    postio_model::RemoteId::new(format!("4242:{uid}"))
}

fn cancel() -> CancelToken {
    CancelToken::new()
}

// ---------------------------------------------------------------------------
// The stack, end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_client_stack_lists_folders_and_fetches_headers_over_a_socket() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();
    let paths: Vec<&str> = mailboxes
        .iter()
        .map(|mailbox| mailbox.path.as_str())
        .collect();
    assert_eq!(paths, ["INBOX", "Archive", "Sent Messages"]);

    let messages = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        None,
        Priority::Interactive,
        &cancel(),
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 3);
    let first = &messages[0];
    assert_eq!(first.uid, Uid::new(1));
    assert_eq!(first.uid_validity, UidValidity::new(4_242));
    assert_eq!(first.size, corpus("plain-text-simple").len() as u64);

    let envelope = first.envelope.as_ref().expect("an envelope");
    assert_eq!(
        envelope.subject.as_deref(),
        Some("Tuesday walkthrough notes")
    );
    assert_eq!(envelope.from[0].address, "ada.norwood@example.com");

    // BODYSTRUCTURE, round-tripped through the wire and back into the model:
    // the multipart tree, its section numbers, and the filename an attachment
    // is offered under. None of that is reachable from a mock that takes a
    // structure as given.
    let structure = messages[1]
        .structure
        .as_ref()
        .expect("attachment-pdf is a multipart");
    assert_eq!(structure.text_part().map(|part| part.section()), Some("1"));
    let attachments: Vec<&str> = structure
        .attachments()
        .filter_map(|part| part.filename())
        .collect();
    assert_eq!(attachments, ["layout-rev-c.pdf", "checksums.txt"]);
    assert_eq!(
        structure
            .attachments()
            .next()
            .map(|part| part.mime_type().to_owned()),
        Some("application/pdf".to_owned())
    );

    // The extensions this provider hides until login are the ones every fast
    // path is gated on — ADR 0001, Q3.
    let capabilities = pool.capabilities().expect("a session was opened");
    assert!(capabilities.supports_incremental_sync());
    assert!(capabilities.contains(Capability::UidPlus));
}

#[tokio::test]
async fn a_whole_body_streams_off_the_socket_byte_for_byte() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let mut sink = VecSink::new();

    let fetched = fetch_part(
        &pool,
        "INBOX",
        &rid(2),
        &BodyPart::Whole,
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();

    let expected = corpus("attachment-pdf");
    assert_eq!(fetched.bytes_written, expected.len() as u64);
    assert_eq!(sink.as_slice(), expected.as_slice());
    assert!(sink.is_finished());
}

#[tokio::test]
async fn one_mime_section_is_fetched_without_the_rest_of_the_message() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let mut sink = VecSink::new();

    fetch_part(
        &pool,
        "INBOX",
        &rid(2),
        &BodyPart::Section("1".to_owned()),
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();

    // Section 1 of a multipart is its first part's *content*: no message
    // headers, no MIME headers of its own, and nothing of the part after it.
    let whole = String::from_utf8(corpus("attachment-pdf")).expect("a utf-8 fixture");
    let section = String::from_utf8(sink.into_inner()).expect("the text part is utf-8");

    assert!(section.starts_with("Signed and attached."), "{section:?}");
    assert!(whole.contains(&section));
    assert!(!section.contains("=_mixed_pdf"), "the boundary leaked in");
    assert!(!section.contains("Content-Type"));
}

// ---------------------------------------------------------------------------
// Incremental sync
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_changedsince_fetch_returns_only_what_moved() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let floor = ModSeq::new(server.highest_mod_seq("INBOX"));

    server.set_flags("INBOX", Uid::new(2), FlagSet::from_iter([Flag::Seen]));

    let changed = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        Some(floor),
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();

    let uids: Vec<u32> = changed.iter().map(|message| message.uid.get()).collect();
    assert_eq!(uids, [2]);
    assert!(changed[0].flags.is_seen());
    assert!(changed[0].mod_seq > Some(floor));
}

#[tokio::test]
async fn a_delivery_is_visible_to_the_next_changedsince_fetch() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let floor = ModSeq::new(server.highest_mod_seq("INBOX"));

    let uid = server.deliver("INBOX", TestMessage::corpus("list-thread-01-root"));

    let changed = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        Some(floor),
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();

    let uids: Vec<u32> = changed.iter().map(|message| message.uid.get()).collect();
    assert_eq!(uids, [uid.get()]);
}

#[tokio::test]
async fn a_uidvalidity_bump_is_refused_rather_than_acted_on() {
    // The worst thing this codebase can do: a mailbox is renumbered, the
    // session's cached SELECT still reports the old generation, and UIDs from
    // the new one are read as though they were the old ones. Flags land on
    // the wrong messages and a delete hits mail the user never chose — with
    // nothing erroring, because every layer above believes the generation it
    // was told.
    let server = server().await;
    let pool = pool_with(
        &server,
        PoolConfig {
            // Trust no cached selection across a checkout, so the renumber is
            // caught on the very next operation rather than up to
            // `selection_max_age` later.
            selection_max_age: Duration::ZERO,
            ..PoolConfig::default()
        },
    )
    .await;

    let before = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(1)),
        None,
        Priority::Interactive,
        &cancel(),
    )
    .await
    .unwrap();
    assert_eq!(before[0].uid_validity, UidValidity::new(4_242));

    server.set_uid_validity("INBOX", UidValidity::new(9_001));

    // No mailbox switch in between: the same mailbox, the same pool, the next
    // operation.
    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(1)),
        None,
        Priority::Interactive,
        &cancel(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            &error,
            BackendError::UidValidityChanged { mailbox, known, observed }
                if mailbox == "INBOX"
                    && *known == UidValidity::new(4_242)
                    && *observed == UidValidity::new(9_001)
        ),
        "{error}"
    );
    assert!(error.requires_full_resync());

    // Reported once, then work resumes in the new generation: the caller
    // rebuilds and retries, and the retry must not keep failing.
    let after = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(1)),
        None,
        Priority::Interactive,
        &cancel(),
    )
    .await
    .unwrap();
    assert_eq!(after[0].uid_validity, UidValidity::new(9_001));
}

#[tokio::test]
async fn a_body_fetch_is_refused_across_a_uidvalidity_bump_too() {
    // Bodies are the half that attaches downloaded bytes to a row, so a
    // generation the caller did not ask for has to stop this path as well.
    let server = server().await;
    let pool = pool_with(
        &server,
        PoolConfig {
            selection_max_age: Duration::ZERO,
            ..PoolConfig::default()
        },
    )
    .await;
    let mut sink = VecSink::new();

    fetch_part(
        &pool,
        "INBOX",
        &rid(1),
        &BodyPart::Whole,
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();

    server.set_uid_validity("INBOX", UidValidity::new(9_001));

    let mut sink = VecSink::new();
    let error = fetch_part(
        &pool,
        "INBOX",
        &rid(1),
        &BodyPart::Whole,
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap_err();

    assert!(error.requires_full_resync(), "{error}");
    assert!(sink.is_empty(), "no bytes may reach the sink");
}

#[tokio::test]
async fn consecutive_fetches_on_one_mailbox_still_select_once() {
    // The cache this guard sits on is a real optimisation: a backfill is many
    // small fetches on one mailbox, and re-selecting before each of them
    // would double the round trips. Making staleness impossible must not cost
    // that.
    let server = server().await;
    let pool = pool_for(&server).await;

    for uid in [1u32, 2, 3] {
        fetch_headers(
            &pool,
            "INBOX",
            &UidSet::single(Uid::new(uid)),
            None,
            Priority::Background,
            &cancel(),
        )
        .await
        .unwrap();
    }

    assert_eq!(
        selects(&server),
        1,
        "one SELECT for three chunks: {:?}",
        server.commands()
    );
}

#[tokio::test]
async fn a_qresync_select_reports_the_changes_and_the_vanishes_together() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let baseline = ModSeq::new(server.highest_mod_seq("INBOX"));
    server.set_flags("INBOX", Uid::new(1), FlagSet::from_iter([Flag::Flagged]));
    server.vanish("INBOX", Uid::new(3));

    let mut session = pool.acquire(Priority::Background).await.unwrap();
    let data = session
        .select_qresync(
            io_imap::types::mailbox::Mailbox::try_from("INBOX").unwrap(),
            std::num::NonZeroU32::new(4_242).unwrap(),
            baseline.get(),
            &[io_imap::types::response::Capability::QResync],
        )
        .await
        .unwrap();

    let vanished: Vec<u32> = data.vanished_earlier.iter().map(|uid| uid.get()).collect();
    assert_eq!(vanished, [3]);
    assert_eq!(data.changed.len(), 1);
    assert!(data.highest_mod_seq.unwrap() > baseline.get());
}

#[tokio::test]
async fn a_server_without_condstore_refuses_the_incremental_path_rather_than_guessing() {
    // Every extension is optional and Postio will meet servers without this
    // one. The capability list is the only thing the choice may be made from,
    // so dropping CONDSTORE here has to surface as a refusal, not as a fetch
    // that quietly returns everything.
    let server = TestServer::builder()
        .capabilities(["IMAP4rev1", "SASL-IR", "AUTH=PLAIN", "ENABLE"])
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        .start()
        .await;
    let pool = pool_for(&server).await;

    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        Some(ModSeq::new(1)),
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        BackendError::Unsupported {
            capability: Capability::CondStore
        }
    ));

    // The floor path still works, and reports no modification sequence
    // because the server never offered one.
    let messages = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        None,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].mod_seq, None);
}

#[tokio::test]
async fn a_subscribed_only_listing_asks_the_server_which_folders_those_are() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX"))
        .mailbox(TestMailbox::new("Archive").subscribed(false))
        .start()
        .await;
    let pool = pool_for(&server).await;

    let all = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();
    let subscribed = list_mailboxes(&pool, &MailboxFilter::subscribed(), Priority::Interactive)
        .await
        .unwrap();

    assert_eq!(all.len(), 2);
    let paths: Vec<&str> = subscribed
        .iter()
        .map(|mailbox| mailbox.path.as_str())
        .collect();
    assert_eq!(paths, ["INBOX"]);
    assert!(
        server
            .commands()
            .iter()
            .any(|command| command.contains("LSUB")),
        "the subscribed listing has to ask: {:?}",
        server.commands()
    );
}

// ---------------------------------------------------------------------------
// The lies a real server tells
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_malformed_fetch_sequence_number_fails_the_resync_rather_than_losing_mail() {
    // At least one mainstream provider has shipped `* -1 FETCH (…)` under
    // QRESYNC. `io-imap` skips an untagged line it cannot decode and completes
    // the command `Ok` (ADR 0001, iCloud hazard 2), so the pull looks complete
    // while a message's flags silently never arrived.
    let server = server().await;
    let pool = pool_for(&server).await;
    let floor = ModSeq::new(server.highest_mod_seq("INBOX"));

    server.set_flags("INBOX", Uid::new(2), FlagSet::from_iter([Flag::Seen]));
    server.quirk(Quirk::MalformedFetchSequenceNumber);

    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        Some(floor),
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, BackendError::ResyncIntegrityLost { .. }));
    assert!(error.requires_full_resync());
}

#[tokio::test]
async fn a_connection_dropped_mid_fetch_is_a_transient_error() {
    let server = server().await;
    let pool = pool_for(&server).await;
    server.inject(Fault::DropConnection {
        during: "FETCH".to_owned(),
    });

    let mut sink = VecSink::new();
    let error = fetch_part(
        &pool,
        "INBOX",
        &rid(1),
        &BodyPart::Whole,
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap_err();

    assert!(error.is_transient(), "{error}");
    assert!(!sink.is_finished(), "a torn fetch never finishes its sink");
}

#[tokio::test]
async fn a_missing_enabled_echo_is_success_not_a_downgrade() {
    // RFC 5161 §3.1 requires the untagged `* ENABLED` line. At least one
    // mainstream provider has omitted it; gating QRESYNC on the echo rather
    // than on the post-auth capability list is how a client silently
    // degrades to full resync forever. ADR 0001, hazard 1.
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        .quirk(Quirk::OmitEnabledEcho)
        .start()
        .await;
    let pool = pool_for(&server).await;

    let mut session = pool.acquire(Priority::Background).await.unwrap();
    let echoed = session
        .enable(Vec1::from(CapabilityEnable::CondStore))
        .await
        .unwrap();

    assert!(echoed.is_none(), "no echo, and no error either");
    assert!(session.capabilities().contains(Capability::QResync));
}

#[tokio::test]
async fn a_stalled_server_fails_the_command_and_frees_the_connection() {
    // The pool is bounded, so a server that accepts a command and never
    // answers does not cost one connection — it costs one of four,
    // permanently. Enough of those and sync stops with nothing logged, which
    // reads to the user as the app quietly ceasing to work.
    let server = server().await;
    let pool = pool_with(
        &server,
        PoolConfig {
            command_timeout: Duration::from_millis(200),
            ..PoolConfig::default()
        },
    )
    .await;
    server.inject(Fault::Stall {
        during: "FETCH".to_owned(),
    });

    let started = tokio::time::Instant::now();
    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        None,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, BackendError::TimedOut { .. }), "{error}");
    assert!(error.is_transient(), "the caller has to be able to retry");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "it gave up promptly"
    );

    // The connection is discarded rather than parked, so the retry gets a
    // fresh one and the slot is not poisoned for everything after it.
    let recovered = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::all(),
        None,
        Priority::Background,
        &cancel(),
    )
    .await
    .unwrap();
    assert_eq!(recovered.len(), 3);
    assert_eq!(
        pool.stats().opened,
        2,
        "the stalled connection was replaced, not reused"
    );
}

#[tokio::test]
async fn a_slow_but_progressing_fetch_is_not_killed_by_the_deadline() {
    // The trap in bounding a command: a large attachment over a slow link
    // takes longer than any deadline worth setting on a SELECT. So the bound
    // is on silence, not on duration — a response that keeps arriving keeps
    // its connection however long it takes.
    let server = server().await;
    let pool = pool_with(
        &server,
        PoolConfig {
            command_timeout: Duration::from_millis(500),
            ..PoolConfig::default()
        },
    )
    .await;
    server.inject(Fault::Trickle {
        during: "FETCH".to_owned(),
        gap: Duration::from_millis(100),
    });

    let mut sink = VecSink::new();
    let started = tokio::time::Instant::now();
    fetch_part(
        &pool,
        "INBOX",
        &rid(2),
        &BodyPart::Whole,
        &mut sink,
        Priority::Background,
        &cancel(),
    )
    .await
    .expect("a response that never stops for longer than the deadline survives it");

    assert_eq!(sink.as_slice(), corpus("attachment-pdf").as_slice());
    assert!(
        started.elapsed() > Duration::from_millis(500),
        "the fetch outlived the deadline it never tripped"
    );
}

#[tokio::test]
async fn a_server_that_never_finishes_the_handshake_does_not_hold_the_opening() {
    // The same hang, one layer earlier: the socket connected and the TLS
    // handshake finished, so `connect_timeout` had already been satisfied
    // before the exchange that never completes.
    let server = server().await;
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    let pool = ConnectionPool::new(
        server
            .settings()
            .with_connect_timeout(Duration::from_millis(200)),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    );
    server.inject(Fault::Stall {
        during: "AUTHENTICATE".to_owned(),
    });

    let error = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::TimedOut { .. }), "{error}");
    assert!(error.is_transient());
}

// ---------------------------------------------------------------------------
// Mutating commands
// ---------------------------------------------------------------------------

#[tokio::test]
async fn storing_flags_reports_what_they_are_now() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let updates = store_flags(
        &pool,
        "INBOX",
        &uids([1, 2]),
        &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        Priority::Interactive,
    )
    .await
    .unwrap();

    assert_eq!(updates.len(), 2);
    assert!(updates.iter().all(|update| update.flags.is_seen()));
    assert!(
        updates.iter().all(|update| update.mod_seq.is_some()),
        "a CONDSTORE server stamps every change"
    );
    assert!(server.flags("INBOX", Uid::new(1)).is_seen());
    assert!(!server.flags("INBOX", Uid::new(3)).is_seen());
}

#[tokio::test]
async fn moving_uses_move_where_the_server_has_it() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let mapping = move_messages(&pool, "INBOX", &uids([1]), "Archive", Priority::Background)
        .await
        .unwrap();

    assert_eq!(mapping.len(), 1, "UIDPLUS reports where it landed");
    assert_eq!(mapping[0].source, Uid::new(1));
    assert_eq!(server.uids("Archive"), vec![Uid::new(1)]);
    assert_eq!(server.uids("INBOX"), vec![Uid::new(2), Uid::new(3)]);
    assert!(
        server
            .commands()
            .iter()
            .any(|command| command.to_ascii_uppercase().contains("UID MOVE"))
    );
}

#[tokio::test]
async fn moving_without_move_copies_then_deletes() {
    // Three round trips instead of one, and not atomic: a crash between the
    // copy and the store leaves the message in both mailboxes. That is the
    // cost of the fallback, and it is the caller's to tolerate.
    let server = without(&["MOVE"]).await;
    let pool = pool_for(&server).await;

    move_messages(&pool, "INBOX", &uids([1]), "Archive", Priority::Background)
        .await
        .unwrap();

    assert_eq!(server.uids("Archive").len(), 1);
    assert_eq!(server.uids("INBOX"), vec![Uid::new(2), Uid::new(3)]);

    let commands = server.commands().join("\n").to_ascii_uppercase();
    assert!(commands.contains("UID COPY"), "{commands}");
    assert!(commands.contains("UID STORE"), "{commands}");
    assert!(commands.contains("UID EXPUNGE"), "{commands}");
    assert!(!commands.contains("UID MOVE"), "{commands}");
}

#[tokio::test]
async fn moving_without_uidplus_leaves_the_source_marked_rather_than_expunged() {
    // Without UIDPLUS the only expunge available also removes messages
    // another client marked `\Deleted`. Losing somebody else's mail is worse
    // than leaving ours in place, so the removal is left to the server.
    let server = without(&["MOVE", "UIDPLUS"]).await;
    let pool = pool_for(&server).await;

    let mapping = move_messages(&pool, "INBOX", &uids([1]), "Archive", Priority::Background)
        .await
        .unwrap();

    assert!(
        mapping.is_empty(),
        "no UIDPLUS, so no destination to report"
    );
    assert_eq!(server.uids("Archive").len(), 1, "the copy still happened");
    assert!(
        server.uids("INBOX").contains(&Uid::new(1)),
        "the source is still there"
    );
    assert!(server.flags("INBOX", Uid::new(1)).is_deleted());
    assert!(
        !server
            .commands()
            .join("\n")
            .to_ascii_uppercase()
            .contains("EXPUNGE")
    );
}

#[tokio::test]
async fn copying_reports_the_destination_only_when_uidplus_said_so() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let with_uidplus = copy_messages(&pool, "INBOX", &uids([1]), "Archive", Priority::Background)
        .await
        .unwrap();
    assert_eq!(with_uidplus.len(), 1);
    assert_eq!(with_uidplus[0].destination, Uid::new(1));

    let bare = without(&["UIDPLUS"]).await;
    let bare_pool = pool_for(&bare).await;
    let without_uidplus = copy_messages(
        &bare_pool,
        "INBOX",
        &uids([1]),
        "Archive",
        Priority::Background,
    )
    .await
    .unwrap();

    assert!(
        without_uidplus.is_empty(),
        "a destination UID has to be searched for, not guessed"
    );
    assert_eq!(bare.uids("Archive").len(), 1, "the copy still happened");
}

#[tokio::test]
async fn appending_lands_the_message_and_reports_where() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let message = AppendMessage::new(b"Subject: a draft\r\n\r\nnot sent yet\r\n".to_vec())
        .with_flags(FlagSet::from_iter([Flag::Draft]));
    let landed = append(&pool, "Archive", &message, Priority::Interactive)
        .await
        .unwrap();

    let landed = landed.expect("UIDPLUS is advertised, so APPENDUID is reported");
    assert_eq!(landed.destination, Uid::new(1));
    assert_eq!(landed.uid_validity, server.uid_validity("Archive"));
    assert!(server.flags("Archive", Uid::new(1)).is_draft());
}

#[tokio::test]
async fn a_targeted_expunge_without_uidplus_is_deferred_rather_than_widened() {
    let server = without(&["UIDPLUS"]).await;
    let pool = pool_for(&server).await;

    // One we marked, one another client marked.
    store_flags(
        &pool,
        "INBOX",
        &uids([1]),
        &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        Priority::Background,
    )
    .await
    .unwrap();
    server.set_flags("INBOX", Uid::new(3), FlagSet::from_iter([Flag::Deleted]));

    let gone = expunge(&pool, "INBOX", Some(&uids([1])), Priority::Background)
        .await
        .unwrap();

    assert!(gone.is_empty());
    assert_eq!(
        server.uids("INBOX").len(),
        3,
        "nothing may be destroyed to make our own expunge convenient"
    );
}

#[tokio::test]
async fn an_untargeted_expunge_removes_every_deleted_message() {
    let server = server().await;
    let pool = pool_for(&server).await;

    store_flags(
        &pool,
        "INBOX",
        &uids([2]),
        &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        Priority::Background,
    )
    .await
    .unwrap();

    expunge(&pool, "INBOX", None, Priority::Background)
        .await
        .unwrap();

    assert_eq!(server.uids("INBOX"), vec![Uid::new(1), Uid::new(3)]);
}

#[tokio::test]
async fn status_reports_a_mailbox_without_selecting_it() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let status = status(&pool, "INBOX", Priority::Background).await.unwrap();

    assert_eq!(status.exists, 3);
    assert_eq!(status.generation, postio_model::Generation::new(4_242));
    assert_eq!(status.uid_next, Uid::new(4));
    assert_eq!(status.highest_mod_seq, Some(ModSeq::new(900)));
    assert_eq!(
        selects(&server),
        0,
        "the cheap per-mailbox check must stay cheap: {:?}",
        server.commands()
    );
}

#[tokio::test]
async fn selecting_reports_the_mailbox_and_whether_it_can_be_written() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let writable = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Interactive)
        .await
        .unwrap();
    assert_eq!(writable.exists, 3);
    assert_eq!(writable.generation, postio_model::Generation::new(4_242));
    assert!(!writable.read_only);
    assert!(
        writable.permanent_flags.is_seen(),
        "the server said \\Seen is permanent"
    );

    let readable = select(&pool, "INBOX", SelectMode::ReadOnly, Priority::Interactive)
        .await
        .unwrap();
    assert!(readable.read_only);
    assert!(
        server
            .commands()
            .iter()
            .any(|command| command.to_ascii_uppercase().contains("EXAMINE"))
    );
}

// ---------------------------------------------------------------------------
// IDLE
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idle_returns_the_change_the_server_pushed() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let token = cancel();
    let (events, _) = tokio::join!(
        idle(&pool, "INBOX", Duration::from_secs(5), &token),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            server.deliver("INBOX", TestMessage::corpus("list-thread-01-root"))
        }
    );

    let events = events.unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MailboxEvent::Exists { .. })),
        "{events:?}"
    );
    // This server hides IDLE from its banner, like the provider Postio
    // targets. Gating on the post-auth capability list rather than on the
    // greeting is the difference between watching and polling forever.
    assert!(
        server
            .commands()
            .iter()
            .any(|command| command.contains("IDLE")),
        "{:?}",
        server.commands()
    );
}

#[tokio::test]
async fn idle_returns_empty_when_nothing_happens_before_the_timeout() {
    let server = server().await;
    let pool = pool_for(&server).await;

    let events = idle(&pool, "INBOX", Duration::from_millis(150), &cancel())
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "nothing happened, which is not a failure"
    );
}

#[tokio::test]
async fn idle_returns_empty_when_cancelled() {
    let server = server().await;
    let pool = pool_for(&server).await;
    let token = CancelToken::new();

    let (events, ()) = tokio::join!(
        idle(&pool, "INBOX", Duration::from_secs(30), &token),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        }
    );

    assert!(events.unwrap().is_empty());
}

/// How many `IDLE` commands the server has logged so far.
fn idle_armings(server: &TestServer) -> usize {
    server
        .commands()
        .iter()
        .filter(|command| command.contains("IDLE"))
        .count()
}

/// Waits until the server has seen at least `count` `IDLE` arm-ups.
///
/// Observed rather than scheduled (#80): the property under test is "the
/// watcher re-arms before the server's patience runs out", which is a fact
/// about how many times `IDLE` was sent, not about how much real time
/// passed. A sleep long enough to outlast the server's limit on a quiet
/// machine is also long enough to be outlasted *by* the limit on a loaded
/// one — the two wall-clock budgets blur together under exactly the load
/// this project's own CLAUDE.md says is ordinary. Waiting for the count
/// directly means a slow re-arm only makes the test slower, never wrong; the
/// timeout here is a liveness bound for a genuinely deaf watcher, not a
/// measurement.
async fn wait_for_idle_armings(server: &TestServer, count: usize) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while idle_armings(server) < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the watcher never armed IDLE the expected number of times");
}

#[tokio::test]
async fn idle_re_arms_before_the_server_gets_impatient() {
    // The failure this guards is silent: a server drops an IDLE that has run
    // too long, and the watcher goes deaf without an error anywhere. New mail
    // simply stops appearing.
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        // 50x the refresh interval below, not 5x: the margin is what used to
        // blur under load, not the property itself (re-arm cadence < server
        // limit is unchanged either way). #80.
        .idle_limit(Duration::from_secs(5))
        .start()
        .await;
    let pool = pool_with(
        &server,
        PoolConfig {
            watch_refresh: Duration::from_millis(100),
            ..PoolConfig::default()
        },
    )
    .await;

    let token = cancel();
    let (events, _) = tokio::join!(
        idle(&pool, "INBOX", Duration::from_secs(5), &token),
        async {
            // Deliver once the watcher has demonstrably re-armed at least
            // once beyond its first IDLE, rather than sleeping past where
            // that "should" have happened by now.
            wait_for_idle_armings(&server, 2).await;
            server.deliver("INBOX", TestMessage::corpus("list-thread-01-root"))
        }
    );

    assert!(!events.unwrap().is_empty(), "the watcher went deaf");
    let armings = idle_armings(&server);
    assert!(
        armings > 1,
        "IDLE was armed {armings} times, never re-armed"
    );
}

#[tokio::test]
async fn a_server_without_idle_is_polled_instead() {
    let server = TestServer::builder()
        .capabilities(["IMAP4rev1", "SASL-IR", "AUTH=PLAIN", "ENABLE", "CONDSTORE"])
        .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
        .start()
        .await;
    let pool = pool_with(
        &server,
        PoolConfig {
            watch_poll_interval: Duration::from_millis(50),
            ..PoolConfig::default()
        },
    )
    .await;

    let token = cancel();
    let (events, _) = tokio::join!(
        idle(&pool, "INBOX", Duration::from_secs(5), &token),
        async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            server.deliver("INBOX", TestMessage::corpus("list-thread-01-root"))
        }
    );

    assert!(
        events
            .unwrap()
            .iter()
            .any(|event| matches!(event, MailboxEvent::Exists { count: 2 })),
        "polling has to notice the arrival"
    );
    let commands = server.commands();
    assert!(commands.iter().any(|command| command.contains("STATUS")));
    assert!(
        !commands.iter().any(|command| command.contains("IDLE")),
        "a server that never advertised IDLE must not be sent one"
    );
}

#[tokio::test]
async fn a_command_the_server_does_not_implement_is_refused_loudly() {
    let server = server().await;
    let mut raw = RawClient::connect(&server).await;

    raw.command("a1 LOGIN {user} {password}").await;
    raw.send("a2 GETQUOTAROOT INBOX\r\n").await;

    let reply = raw.expect_prefix("a2").await;
    assert!(reply.starts_with("a2 BAD"), "{reply}");
}

// ---------------------------------------------------------------------------
// A hand-written client, for the commands `postio-account` cannot issue yet
// ---------------------------------------------------------------------------

struct RawClient {
    stream: tokio::net::TcpStream,
    buffer: Vec<u8>,
    account: String,
    password: String,
}

impl RawClient {
    async fn connect(server: &TestServer) -> Self {
        let stream = tokio::net::TcpStream::connect(server.addr())
            .await
            .expect("connect to the test server");
        let mut client = Self {
            stream,
            buffer: Vec::new(),
            account: server.account().to_owned(),
            password: server.password().to_owned(),
        };
        client.expect_prefix("*").await;
        client
    }

    async fn send(&mut self, text: &str) {
        use tokio::io::AsyncWriteExt;
        let text = text
            .replace("{user}", &self.account)
            .replace("{password}", &self.password);
        self.stream
            .write_all(text.as_bytes())
            .await
            .expect("write to the test server");
    }

    /// Sends a command and reads until its tagged response.
    async fn command(&mut self, text: &str) {
        let tag = text.split(' ').next().unwrap().to_owned();
        self.send(&format!("{text}\r\n")).await;
        let reply = self.expect_prefix(&tag).await;
        assert!(reply.contains(" OK"), "{reply}");
    }

    /// Reads lines until one starts with `prefix`, and returns it.
    async fn expect_prefix(&mut self, prefix: &str) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let line = tokio::time::timeout_at(deadline, self.read_line())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for a line starting {prefix:?}"));
            if line.starts_with(prefix) {
                return line;
            }
        }
    }

    async fn read_line(&mut self) -> String {
        use tokio::io::AsyncReadExt;
        loop {
            if let Some(end) = self.buffer.windows(2).position(|window| window == b"\r\n") {
                let line = String::from_utf8_lossy(&self.buffer[..end]).into_owned();
                self.buffer.drain(..end + 2);
                return line;
            }
            let mut chunk = [0u8; 4096];
            let read = self.stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "the server closed the connection");
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

fn corpus(name: &str) -> Vec<u8> {
    postio_model::test_corpus::load(name).bytes().to_vec()
}
