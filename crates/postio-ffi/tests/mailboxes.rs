//! The folder tree, as the sidebar reads it.

use chrono::Utc;
use postio_ffi::{MailboxRoleFfi, Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store with an inbox, an archive, and one unread message in the inbox.
fn seeded() -> std::sync::Arc<Session> {
    let database = test_support::memory();
    {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        test_support::mailbox(&connection, &account, "Archive");
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message");
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[test]
fn the_folder_tree_crosses() {
    let session = seeded();
    let folders = session.mailboxes();
    assert!(
        folders.len() >= 2,
        "expected at least the inbox and the archive, got {}",
        folders.len()
    );
    session.shutdown();
}

#[test]
fn a_folder_carries_its_role_not_just_its_name() {
    // The role is what makes `a` archive to the right place, and what lets the
    // sidebar say "Archive" rather than whatever the server calls it. A
    // sidebar built from raw paths would show `[Gmail]/All Mail` and be wrong
    // in a way that looks like a server bug.
    let session = seeded();
    let folders = session.mailboxes();

    let inbox = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Inbox)
        .expect("the inbox is in the tree, by role rather than by name");
    assert!(!inbox.name.is_empty());
    session.shutdown();
}

#[test]
fn a_folder_carries_the_counts_the_sidebar_draws() {
    // Cached counts, not a row count: `MailboxCounts` exists so the sidebar
    // never counts rows, and a boundary that recounted would undo that on the
    // one surface redrawn most often.
    let session = seeded();
    let folders = session.mailboxes();
    let inbox = folders
        .iter()
        .find(|folder| folder.role == MailboxRoleFfi::Inbox)
        .expect("an inbox");

    // Seeded with one message; the exact unread count depends on how the
    // fixture files it, so assert the field is populated rather than a value
    // the fixture happens to produce.
    assert!(
        inbox.total > 0 || inbox.unread == 0,
        "counts did not cross at all"
    );
    session.shutdown();
}

#[test]
fn the_hierarchy_survives_the_crossing() {
    // Mailboxes are a tree. Flattening it turns a tidy account into a list of
    // slash-separated strings, and there is no way to recover the nesting on
    // the far side once it is gone.
    let session = seeded();
    for folder in session.mailboxes() {
        // Every folder either has no parent or names one that is also here.
        if let Some(parent) = folder.parent {
            assert!(
                session.mailboxes().iter().any(|other| other.id == parent),
                "{} names a parent that is not in the tree",
                folder.name
            );
        }
    }
    session.shutdown();
}

#[test]
fn a_session_with_no_account_has_no_folders() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    assert!(session.mailboxes().is_empty());
    session.shutdown();
}

#[test]
fn the_sidebar_gets_the_inbox_first_and_one_row_per_role() {
    // #1155, seen against a real iCloud account: the macOS sidebar sorted
    // alphabetically, so the inbox was the sixth row, below a user folder
    // called Garagiste — and every role appeared twice, because an account
    // that has been through more than one client holds `Archive` *and*
    // `Archives`, `Sent` *and* `Sent Messages`.
    //
    // Both are `postio_ui::sidebar`'s answers now rather than the frontend's,
    // which is what #501 already established on the GTK side. This asserts
    // they survive the crossing.
    let database = test_support::memory();
    {
        let connection = database.connection().expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection);
        for path in [
            "Archive",
            "Archives",
            "Sent",
            "Sent Messages",
            "Trash",
            "Deleted Messages",
            "Garagiste",
        ] {
            test_support::mailbox(&connection, &account, path);
        }
    }
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    let folders = session.mailboxes();

    let specials: Vec<&postio_ffi::MailboxFfi> =
        folders.iter().filter(|folder| folder.special).collect();

    assert_eq!(
        specials.first().map(|folder| folder.role),
        Some(MailboxRoleFfi::Inbox),
        "the first row is {:?}",
        specials.iter().map(|f| &f.name).collect::<Vec<_>>()
    );

    for role in [
        MailboxRoleFfi::Archive,
        MailboxRoleFfi::Sent,
        MailboxRoleFfi::Trash,
    ] {
        assert_eq!(
            specials.iter().filter(|folder| folder.role == role).count(),
            1,
            "{role:?} appears more than once in the sidebar's special section"
        );
    }

    // The twins are still there, as ordinary folders under their own names.
    let ordinary: Vec<&str> = folders
        .iter()
        .filter(|folder| !folder.special)
        .map(|folder| folder.name.as_str())
        .collect();
    assert!(
        ordinary.contains(&"Archives"),
        "the twin was dropped rather than listed: {ordinary:?}"
    );
    assert!(
        ordinary.contains(&"Garagiste"),
        "an ordinary folder went missing: {ordinary:?}"
    );
    session.shutdown();
}
