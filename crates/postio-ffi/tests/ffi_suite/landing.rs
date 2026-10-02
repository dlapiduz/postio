//! A list with mail in it has a message showing; only a chosen one is read.
//!
//! GTK's `SingleSelection` lands on row 0 the moment the list has rows, so
//! its reading pane is never blank beside a full list (#70, #601). The Mac
//! had no such landing: a folder opened, or a first sync filled the inbox,
//! and nothing was selected until somebody clicked. This is the same rule,
//! here, where the cursor is.
//!
//! **And the landing is not a choice.** #601's half: the pane fills for it,
//! but the read clock (#71) starts only for a row a person put the cursor
//! on. Otherwise every launch would mark the newest message read for no
//! better reason than that Postio was opened.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions, UiEvent};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store with `count` messages in an inbox, opened and paged in.
async fn listed(count: u32) -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory().await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        for _ in 0..count {
            let mut message = Message::new(account.id, inbox, Utc::now());
            repository.create(&mut message).await.expect("a message");
        }
        inbox.get()
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    session.open_scope(ScopeFfi::Mailbox { mailbox });
    let _ = session.row_at(0);
    session.settle_for_test();
    (session, mailbox)
}

/// Every cursor report waiting, oldest first, as `(row, message, chosen)`.
fn cursor_reports(session: &Session) -> Vec<(Option<u32>, Option<i64>, bool)> {
    let mut reports = Vec::new();
    while let Some(event) = session.try_next_event() {
        if let UiEvent::CursorMoved {
            row,
            message,
            chosen,
        } = event
        {
            reports.push((row, message, chosen));
        }
    }
    reports
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_with_mail_and_no_cursor_lands_on_the_first_row() {
    let (session, _) = listed(3).await;
    assert_eq!(
        session.cursor_row(),
        None,
        "opening a folder places nothing"
    );
    let _ = cursor_reports(&session);

    session.settle_cursor();

    assert_eq!(session.cursor_row(), Some(0));
    let first = session.row_at(0).expect("resident").id;
    assert_eq!(
        cursor_reports(&session),
        [(Some(0), Some(first), false)],
        "the pane is told what to show, and that nobody chose it"
    );
    assert!(!session.cursor_chosen());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_list_lands_nowhere() {
    // A first sync's inbox before the first page: nothing to show, and a
    // cursor on row 0 of nothing would aim every verb at a row that is not
    // there.
    let (session, _) = listed(0).await;
    let _ = cursor_reports(&session);

    session.settle_cursor();

    assert_eq!(session.cursor_row(), None);
    assert!(cursor_reports(&session).is_empty());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_somebody_put_somewhere_stays_there() {
    // Settling runs on every list change -- a sync pass, a flag, new mail.
    // Pulling the cursor back to the top each time would be #1177 again.
    let (session, _) = listed(5).await;
    session.invoke("next_message");
    session.invoke("next_message");
    let at = session.cursor_row();
    let _ = cursor_reports(&session);

    session.settle_cursor();

    assert_eq!(session.cursor_row(), at);
    assert!(
        cursor_reports(&session).is_empty(),
        "nothing moved, so nothing is said"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_from_the_landing_is_a_choice() {
    let (session, _) = listed(3).await;
    session.settle_cursor();
    let _ = cursor_reports(&session);

    session.invoke("next_message");

    assert!(session.cursor_chosen());
    let reports = cursor_reports(&session);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].2, "the move is reported as chosen: {reports:?}");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn clicking_the_row_it_landed_on_chooses_it() {
    // The click lands on the row already showing. Ignoring it as "no move"
    // would leave that message unreadable by dwell for as long as nobody
    // clicked another one.
    let (session, _) = listed(3).await;
    session.settle_cursor();
    let _ = cursor_reports(&session);

    session.set_cursor_row_ffi(Some(0));

    assert!(session.cursor_chosen());
    let reports = cursor_reports(&session);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].2);

    // A second click on it is no move at all.
    session.set_cursor_row_ffi(Some(0));
    assert!(cursor_reports(&session).is_empty());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_folder_forgets_the_choice() {
    let (session, mailbox) = listed(3).await;
    session.invoke("next_message");
    assert!(session.cursor_chosen());

    session.open_scope(ScopeFfi::Mailbox { mailbox });

    assert!(
        !session.cursor_chosen(),
        "the next folder's landing would otherwise start a read clock"
    );
    session.shutdown();
}
