//! No level, ever, puts message content in the log.
//!
//! Email is the most sensitive thing on most people's machines, and a debug
//! log full of it is exactly the leak `CLAUDE.md` states as a rule. A rule
//! nothing checks is a rule that lasts until the next person adds a `?message`
//! to a `tracing` call because it would have been convenient that once.
//!
//! So this drives the sync path — the engine, `postio-sync`, `postio-imap` and
//! `postio-storage`, which are the crates message content actually flows
//! through — with a subscriber capturing **everything at `TRACE`**, and then
//! looks for the store's own subjects, previews and sender addresses in what
//! came out.
//!
//! # Why it reads the content out of the database
//!
//! Asserting against a hard-coded list of forbidden strings would only ever
//! catch the strings somebody remembered to list. Reading them back out of the
//! seeded store means the test is checking the *actual* mail this run handled:
//! add a fixture, and it is covered without touching this file.
//!
//! # What is allowed through
//!
//! Ids, counts, durations, outcomes, capability names, folder paths and a
//! server endpoint. A folder is a container the user named, not a message; a
//! capability list is the server's public advertisement; an endpoint is a
//! hostname they chose to connect to. None of them says anything about who
//! wrote to them or what was said.

use std::io;
use std::sync::{Arc, Mutex};

use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_model::MailboxRole;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A writer every `tracing` line lands in, so a test can read them back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Every string in the store that would be a leak if it appeared in a log.
///
/// Subjects and sender addresses whole; previews trimmed to a distinctive
/// prefix, because a whole preview is long enough that a formatter could wrap
/// or truncate it and the test would pass by accident.
fn content_of(
    database: &postio_storage::Database,
    account: postio_model::AccountId,
) -> Vec<String> {
    let connection = database.connection().expect("a connection");
    let rows = MessageRepository::new(&connection)
        .page(&ListQuery::account(account).limit(500))
        .expect("reading the seeded mail");
    assert!(!rows.is_empty(), "the fixture seeded no mail to protect");

    let mut secrets = Vec::new();
    // The account's own address too. It is not message content, but it is the
    // user's identity, and an error naming it — "no password is stored for
    // ada@example.com" — is right on screen and wrong in a log.
    {
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
            .expect("reading the account")
            .into_iter()
            .next()
            .expect("the fixture made one");
        secrets.push(account.address.address.clone());
    }
    for row in rows {
        if let Some(subject) = row.subject.filter(|s| s.trim().len() >= 8) {
            secrets.push(subject.trim().to_string());
        }
        if let Some(preview) = row.preview.filter(|p| p.trim().len() >= 24) {
            secrets.push(preview.trim().chars().take(24).collect());
        }
        if let Some(from) = row.from {
            secrets.push(from.address.clone());
            if let Some(name) = from.name.clone().filter(|n| n.trim().len() >= 5) {
                secrets.push(name);
            }
        }
    }
    secrets.sort();
    secrets.dedup();
    secrets
}

fn server() -> MockBackend {
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
                To: Postio <postio@example.net>\r\n\
                Subject: a message the server holds\r\n\
                Message-ID: <held-1@example.com>\r\n\
                Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                \r\n\
                The bytes that had to travel to get here.\r\n"
        .to_vec();
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::new(raw)))
        .build()
}

#[test]
fn no_message_content_reaches_the_log_at_any_level() {
    let captured = Captured::default();

    // Everything, with no filter at all: this has to hold at `trace` in a
    // debug build, which is the loudest the application can ever be.
    //
    // Globally, not per-thread: the engine does its work on a thread of its
    // own, and `set_default` would have left that thread — the one that
    // actually handles mail — with no subscriber and this test asserting over
    // an empty buffer. That mistake passes, which is the worst kind. Hence
    // also the assertion further down that the engine's own lines are present
    // before anything is concluded from their absence.
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("this test binary runs one test and owns the subscriber");

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let secrets = content_of(&database, report.account.id);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = postio_core::bridge::event_channel();

    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: Arc::new(server()),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_imap::auth::StoredPasswordSource::new(Arc::new(
            postio_imap::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    // Drive the paths that handle mail: connect, drain, sync a real folder,
    // and seed a backfill over the seeded messages.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    runtime.block_on(async {
        let _ = engine.drain().await;
        let _ = engine.sync(inbox).await;
        let _ = engine.seed_backfill(inbox, 50).await;
    });
    drop(engine);

    let log = captured.text();
    // Proof that the paths under test actually ran and were observed. Without
    // it an engine that logged nothing — or a subscriber that never reached
    // its thread — would pass this test while proving nothing at all.
    for expected in ["engine{account=", ":drain:", ":sync{"] {
        assert!(
            log.contains(expected),
            "the log has no `{expected}` span, so the sync path was never \
             observed and this test proves nothing:\n{log}"
        );
    }

    let leaked: Vec<&String> = secrets
        .iter()
        .filter(|secret| log.contains(secret.as_str()))
        .collect();

    assert!(
        leaked.is_empty(),
        "{} of the store's own subjects, previews or sender addresses reached \
         the log. Log ids, counts, folder paths, durations and outcomes — never \
         what a message says or who sent it. Leaked: {:?}",
        leaked.len(),
        leaked
    );
}
