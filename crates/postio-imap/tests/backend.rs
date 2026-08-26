//! The [`MailBackend`] seam and the mock every crate above it tests against.
//!
//! Nothing here touches a socket. That is the point: this is the surface
//! `postio-sync` is written against, so the whole sync engine can be developed
//! and tested with no server at all.

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use postio_imap::backend::{
    AppendMessage, BackendError, BodyPart, Capabilities, Capability, Disposition, Envelope, Fault,
    FetchedMessage, FlagChange, MailBackend, MailboxFilter, MailboxStatus, MailboxSummary,
    MockBackend, MockMailbox, PartNode, SelectMode, UidSet, VecSink,
};
use postio_imap::cancel::CancelToken;
use postio_model::{
    AccountId, BodyState, EmailAddress, Flag, FlagSet, MailboxId, MailboxRole, MessageId, ModSeq,
    RfcMessageId, Uid, UidValidity,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A capability list of the shape a mainstream provider advertises *after*
/// login, unknown extension included.
const EXTENSION_RICH: [&str; 8] = [
    "IMAP4rev1",
    "CONDSTORE",
    "ENABLE",
    "QRESYNC",
    "IDLE",
    "UIDPLUS",
    "MOVE",
    "X-VENDOR-PUSH",
];

fn uids(values: impl IntoIterator<Item = u32>) -> UidSet {
    values.into_iter().map(Uid::new).collect()
}

/// A backend shaped like the account this project targets.
fn server() -> MockBackend {
    MockBackend::builder()
        .capabilities(EXTENSION_RICH)
        .mailbox(
            MockMailbox::new("INBOX")
                .uid_validity(UidValidity::new(4_242))
                .highest_mod_seq(ModSeq::new(900))
                .corpus(["plain-text-simple", "attachment-pdf", "html-newsletter"]),
        )
        .mailbox(MockMailbox::new("Sent Messages"))
        .mailbox(MockMailbox::new("Deleted Messages"))
        .mailbox(MockMailbox::new("Archive"))
        .mailbox(MockMailbox::new("Projects/Postio").delimiter('/'))
        .build()
}

async fn connected() -> MockBackend {
    let backend = server();
    backend.connect().await.expect("connect");
    backend
}

// ---------------------------------------------------------------------------
// UidSet
// ---------------------------------------------------------------------------

#[test]
fn a_uid_set_coalesces_and_orders_what_it_is_given() {
    let set = uids([3, 1, 2, 7, 2]);

    assert_eq!(set.to_sequence_set(), "1:3,7");
    assert_eq!(set.len(), Some(4));
    assert!(set.contains(Uid::new(2)));
    assert!(!set.contains(Uid::new(4)));
}

#[test]
fn a_uid_set_ignores_uid_zero() {
    // IMAP UIDs start at 1; a zero is a bug upstream, not a message.
    let set = uids([0, 1, 2]);

    assert_eq!(set.to_sequence_set(), "1:2");
    assert!(!set.contains(Uid::new(0)));
}

#[test]
fn an_empty_uid_set_renders_as_nothing() {
    let set = UidSet::new();

    assert!(set.is_empty());
    assert_eq!(set.to_sequence_set(), "");
    assert_eq!(set.len(), Some(0));
}

#[test]
fn an_open_ended_uid_set_renders_as_a_star() {
    let set = UidSet::from_uid_onwards(Uid::new(10));

    assert_eq!(set.to_sequence_set(), "10:*");
    assert!(set.is_open_ended());
    assert_eq!(set.len(), None);
    assert!(set.contains(Uid::new(u32::MAX)));

    assert_eq!(UidSet::all().to_sequence_set(), "1:*");
}

#[test]
fn a_uid_set_chunks_without_losing_or_reordering_uids() {
    let set: UidSet = (1..=10_000).map(Uid::new).collect();
    let chunks = set.chunks(500);

    assert_eq!(chunks.len(), 20);
    assert!(chunks.iter().all(|chunk| chunk.len() == Some(500)));

    let round_tripped: Vec<u32> = chunks
        .iter()
        .flat_map(|chunk| chunk.uids())
        .map(Uid::get)
        .collect();
    assert_eq!(round_tripped, (1..=10_000).collect::<Vec<_>>());
}

#[test]
fn chunking_splits_ranges_rather_than_only_commas() {
    let set = UidSet::range(Uid::new(1), Uid::new(10));
    let chunks = set.chunks(4);

    let rendered: Vec<String> = chunks.iter().map(UidSet::to_sequence_set).collect();
    assert_eq!(rendered, ["1:4", "5:8", "9:10"]);
}

#[test]
fn an_open_ended_uid_set_cannot_be_chunked() {
    // `10:*` has no known size, so there is nothing to split it on. One
    // chunk, unchanged, is the only honest answer.
    let chunks = UidSet::from_uid_onwards(Uid::new(10)).chunks(4);

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].to_sequence_set(), "10:*");
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

#[test]
fn capability_names_are_matched_case_insensitively() {
    let capabilities = Capabilities::from_names(["imap4rev1", "CondStore", "qresync"]);

    assert!(capabilities.contains(Capability::CondStore));
    assert!(capabilities.contains(Capability::QResync));
    assert!(capabilities.has_name("QRESYNC"));
}

#[test]
fn an_unrecognized_capability_is_kept_verbatim_not_dropped() {
    let capabilities = Capabilities::from_names(EXTENSION_RICH);

    assert!(capabilities.has_name("X-VENDOR-PUSH"));
    assert!(capabilities.has_name("x-vendor-push"));
    assert!(capabilities.names().contains(&"X-VENDOR-PUSH".to_owned()));
}

#[test]
fn requiring_an_absent_capability_names_it() {
    let capabilities = Capabilities::from_names(["IMAP4rev1", "IDLE"]);

    let error = capabilities.require(Capability::QResync).unwrap_err();

    assert!(matches!(error, BackendError::Unsupported { .. }));
    assert!(error.to_string().contains("QRESYNC"));
}

#[test]
fn incremental_sync_needs_both_condstore_and_qresync() {
    assert!(Capabilities::from_names(EXTENSION_RICH).supports_incremental_sync());
    assert!(!Capabilities::from_names(["IMAP4rev1", "CONDSTORE"]).supports_incremental_sync());
    assert!(!Capabilities::from_names(["IMAP4rev1", "QRESYNC"]).supports_incremental_sync());
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn a_transient_failure_is_distinguishable_from_a_permanent_one() {
    let dropped = BackendError::Disconnected {
        context: "INBOX".into(),
        reason: "connection reset by peer".into(),
    };
    let refused = BackendError::Auth {
        account: "someone@example.com".into(),
        reason: "invalid credentials".into(),
    };

    assert!(dropped.is_transient());
    assert!(!refused.is_transient());
    assert!(refused.is_authentication_failure());
    assert!(!dropped.is_authentication_failure());
}

#[test]
fn a_uidvalidity_change_demands_a_full_resync() {
    let error = BackendError::UidValidityChanged {
        mailbox: "INBOX".into(),
        known: UidValidity::new(1),
        observed: UidValidity::new(2),
    };

    assert!(error.requires_full_resync());
    assert!(!error.is_transient());
    assert!(!BackendError::Cancelled.requires_full_resync());
}

// ---------------------------------------------------------------------------
// The seam itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_backend_is_usable_behind_a_trait_object() {
    // postio-sync holds one of these and knows nothing about IMAP.
    let backend: Arc<dyn MailBackend> = Arc::new(server());

    let capabilities = backend.connect().await.unwrap();

    assert_eq!(backend.describe(), "mock");
    assert!(capabilities.contains(Capability::Idle));
}

/// The seam's sources, baked into this binary at compile time.
///
/// `include_str!` rather than a runtime `read_dir` (#225): a filesystem walk
/// at test time raced this machine's normal condition of several sessions
/// building at once — and worse, a binary served stale out of a shared
/// target directory carried a `CARGO_MANIFEST_DIR` baked in a worktree that
/// no longer existed, so the path was gone *for that binary, permanently*
/// (#178). Baked contents cannot race and cannot dangle: the text checked is
/// by construction the text this binary was compiled from.
///
/// A new module cannot dodge the list silently:
/// [`no_io_imap_type_reaches_the_seam`] cross-checks it against `mod.rs`'s
/// own `mod` declarations, which a new file must appear in to be compiled at
/// all.
const BACKEND_SOURCES: &[(&str, &str)] = &[
    (
        "capability.rs",
        include_str!("../src/backend/capability.rs"),
    ),
    ("error.rs", include_str!("../src/backend/error.rs")),
    ("message.rs", include_str!("../src/backend/message.rs")),
    ("mock.rs", include_str!("../src/backend/mock.rs")),
    ("mod.rs", include_str!("../src/backend/mod.rs")),
    ("sink.rs", include_str!("../src/backend/sink.rs")),
    ("uid_set.rs", include_str!("../src/backend/uid_set.rs")),
];

#[test]
fn no_io_imap_type_reaches_the_seam() {
    // ADR 0001 rule 7. io-imap is pre-1.0 and reshuffles its public API every
    // fortnight; the whole point of this module is that the churn stops here.
    for (name, source) in BACKEND_SOURCES {
        for (number, line) in source.lines().enumerate() {
            // Prose may name the crate; code may not. This module's own
            // documentation explains the rule, and would otherwise trip it.
            if line.trim_start().starts_with("//") {
                continue;
            }
            assert!(
                !line.contains("io_imap") && !line.contains("imap_types"),
                "src/backend/{name}:{} names a protocol crate; translate at \
                 the edge instead",
                number + 1
            );
        }
    }

    // Completeness, without touching the filesystem: every module mod.rs
    // declares must be in the baked list, so a file added to the seam cannot
    // silently escape the scan. (One it stops declaring stops compiling, and
    // a listed file that is deleted fails the include_str! at build time.)
    let (_, mod_rs) = BACKEND_SOURCES
        .iter()
        .find(|(name, _)| *name == "mod.rs")
        .expect("mod.rs is in the list");
    for line in mod_rs.lines() {
        let line = line.trim_start();
        let Some(declared) = line
            .strip_prefix("pub mod ")
            .or_else(|| line.strip_prefix("mod "))
        else {
            continue;
        };
        let file = format!("{}.rs", declared.trim_end_matches(';').trim());
        assert!(
            BACKEND_SOURCES.iter().any(|(name, _)| *name == file),
            "src/backend/{file} is compiled into the seam but missing from \
             BACKEND_SOURCES, so nothing scans it -- add it to the list"
        );
    }
}

// ---------------------------------------------------------------------------
// Mock: connection and capabilities
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capabilities_are_only_available_once_connected() {
    let backend = server();

    let error = backend
        .list_mailboxes(&MailboxFilter::all())
        .await
        .unwrap_err();
    assert!(matches!(error, BackendError::NotConnected { .. }));

    backend.connect().await.unwrap();
    assert!(backend.list_mailboxes(&MailboxFilter::all()).await.is_ok());

    backend.disconnect().await.unwrap();
    assert!(matches!(
        backend.capabilities().await,
        Err(BackendError::NotConnected { .. })
    ));
}

#[tokio::test]
async fn an_empty_post_auth_capability_list_is_an_error_not_a_downgrade() {
    // ADR 0001 rule 2. io-imap will hand back an empty vec without erroring if
    // the auth coroutine is driven with `ensure_capabilities: false`; every
    // gate downstream would then silently fall back to full resync forever.
    let backend = MockBackend::builder()
        .capabilities(Vec::<String>::new())
        .mailbox(MockMailbox::new("INBOX"))
        .build();

    let error = backend.connect().await.unwrap_err();

    assert!(matches!(error, BackendError::EmptyCapabilities { .. }));
    assert!(error.to_string().contains("capabilit"));
}

// ---------------------------------------------------------------------------
// Mock: mailboxes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listing_mailboxes_reports_paths_delimiters_and_roles() {
    let backend = connected().await;

    let mailboxes = backend.list_mailboxes(&MailboxFilter::all()).await.unwrap();
    let by_path = |path: &str| -> MailboxSummary {
        mailboxes
            .iter()
            .find(|mailbox| mailbox.path == path)
            .unwrap_or_else(|| panic!("{path} was not listed"))
            .clone()
    };

    assert_eq!(by_path("INBOX").role, MailboxRole::Inbox);
    assert_eq!(by_path("Sent Messages").role, MailboxRole::Sent);
    assert_eq!(by_path("Deleted Messages").role, MailboxRole::Trash);
    assert_eq!(by_path("Projects/Postio").role, MailboxRole::Regular);
    assert_eq!(by_path("Projects/Postio").name(), "Postio");
}

#[tokio::test]
async fn a_mailbox_summary_becomes_a_domain_mailbox() {
    let backend = connected().await;
    let mailboxes = backend.list_mailboxes(&MailboxFilter::all()).await.unwrap();

    let summary = mailboxes
        .iter()
        .find(|mailbox| mailbox.path == "Projects/Postio")
        .unwrap()
        .clone();
    let mailbox = summary.into_mailbox(AccountId::new(7));

    assert_eq!(mailbox.account_id, AccountId::new(7));
    assert_eq!(mailbox.path, "Projects/Postio");
    assert_eq!(mailbox.name, "Postio");
    assert_eq!(mailbox.delimiter, Some('/'));
}

#[tokio::test]
async fn selecting_reports_uid_validity_uid_next_and_the_mod_sequence() {
    let backend = connected().await;

    let status = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .unwrap();

    assert_eq!(status.uid_validity, UidValidity::new(4_242));
    assert_eq!(status.exists, 3);
    assert_eq!(status.uid_next, Uid::new(4));
    assert_eq!(status.highest_mod_seq, Some(ModSeq::new(900)));
    assert!(!status.read_only);

    let read_only = backend.select("INBOX", SelectMode::ReadOnly).await.unwrap();
    assert!(read_only.read_only);
}

#[tokio::test]
async fn status_reads_a_mailbox_without_selecting_it() {
    let backend = connected().await;

    let status: MailboxStatus = backend.status("Archive").await.unwrap();

    assert_eq!(status.path, "Archive");
    assert_eq!(status.exists, 0);
}

#[tokio::test]
async fn an_unknown_mailbox_is_reported_by_name() {
    let backend = connected().await;

    let error = backend.status("Nowhere").await.unwrap_err();

    assert!(matches!(error, BackendError::NoSuchMailbox { .. }));
    assert!(error.to_string().contains("Nowhere"));
}

// ---------------------------------------------------------------------------
// Mock: fetching
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fetching_headers_returns_only_the_uids_that_were_asked_for() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let fetched = backend
        .fetch_headers("INBOX", &uids([1, 3, 99]), None, &cancel)
        .await
        .unwrap();

    let returned: Vec<u32> = fetched.iter().map(|message| message.uid.get()).collect();
    assert_eq!(returned, [1, 3]);
    assert!(fetched.iter().all(|message| message.size > 0));
    assert!(fetched.iter().all(|message| message.envelope.is_some()));
}

#[tokio::test]
async fn fetching_headers_honours_changedsince() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    backend
        .store_flags(
            "INBOX",
            &uids([2]),
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .unwrap();

    let changed = backend
        .fetch_headers("INBOX", &UidSet::all(), Some(ModSeq::new(900)), &cancel)
        .await
        .unwrap();

    let returned: Vec<u32> = changed.iter().map(|message| message.uid.get()).collect();
    assert_eq!(returned, [2]);
    assert!(changed[0].mod_seq > Some(ModSeq::new(900)));
}

#[tokio::test]
async fn a_cancelled_fetch_stops_rather_than_returning_a_partial_answer() {
    let backend = connected().await;
    let cancel = CancelToken::new();
    cancel.cancel();

    let error = backend
        .fetch_headers("INBOX", &UidSet::all(), None, &cancel)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::Cancelled));
}

#[tokio::test]
async fn fetching_a_body_streams_into_the_sink_rather_than_returning_bytes() {
    let backend = connected().await;
    let cancel = CancelToken::new();
    let mut sink = VecSink::new();

    let fetched = backend
        .fetch_body("INBOX", Uid::new(1), &mut sink, &cancel)
        .await
        .unwrap();

    assert!(sink.is_finished());
    assert_eq!(fetched.bytes_written, sink.len() as u64);
    assert!(sink.into_inner().starts_with(b"Return-Path:"));
}

#[tokio::test]
async fn a_large_body_arrives_in_chunks_so_nothing_buffers_it_whole() {
    let backend = MockBackend::builder()
        .capabilities(EXTENSION_RICH)
        .mailbox(MockMailbox::new("INBOX").corpus(["attachment-large"]))
        .chunk_size(8 * 1024)
        .build();
    backend.connect().await.unwrap();

    let cancel = CancelToken::new();
    let mut sink = VecSink::new();
    backend
        .fetch_body("INBOX", Uid::new(1), &mut sink, &cancel)
        .await
        .unwrap();

    assert!(
        sink.chunks() > 4,
        "a 256 KiB message arrived in {} chunk(s)",
        sink.chunks()
    );
}

#[tokio::test]
async fn fetching_an_absent_uid_is_a_clean_error() {
    let backend = connected().await;
    let cancel = CancelToken::new();
    let mut sink = VecSink::new();

    let error = backend
        .fetch_body("INBOX", Uid::new(404), &mut sink, &cancel)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::NoSuchMessage { .. }));
    assert!(sink.into_inner().is_empty());
}

#[tokio::test]
async fn a_part_fetch_asks_for_a_section_not_the_whole_message() {
    let backend = connected().await;
    let cancel = CancelToken::new();
    let mut whole = VecSink::new();
    let mut headers = VecSink::new();

    backend
        .fetch_part("INBOX", Uid::new(1), &BodyPart::Whole, &mut whole, &cancel)
        .await
        .unwrap();
    backend
        .fetch_part(
            "INBOX",
            Uid::new(1),
            &BodyPart::Headers,
            &mut headers,
            &cancel,
        )
        .await
        .unwrap();

    let headers = headers.into_inner();
    assert!(!headers.is_empty());
    assert!(headers.len() < whole.into_inner().len());
}

#[test]
fn a_body_part_renders_the_section_imap_expects() {
    assert_eq!(BodyPart::Whole.section_spec(), "");
    assert_eq!(BodyPart::Headers.section_spec(), "HEADER");
    assert_eq!(BodyPart::Text.section_spec(), "TEXT");
    assert_eq!(BodyPart::section("2.1").section_spec(), "2.1");
}

// ---------------------------------------------------------------------------
// Mock: mutations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn storing_flags_adds_removes_and_replaces() {
    let backend = connected().await;
    let target = uids([1]);

    let added = backend
        .store_flags(
            "INBOX",
            &target,
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen, Flag::Flagged])),
        )
        .await
        .unwrap();
    assert!(added[0].flags.is_seen() && added[0].flags.is_flagged());

    let removed = backend
        .store_flags(
            "INBOX",
            &target,
            &FlagChange::Remove(FlagSet::from_iter([Flag::Flagged])),
        )
        .await
        .unwrap();
    assert!(removed[0].flags.is_seen() && !removed[0].flags.is_flagged());

    let replaced = backend
        .store_flags(
            "INBOX",
            &target,
            &FlagChange::Replace(FlagSet::from_iter([Flag::Answered])),
        )
        .await
        .unwrap();
    assert!(replaced[0].flags.is_answered() && !replaced[0].flags.is_seen());
}

#[tokio::test]
async fn storing_flags_advances_the_mod_sequence() {
    let backend = connected().await;

    let before = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .unwrap();
    let update = backend
        .store_flags(
            "INBOX",
            &uids([1]),
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .unwrap();
    let after = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .unwrap();

    assert!(after.highest_mod_seq > before.highest_mod_seq);
    assert_eq!(update[0].mod_seq, after.highest_mod_seq);
}

#[tokio::test]
async fn moving_leaves_the_source_and_reports_the_new_uids() {
    let backend = connected().await;

    let mapping = backend
        .move_messages("INBOX", &uids([1, 2]), "Archive")
        .await
        .unwrap();

    assert_eq!(mapping.len(), 2);
    assert_eq!(
        backend
            .select("INBOX", SelectMode::ReadWrite)
            .await
            .unwrap()
            .exists,
        1
    );
    assert_eq!(backend.status("Archive").await.unwrap().exists, 2);
    assert!(mapping.iter().all(|entry| entry.destination.get() > 0));
}

#[tokio::test]
async fn copying_leaves_the_source_intact() {
    let backend = connected().await;

    backend
        .copy_messages("INBOX", &uids([1]), "Archive")
        .await
        .unwrap();

    assert_eq!(backend.status("INBOX").await.unwrap().exists, 3);
    assert_eq!(backend.status("Archive").await.unwrap().exists, 1);
}

#[tokio::test]
async fn expunge_removes_only_messages_marked_deleted() {
    let backend = connected().await;

    backend
        .store_flags(
            "INBOX",
            &uids([2]),
            &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .unwrap();

    let expunged = backend.expunge("INBOX", None).await.unwrap();

    assert_eq!(expunged, [Uid::new(2)]);
    assert_eq!(backend.status("INBOX").await.unwrap().exists, 2);
}

#[tokio::test]
async fn append_assigns_the_next_uid_and_keeps_the_flags() {
    let backend = connected().await;
    let before = backend.status("Archive").await.unwrap();

    let message = AppendMessage::new(b"Subject: hello\r\n\r\nbody\r\n".to_vec())
        .with_flags(FlagSet::from_iter([Flag::Seen]))
        .with_internal_date(Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap());
    let mapping = backend.append("Archive", &message).await.unwrap();

    let mapping = mapping.expect("UIDPLUS is advertised, so APPENDUID is reported");
    assert_eq!(mapping.destination, before.uid_next);
    assert_eq!(backend.status("Archive").await.unwrap().exists, 1);

    let cancel = CancelToken::new();
    let fetched = backend
        .fetch_headers("Archive", &UidSet::all(), None, &cancel)
        .await
        .unwrap();
    assert!(fetched[0].flags.is_seen());
}

#[tokio::test]
async fn an_appended_message_is_visible_to_a_changedsince_fetch() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let before = backend.status("Archive").await.unwrap();
    let floor = before.highest_mod_seq.expect("CONDSTORE is advertised");

    backend
        .append(
            "Archive",
            &AppendMessage::new(b"Subject: new mail\r\n\r\nbody\r\n".to_vec()),
        )
        .await
        .unwrap();

    // RFC 7162 §3.1.2.1: an appended message's MODSEQ must *exceed* the
    // HIGHESTMODSEQ that preceded it, or the strictly-greater-than filter of
    // CHANGEDSINCE never reports the arrival.
    let changed = backend
        .fetch_headers("Archive", &UidSet::all(), Some(floor), &cancel)
        .await
        .unwrap();

    let returned: Vec<u32> = changed.iter().map(|message| message.uid.get()).collect();
    assert_eq!(returned, [1]);
    assert!(changed[0].mod_seq > Some(floor));
    assert!(backend.status("Archive").await.unwrap().highest_mod_seq > Some(floor));
}

#[tokio::test]
async fn a_moved_message_is_visible_to_a_changedsince_fetch() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let floor = backend
        .status("Archive")
        .await
        .unwrap()
        .highest_mod_seq
        .expect("CONDSTORE is advertised");

    backend
        .move_messages("INBOX", &uids([1]), "Archive")
        .await
        .unwrap();

    let changed = backend
        .fetch_headers("Archive", &UidSet::all(), Some(floor), &cancel)
        .await
        .unwrap();

    assert_eq!(changed.len(), 1);
    assert!(changed[0].mod_seq > Some(floor));
}

#[tokio::test]
async fn a_copied_message_is_visible_to_a_changedsince_fetch() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let floor = backend
        .status("Archive")
        .await
        .unwrap()
        .highest_mod_seq
        .expect("CONDSTORE is advertised");

    backend
        .copy_messages("INBOX", &uids([1, 2]), "Archive")
        .await
        .unwrap();

    let changed = backend
        .fetch_headers("Archive", &UidSet::all(), Some(floor), &cancel)
        .await
        .unwrap();

    // Each arrival carries its own modification sequence, as a server that
    // assigns them one at a time would.
    assert_eq!(changed.len(), 2);
    assert!(changed[0].mod_seq < changed[1].mod_seq);
    assert!(changed[0].mod_seq > Some(floor));
}

#[tokio::test]
async fn append_reports_no_uid_when_the_server_lacks_uidplus() {
    let backend = MockBackend::builder()
        .capabilities(["IMAP4rev1"])
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    backend.connect().await.unwrap();

    let message = AppendMessage::new(b"Subject: hi\r\n\r\n".to_vec());

    assert!(backend.append("INBOX", &message).await.unwrap().is_none());
}

// ---------------------------------------------------------------------------
// Mock: IDLE
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idle_returns_the_events_the_server_pushed() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let watcher = backend.clone();
    let handle = tokio::spawn(async move {
        watcher
            .idle("INBOX", Duration::from_secs(5), &CancelToken::new())
            .await
    });

    // Something lands in the mailbox while the watcher is parked.
    tokio::time::sleep(Duration::from_millis(10)).await;
    backend
        .append(
            "INBOX",
            &AppendMessage::new(b"Subject: new\r\n\r\nbody\r\n".to_vec()),
        )
        .await
        .unwrap();

    let events = handle.await.unwrap().unwrap();
    assert!(!events.is_empty(), "IDLE returned nothing");
    drop(cancel);
}

#[tokio::test]
async fn idle_returns_empty_when_cancelled_rather_than_erroring() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        stopper.cancel();
    });

    let events = backend
        .idle("INBOX", Duration::from_secs(30), &cancel)
        .await
        .unwrap();

    assert!(events.is_empty());
}

// ---------------------------------------------------------------------------
// Mock: injected failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_mock_can_simulate_a_dropped_connection() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    backend.inject(Fault::Disconnect);

    let error = backend
        .fetch_headers("INBOX", &UidSet::all(), None, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(error, BackendError::Disconnected { .. }));
    assert!(error.is_transient());

    // A dropped connection really is dropped: the next call says so too.
    assert!(matches!(
        backend.status("INBOX").await,
        Err(BackendError::NotConnected { .. })
    ));

    backend.connect().await.unwrap();
    assert!(backend.status("INBOX").await.is_ok());
}

#[tokio::test]
async fn the_mock_can_simulate_a_timeout() {
    let backend = connected().await;

    backend.inject(Fault::Timeout);

    let error = backend.status("INBOX").await.unwrap_err();

    assert!(matches!(error, BackendError::TimedOut { .. }));
    assert!(error.is_transient());
}

#[tokio::test]
async fn the_mock_can_simulate_a_uidvalidity_change() {
    let backend = connected().await;
    let cancel = CancelToken::new();

    let before = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .unwrap();
    backend.change_uid_validity("INBOX", UidValidity::new(9_999));
    let after = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .unwrap();

    assert_ne!(before.uid_validity, after.uid_validity);

    // A fetch that still believes the old generation is refused, rather than
    // being answered with UIDs that mean something else now.
    let error = backend
        .fetch_headers("INBOX", &UidSet::all(), None, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(error, BackendError::UidValidityChanged { .. }));
    assert!(error.requires_full_resync());

    // Once the caller has resynced and accepted the new generation, the
    // mailbox is usable again.
    backend.acknowledge_uid_validity("INBOX");
    assert!(
        backend
            .fetch_headers("INBOX", &UidSet::all(), None, &cancel)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_fault_can_be_scheduled_so_retry_paths_are_testable() {
    let backend = connected().await;

    backend.inject_after(2, Fault::Timeout);

    assert!(backend.status("INBOX").await.is_ok());
    assert!(backend.status("INBOX").await.is_ok());
    assert!(matches!(
        backend.status("INBOX").await,
        Err(BackendError::TimedOut { .. })
    ));
    assert!(backend.status("INBOX").await.is_ok());
}

#[tokio::test]
async fn authentication_failure_is_injectable_at_connect() {
    let backend = server();
    backend.inject(Fault::AuthFailed);

    let error = backend.connect().await.unwrap_err();

    assert!(error.is_authentication_failure());
    assert!(!error.is_transient());
}

#[tokio::test(start_paused = true)]
async fn every_call_can_be_given_a_latency() {
    let backend = connected().await;
    backend.set_latency(Duration::from_millis(250));

    let started = tokio::time::Instant::now();
    backend.status("INBOX").await.unwrap();

    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[tokio::test]
async fn the_mock_counts_the_calls_it_served() {
    let backend = connected().await;

    backend.status("INBOX").await.unwrap();
    backend.status("Archive").await.unwrap();

    assert_eq!(backend.calls(), 3); // connect, then two statuses
}

// ---------------------------------------------------------------------------
// Translation into the domain model
// ---------------------------------------------------------------------------

#[test]
fn a_fetched_message_becomes_a_domain_message() {
    let envelope = Envelope {
        date: Some(Utc.with_ymd_and_hms(2026, 8, 20, 9, 30, 0).unwrap()),
        subject: Some("Re: the plan".to_owned()),
        from: vec![EmailAddress::new(Some("Ada"), "ada@example.com")],
        sender: None,
        reply_to: vec![],
        to: vec![EmailAddress::new(None::<String>, "diego@example.com")],
        cc: vec![],
        bcc: vec![],
        message_id: Some(RfcMessageId::new("child@example.com")),
        in_reply_to: Some(RfcMessageId::new("root@example.com")),
        references: vec![RfcMessageId::new("root@example.com")],
        list_id: Some("harbour-dev.lists.example.org".to_owned()),
    };

    let fetched = FetchedMessage {
        uid: Uid::new(12),
        uid_validity: UidValidity::new(4_242),
        mod_seq: Some(ModSeq::new(901)),
        flags: FlagSet::from_iter([Flag::Seen, Flag::Recent]),
        internal_date: Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap(),
        size: 4_096,
        envelope: Some(envelope),
        structure: None,
    };

    let message = fetched.into_message(AccountId::new(1), MailboxId::new(2));

    assert_eq!(message.account_id, AccountId::new(1));
    assert_eq!(message.mailbox_id, MailboxId::new(2));
    assert_eq!(message.subject.as_deref(), Some("Re: the plan"));
    assert_eq!(message.normalized_subject(), "the plan");
    assert_eq!(
        message.list_id.as_deref(),
        Some("harbour-dev.lists.example.org")
    );
    assert_eq!(message.size, 4_096);
    assert_eq!(message.server.uid, Some(Uid::new(12)));
    assert_eq!(message.server.uid_validity, Some(UidValidity::new(4_242)));
    assert_eq!(message.server.mod_seq, Some(ModSeq::new(901)));
    assert_eq!(message.sync.body_state, BodyState::HeadersOnly);
    assert_eq!(
        message.received_at,
        Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap()
    );
    assert_eq!(
        message.reference_chain().collect::<Vec<_>>(),
        [&RfcMessageId::new("root@example.com")]
    );

    // `\Recent` is a per-session signal and must never be persisted.
    assert!(message.flags.is_seen());
    assert!(!message.flags.contains(&Flag::Recent));
}

#[test]
fn a_fetched_message_with_a_body_structure_carries_its_own_content_type() {
    let structure = postio_imap::backend::BodyStructure::from_parts(
        "multipart/related",
        [PartNode::new("1", "text/html", 512)],
    );
    let fetched = FetchedMessage {
        uid: Uid::new(1),
        uid_validity: UidValidity::new(1),
        mod_seq: None,
        flags: FlagSet::new(),
        internal_date: Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap(),
        size: 100,
        envelope: None,
        structure: Some(structure),
    };

    let message = fetched.into_message(AccountId::new(1), MailboxId::new(2));

    assert_eq!(message.content_type.as_deref(), Some("multipart/related"));
}

#[test]
fn a_fetched_message_with_no_body_structure_has_no_content_type() {
    let fetched = FetchedMessage {
        uid: Uid::new(1),
        uid_validity: UidValidity::new(1),
        mod_seq: None,
        flags: FlagSet::new(),
        internal_date: Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap(),
        size: 100,
        envelope: None,
        structure: None,
    };

    let message = fetched.into_message(AccountId::new(1), MailboxId::new(2));

    assert_eq!(message.content_type, None);
}

#[test]
fn a_body_structure_becomes_attachment_metadata_without_any_bytes() {
    let structure = postio_imap::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            PartNode::new("1", "text/plain", 512),
            PartNode::new("2", "text/html", 2_048),
            PartNode::new("3", "application/pdf", 91_233)
                .with_filename("quarterly.pdf")
                .with_disposition(Disposition::Attachment),
            PartNode::new("4", "image/png", 8_100)
                .with_content_id("<logo@example.com>")
                .with_disposition(Disposition::Inline),
        ],
    );

    assert_eq!(structure.text_part().map(PartNode::section), Some("1"));
    assert_eq!(structure.html_part().map(PartNode::section), Some("2"));

    let attachments = structure.to_attachments(MessageId::new(5));
    let names: Vec<&str> = attachments
        .iter()
        .map(postio_model::Attachment::display_name)
        .collect();

    assert_eq!(names, ["quarterly.pdf", "attachment"]);
    assert!(attachments.iter().all(|part| !part.is_downloaded()));
    assert_eq!(attachments[0].size, 91_233);
    assert_eq!(attachments[0].part_id.as_deref(), Some("3"));
    assert!(attachments[1].is_inline());
}

#[test]
fn a_fetched_message_carries_the_sections_holding_its_own_text() {
    // ADR 0017's text axis fetches `BODY.PEEK[<section>]`, so the section
    // numbers have to survive the header sync -- `into_message` is the only
    // place that sees a `BODYSTRUCTURE` and a `Message` at the same time.
    // The attachment here is what makes the assertion mean something: its
    // section must not be mistaken for a body part.
    let structure = postio_imap::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            PartNode::new("1.1", "text/plain", 512),
            PartNode::new("1.2", "text/html", 2048),
            PartNode::new("2", "application/pdf", 40 * 1024 * 1024).with_filename("statement.pdf"),
        ],
    );
    let fetched = FetchedMessage {
        uid: Uid::new(1),
        uid_validity: UidValidity::new(1),
        mod_seq: None,
        flags: FlagSet::new(),
        internal_date: Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap(),
        size: 40 * 1024 * 1024,
        envelope: None,
        structure: Some(structure),
    };

    let message = fetched.into_message(AccountId::new(1), MailboxId::new(2));

    assert_eq!(message.text_part_id.as_deref(), Some("1.1"));
    assert_eq!(message.html_part_id.as_deref(), Some("1.2"));
    // And the payload stays where it was: metadata only, no bytes.
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].part_id.as_deref(), Some("2"));
}

#[test]
fn a_fetched_message_with_no_body_structure_names_no_text_sections() {
    let fetched = FetchedMessage {
        uid: Uid::new(1),
        uid_validity: UidValidity::new(1),
        mod_seq: None,
        flags: FlagSet::new(),
        internal_date: Utc.with_ymd_and_hms(2026, 8, 20, 9, 31, 0).unwrap(),
        size: 100,
        envelope: None,
        structure: None,
    };

    let message = fetched.into_message(AccountId::new(1), MailboxId::new(2));

    assert_eq!(message.text_part_id, None);
    assert_eq!(message.html_part_id, None);
}
