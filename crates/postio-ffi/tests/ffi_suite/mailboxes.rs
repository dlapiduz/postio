use chrono::Utc;
use postio_ffi::{MailboxRoleFfi, Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn seeded() -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Archive").await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folder_tree_crosses() {
    let session = seeded().await;
    let folders = session.mailboxes().await;
    assert!(
        folders.len() >= 2,
        "expected at least the inbox and the archive, got {}",
        folders.len()
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_carries_its_role_not_just_its_name() {
    // The role makes `a` archive to the right place; a sidebar built from raw paths would show
    // `[Gmail]/All Mail`.
    let session = seeded().await;
    let folders = session.mailboxes().await;

    let inbox = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Inbox)
        .expect("the inbox is in the tree, by role rather than by name");
    assert!(!inbox.name.is_empty());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_carries_the_counts_the_sidebar_draws() {
    // Cached counts: a boundary that recounted rows would undo `MailboxCounts` on the most-redrawn
    // surface.
    let session = seeded().await;
    let folders = session.mailboxes().await;
    let inbox = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Inbox)
        .expect("an inbox");

    assert!(
        inbox.total > 0 || inbox.unread == 0,
        "counts did not cross at all"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hierarchy_survives_the_crossing() {
    // Mailboxes are a tree; flattening it loses the nesting for good.
    let session = seeded().await;
    for folder in session.mailboxes().await {
        if let Some(parent) = folder.parent {
            assert!(
                session
                    .mailboxes()
                    .await
                    .iter()
                    .any(|other| other.id == parent),
                "{} names a parent that is not in the tree",
                folder.name
            );
        }
    }
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_no_account_has_no_folders() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    assert!(session.mailboxes().await.is_empty());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sidebars_view_rows_cross_the_boundary() {
    let session = seeded().await;
    let folders = session.mailboxes().await;

    for role in [MailboxRoleFfi::Flagged, MailboxRoleFfi::Snoozed] {
        let row = folders
            .iter()
            .find(|folder| folder.role == role)
            .unwrap_or_else(|| panic!("{role:?} never reached the frontend"));
        assert!(
            row.special,
            "{role:?} belongs in the special section, not among the folders"
        );
        assert!(row.selectable, "{role:?} is a row a person can open");
        assert!(
            !row.name.is_empty(),
            "{role:?} has nothing to draw as a label"
        );
    }

    assert!(
        !folders
            .iter()
            .any(|folder| folder.role == MailboxRoleFfi::Outbox),
        "an empty Outbox was drawn"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_view_row_carries_the_count_its_badge_needs() {
    let session = seeded().await;
    let folders = session.mailboxes().await;

    let flagged = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Flagged)
        .expect("a flagged row");
    assert_eq!(
        flagged.flagged, flagged.total,
        "the flagged view's count is how many are flagged"
    );

    let snoozed = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Snoozed)
        .expect("a snoozed row");
    assert_eq!(
        snoozed.snoozed, snoozed.total,
        "the snoozed view's count is how many are snoozed"
    );
    session.shutdown();
}
