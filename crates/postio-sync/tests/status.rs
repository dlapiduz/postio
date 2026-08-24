//! `StatusTracker` driven by a real sync pass, end to end.
//!
//! Unlike the throttling and state-transition tests in `src/status.rs`
//! (which drive the tracker with hand-built [`Progress`] values on a
//! deterministic clock), this proves the tracker sees the same events a real
//! caller wiring it around [`sync_mailbox`] would.

use postio_imap::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_imap::cancel::CancelToken;
use postio_storage::test_support;
use postio_sync::{StatusTracker, SyncStatus, sync_mailbox_with_batch_size};

const INBOX: &str = "INBOX";

#[tokio::test]
async fn a_full_pass_leaves_the_tracker_idle_with_a_last_sync_time() {
    let mut mailbox = MockMailbox::new(INBOX);
    for n in 1..=6 {
        mailbox = mailbox.message(MockMessage::new(
            format!("From: Ada Lovelace <ada@example.com>\r\nSubject: Note {n}\r\n\r\nBody.\r\n")
                .into_bytes(),
        ));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);

    let mut tracker = StatusTracker::new();
    let mut reported = Vec::new();

    tracker.on_sync_started(inbox.id);
    sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &CancelToken::new(),
        |progress| {
            if let Some(status) = tracker.on_progress(progress, chrono::Utc::now()) {
                reported.push(status);
            }
        },
    )
    .await
    .expect("sync");
    let finished_at = chrono::Utc::now();
    let final_status = tracker.on_sync_finished(inbox.id, finished_at);

    assert!(
        !reported.is_empty(),
        "a multi-batch pass must report at least one progress update"
    );
    assert!(
        matches!(
            reported.last(),
            Some(SyncStatus::Syncing { progress: Some(p), .. }) if p.is_complete()
        ),
        "the last progress update reported must be the completing one"
    );
    assert_eq!(
        final_status,
        SyncStatus::Idle {
            last_sync: Some(finished_at)
        }
    );
    assert_eq!(tracker.last_sync(), Some(finished_at));
}
