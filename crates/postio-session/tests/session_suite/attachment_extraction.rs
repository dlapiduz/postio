//! Extraction in a child process (spec 010 D28, T153).
//!
//! The indexer never reads an attachment in its own process: it hands the
//! bytes to `postio-extract-helper` and kills it at the deadline. These
//! are the ways that can go: the real helper, built from this source and
//! handed over by cargo, answering exactly what an in-process extraction
//! would; and stand-ins written here as shell scripts for the helpers
//! nobody can build on purpose -- one that never answers, one that dies,
//! one that answers garbage, one from another build -- plus no helper at
//! all.
//!
//! The stand-ins are scripts rather than modes of the real helper so the
//! shipped binary carries no test switch a file could reach.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use postio_extract::{Limits, Outcome, Skip};
use postio_index::index;
use postio_model::{AttachmentId, Message, MessageId};
use postio_session::{Extractor, Via};
use postio_storage::repository::MessageRepository;
use postio_storage::sql::{self, RowExt as _};
use postio_storage::{BlobStore, Checkout, test_support};

/// The helper built from this source.
const HELPER: &str = env!("CARGO_BIN_EXE_postio-extract-helper");

fn seed_file(name: &str) -> postio_demo::SearchFile {
    postio_demo::search_files()
        .into_iter()
        .find(|file| file.name == name)
        .expect("the seed has it")
}

#[test]
fn a_test_binary_finds_the_helper_cargo_built_beside_it() {
    // A test binary runs from `target/<profile>/deps/`; the helper is built
    // one level up, where the application's executable would be.
    if std::env::var_os(postio_session::HELPER_ENV).is_none() {
        assert_eq!(
            std::fs::canonicalize(Extractor::locate().helper()).expect("found"),
            std::fs::canonicalize(HELPER).expect("built")
        );
    }
}

#[test]
fn the_helper_answers_every_seed_file_as_an_in_process_extraction_does() {
    let extractor = Extractor::with_helper(HELPER);
    for file in postio_demo::search_files() {
        let isolated = extractor.extract(file.bytes.clone(), file.mime, Some(file.name));
        assert_eq!(isolated.via, Via::Helper, "{}", file.name);
        assert_eq!(
            isolated.extracted,
            postio_extract::extract(&file.bytes, file.mime, Some(file.name), &Limits::default()),
            "{}",
            file.name
        );
    }
}

/// A stand-in helper: `body` as a shell script, executable, in `dir`.
#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nPATH=/usr/bin:/bin\n{body}\n")).expect("written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("executable");
    path
}

#[cfg(unix)]
#[test]
fn a_helper_that_dies_or_answers_garbage_is_a_failure() {
    let dir = tempfile::tempdir().expect("scratch");
    let file = seed_file("atlas-budget-notes.txt");
    for (name, body) in [
        ("abort", "cat >/dev/null\nkill -ABRT $$"),
        ("exit", "cat >/dev/null\nexit 1"),
        ("garbage", "cat >/dev/null\necho 'not a reply'"),
        ("silent", "cat >/dev/null"),
    ] {
        let extractor = Extractor::with_helper(script(dir.path(), name, body));
        let isolated = extractor.extract(file.bytes.clone(), file.mime, Some(file.name));
        assert_eq!(isolated.extracted.outcome, Outcome::Failed, "{name}");
        assert!(isolated.extracted.units.is_empty(), "{name}");
        assert!(
            matches!(isolated.via, Via::Crashed | Via::Garbled),
            "{name}: {:?}",
            isolated.via
        );
    }
}

#[cfg(unix)]
#[test]
fn a_helper_that_never_answers_is_killed_at_its_deadline() {
    let dir = tempfile::tempdir().expect("scratch");
    let limits = Limits {
        max_time: Duration::from_millis(200),
        ..Limits::default()
    };
    // One that never reads its input, so the writer is stuck too, and one
    // that reads it and then spins.
    for (name, body) in [
        ("deaf", "while :; do :; done"),
        ("spin", "cat >/dev/null\nwhile :; do :; done"),
    ] {
        let extractor = Extractor::new(script(dir.path(), name, body), limits.clone());
        let file = seed_file("Atlas-Sep-actuals.pdf");
        let started = Instant::now();
        let isolated = extractor.extract(file.bytes.clone(), file.mime, Some(file.name));
        let elapsed = started.elapsed();
        assert_eq!(isolated.via, Via::Killed, "{name}");
        assert_eq!(
            isolated.extracted.outcome,
            Outcome::Truncated(postio_extract::Limit::Time),
            "{name}"
        );
        assert!(
            elapsed >= limits.max_time,
            "{name} was given up on early: {elapsed:?}"
        );
        assert!(
            elapsed < limits.max_time + postio_session::HELPER_GRACE + Duration::from_secs(2),
            "{name} outlived its deadline: {elapsed:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn with_no_usable_helper_a_pdf_is_never_read_here_and_text_still_is() {
    let dir = tempfile::tempdir().expect("scratch");
    // A reply from an extractor this build is not: wire 1, extractor
    // u32::MAX, complete, no units.
    let stale = script(
        dir.path(),
        "stale",
        r"cat >/dev/null
printf 'PXRS\001\000\377\377\377\377\000\000\000\000\000\000'",
    );
    let pdf = seed_file("Atlas-Sep-actuals.pdf");
    let text = seed_file("atlas-budget-notes.txt");
    for extractor in [
        Extractor::with_helper(dir.path().join("postio-extract-helper")),
        Extractor::with_helper(stale),
    ] {
        let refused = extractor.extract(pdf.bytes.clone(), pdf.mime, Some(pdf.name));
        assert_eq!(refused.via, Via::Refused);
        assert_eq!(
            refused.extracted.outcome,
            Outcome::Skipped(Skip::Unavailable)
        );
        assert!(refused.extracted.units.is_empty());

        let read = extractor.extract(text.bytes.clone(), text.mime, Some(text.name));
        assert_eq!(read.via, Via::InProcess);
        assert_eq!(
            read.extracted,
            postio_extract::extract(&text.bytes, text.mime, Some(text.name), &Limits::default())
        );
    }
}

/// Every attachment row with this file name.
async fn attachment_named(connection: &Checkout, name: &str) -> AttachmentId {
    sql::first(
        connection,
        "SELECT id FROM attachments WHERE filename = ?1",
        [name],
        |row| Ok(AttachmentId::new(row.col(0)?)),
    )
    .await
    .expect("attachments by name")
    .expect("one")
}

async fn outcome_of(connection: &Checkout, attachment: AttachmentId) -> Option<String> {
    sql::first(
        connection,
        "SELECT e.outcome
           FROM attachment_extraction e
           JOIN messages m ON m.content_id = e.content_id
           JOIN attachments a ON a.message_id = m.id AND a.position = e.position
          WHERE a.id = ?1",
        [attachment.get()],
        |row| row.col(0),
    )
    .await
    .expect("its record")
}

/// One message with one stored attachment.
async fn stored(
    connection: &Checkout,
    blobs: &BlobStore,
    account: postio_model::AccountId,
    inbox: postio_model::MailboxId,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> AttachmentId {
    let mut message = Message::new(account, inbox, chrono::Utc::now());
    message.subject = Some("Quarterly figures".to_owned());
    let mut attachment = postio_model::Attachment::new(MessageId::UNASSIGNED, mime, 100);
    attachment.filename = Some(name.to_owned());
    attachment.part_id = Some("2".to_owned());
    attachment.blob_id = Some(blobs.put(bytes).expect("stored"));
    message.attachments.push(attachment);
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    attachment_named(connection, name).await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_pass_kills_the_helper_that_hangs_and_indexes_the_next_attachment() {
    use postio_test_support::logs::Captured;

    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let hangs = stored(
        &connection,
        &blobs,
        account.id,
        inbox,
        "hangs-forever.pdf",
        "application/pdf",
        &seed_file("Atlas-Sep-actuals.pdf").bytes,
    )
    .await;
    let after = stored(
        &connection,
        &blobs,
        account.id,
        inbox,
        "read-after.txt",
        "text/plain",
        b"Harbor survey notes for the Atlas review\n",
    )
    .await;
    drop(connection);

    // The real helper, except for the file that sends it into a loop.
    let dir = tempfile::tempdir().expect("scratch");
    let helper = script(
        dir.path(),
        "picky",
        &format!(
            "t=$(mktemp)\ncat >\"$t\"\n\
             if grep -q hangs-forever \"$t\"; then rm -f \"$t\"; while :; do :; done; fi\n\
             '{HELPER}' <\"$t\"; s=$?\nrm -f \"$t\"\nexit $s"
        ),
    );
    let extractor = Extractor::new(
        helper,
        Limits {
            max_time: Duration::from_millis(300),
            ..Limits::default()
        },
    );

    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let indexed = {
        let _guard = tracing::subscriber::set_default(subscriber);
        postio_session::index_local_attachments_with(&database, &blobs, &extractor)
            .await
            .expect("the pass runs")
    };
    assert_eq!(indexed, 2, "both recorded");

    let connection = database.connect().await.expect("checkout");
    assert_eq!(
        outcome_of(&connection, hangs).await.as_deref(),
        Some("truncated"),
        "killed at its deadline"
    );
    assert_eq!(
        outcome_of(&connection, after).await.as_deref(),
        Some("complete")
    );
    let text = index::attachment_text(&connection, after)
        .await
        .expect("its text");
    assert!(
        text.iter().any(|(_, said)| said.contains("Harbor survey")),
        "{text:?}"
    );

    // What the log says: which attachment, how it ended and by what path;
    // never a name or a word of the file.
    let log = captured.text();
    assert!(
        log.contains(&format!("attachment={}", hangs.get())),
        "{log}"
    );
    assert!(
        log.contains("via=\"killed\"") || log.contains("via=killed"),
        "{log}"
    );
    assert!(
        log.contains("outcome=\"truncated\"") || log.contains("outcome=truncated"),
        "{log}"
    );
    for secret in ["hangs-forever", "read-after", "Harbor", "Atlas", "survey"] {
        assert!(!log.contains(secret), "{secret:?} reached the log: {log}");
    }
}

/// A PDF met while the helper was missing is recorded so the pass ends --
/// and is read once the helper is there, on the next pass, rather than
/// waiting for an extractor version that happens to move.
#[tokio::test(flavor = "multi_thread")]
async fn a_pdf_skipped_for_want_of_the_helper_is_read_once_the_helper_is_there() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let pdf = stored(
        &connection,
        &blobs,
        account.id,
        inbox,
        "Atlas-Sep-actuals.pdf",
        "application/pdf",
        &seed_file("Atlas-Sep-actuals.pdf").bytes,
    )
    .await;
    drop(connection);

    let dir = tempfile::tempdir().expect("scratch");
    let missing = Extractor::with_helper(dir.path().join("postio-extract-helper"));
    let indexed = postio_session::index_local_attachments_with(&database, &blobs, &missing)
        .await
        .expect("the pass runs");
    assert_eq!(indexed, 1, "recorded, so the pass ends");
    let connection = database.connect().await.expect("checkout");
    assert!(outcome_of(&connection, pdf).await.is_some());
    assert!(
        index::attachment_text(&connection, pdf)
            .await
            .expect("its text")
            .is_empty(),
        "not read in this process"
    );
    drop(connection);

    // Still no helper: nothing to do, and nothing tried again.
    let again = postio_session::index_local_attachments_with(&database, &blobs, &missing)
        .await
        .expect("the pass runs");
    assert_eq!(again, 0, "no helper, no retry");

    // The helper arrives -- an install, a build -- and the next pass reads it.
    let helper = Extractor::with_helper(HELPER);
    let read = postio_session::index_local_attachments_with(&database, &blobs, &helper)
        .await
        .expect("the pass runs");
    assert_eq!(read, 1, "read now");
    let connection = database.connect().await.expect("checkout");
    assert_eq!(
        outcome_of(&connection, pdf).await.as_deref(),
        Some("complete")
    );
    assert!(
        !index::attachment_text(&connection, pdf)
            .await
            .expect("its text")
            .is_empty(),
        "the helper read its pages"
    );
}
