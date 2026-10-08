//! The pickers at the row and the toast's policy at the boundary
//! (specs/009-focus-macos T089, for the Mac's T091-T093).
//!
//! The controller decides (`postio-focus`'s `tests/pickers.rs`); these
//! assert the Mac gets it through the session: `s`, `h`, `l` and `m` open a
//! picker as `FocusOpenPicker`, what it lists comes as `FocusPickerRows`
//! once read, a row is chosen or toggled by its token, the number keys and
//! Return run through `invoke`, and what is chosen reaches the host, which
//! answers with the toast. A toast says how long it stays, and a queued
//! send's Undo cancels the send.

use std::time::Duration;

use chrono::Utc;
use postio_ffi::{
    KeyOutcomeFfi, ModifiersFfi, PickerAnchorFfi, PickerFieldFfi, PickerKindFfi, PickerViewFfi,
    Session, SessionOptions, SurfaceKindFfi, ToastKindFfi, UiContext, UiEvent,
};
use postio_model::Message;
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::test_support;

use crate::focus::{cursor_on_the_first_row, heard, inbox_of};

const NONE: ModifiersFfi = ModifiersFfi {
    control: false,
    option: false,
    shift: false,
    command: false,
};

/// Wait for the picker to be opened or redrawn as `wanted` accepts, and
/// hand it back.
async fn picker(
    session: &Session,
    mut wanted: impl FnMut(&PickerViewFfi) -> bool,
) -> PickerViewFfi {
    let mut seen = None;
    assert!(
        heard(session, 10, |event| match event {
            UiEvent::FocusOpenPicker { view } | UiEvent::FocusPickerRows { view }
                if wanted(view) =>
            {
                seen = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "the picker wanted was never drawn"
    );
    seen.expect("seen")
}

async fn closes_the_picker(session: &Session) -> bool {
    heard(session, 5, |event| {
        matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: SurfaceKindFfi::Picker
            }
        )
    })
    .await
}

/// The next toast, as the Mac hears it.
async fn toast(session: &Session) -> (String, ToastKindFfi, bool, Option<u32>) {
    let mut said = None;
    assert!(
        heard(session, 10, |event| match event {
            UiEvent::FocusToast {
                text,
                kind,
                undoable,
                seconds,
            } => {
                said = Some((text.clone(), *kind, *undoable, *seconds));
                true
            }
            _ => false,
        })
        .await,
        "no toast"
    );
    said.expect("said")
}

#[tokio::test(flavor = "multi_thread")]
async fn s_opens_the_snooze_picker_at_the_cursors_row_and_a_number_chooses() {
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("snooze");
    let view = picker(&session, |_| true).await;
    assert_eq!(view.kind, PickerKindFfi::Snooze);
    assert_eq!(view.anchor, PickerAnchorFfi::Row { position: 0 });
    assert_eq!(view.title, postio_ui::pickers::SNOOZE_TITLE);
    assert_eq!(view.target, "Ada \u{b7} First");
    assert_eq!(view.field, PickerFieldFfi::Date);
    assert_eq!(view.hint.as_deref(), Some(postio_ui::pickers::TYPE_A_DATE));
    let keys: Vec<Option<&str>> = view.rows.iter().map(|row| row.key.as_deref()).collect();
    assert_eq!(keys, [Some("1"), Some("2"), Some("3"), Some("4")]);

    // `1` is the picker's while it is up: the key context is the picker's.
    assert_eq!(
        session.key(Some("1"), None, NONE, UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "picker_choose_1".to_owned()
        }
    );
    session.invoke("picker_choose_1");
    assert!(closes_the_picker(&session).await, "choosing closes it");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusKeyboardHome
        ))
        .await,
        "and the keyboard goes back to the list"
    );
    let (_, _, _, seconds) = toast(&session).await;
    assert!(seconds.is_some(), "a toast says how long it stays");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_and_a_typed_date_are_previewed_and_return_chooses_it() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("remind_if_no_reply");
    let view = picker(&session, |_| true).await;
    assert_eq!(view.kind, PickerKindFfi::Remind);
    session.invoke("picker_type_date");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusPickerField
        ))
        .await,
        "Tab puts the keyboard in the date field"
    );
    session.focus_picker_typed("receipts".to_owned());
    picker(&session, |view| {
        view.hint.as_deref() == Some(postio_ui::pickers::NOT_A_DATE)
    })
    .await;
    session.focus_picker_typed("tue 9am".to_owned());
    let view = picker(&session, |view| view.typed == "tue 9am").await;
    let hint = view.hint.expect("a hint");
    assert!(hint.contains("09:00"), "{hint}");
    session.invoke("picker_confirm");
    assert!(closes_the_picker(&session).await, "Return chose the date");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_label_is_made_with_return_and_space_takes_it_off() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("add_label");
    let opened = picker(&session, |_| true).await;
    assert_eq!(opened.kind, PickerKindFfi::Label);
    assert_eq!(opened.field, PickerFieldFfi::Filter);
    assert_eq!(opened.placeholder, postio_ui::pickers::LABEL_FILTER);
    // The labels are read once it is up; the fixture account has none.
    session.focus_picker_typed("Receipts".to_owned());
    let view = picker(&session, |view| view.rows.iter().any(|row| row.create)).await;
    assert_eq!(
        view.rows[0].name,
        postio_ui::pickers::create_label("Receipts")
    );
    session.invoke("picker_confirm");
    assert!(
        closes_the_picker(&session).await,
        "Return makes it and closes"
    );
    let (text, kind, undoable, _) = toast(&session).await;
    assert_eq!(kind, ToastKindFfi::Completed, "{text}");
    assert!(undoable, "the host put the label on: {text}");

    // Open again: it is there, and on the conversation.
    session.invoke("add_label");
    let view = picker(&session, |view| {
        view.rows.iter().any(|row| row.name == "Receipts")
    })
    .await;
    let receipts = view
        .rows
        .iter()
        .find(|row| row.name == "Receipts")
        .expect("the label");
    assert!(receipts.applied && receipts.dot, "{receipts:?}");
    session.focus_picker_toggle(receipts.token);
    let view = picker(&session, |view| {
        view.rows
            .iter()
            .any(|row| row.name == "Receipts" && !row.applied)
    })
    .await;
    assert_eq!(view.kind, PickerKindFfi::Label, "Space keeps it up");
    let (text, _, _, _) = toast(&session).await;
    assert!(!text.is_empty());
    session.shutdown();
}

/// A session whose inbox holds one conversation from Ada, beside a
/// Receipts folder.
async fn an_inbox_and_receipts() -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Receipts").await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some("Invoice".to_owned());
        message.date = Some(Utc::now());
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ada"),
            "ada@example.com",
        )];
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let mut thread = postio_model::Thread::new(account.id);
        thread.subject = message.subject.clone();
        threads.create(&mut thread).await.expect("a thread");
        threads.add_message(thread.id, id).await.expect("threaded");
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn move_lists_the_destinations_and_choosing_one_moves_the_mail() {
    let session = an_inbox_and_receipts().await;
    cursor_on_the_first_row(&session).await;

    session.invoke("move");
    let view = picker(&session, |view| !view.rows.is_empty()).await;
    assert_eq!(view.kind, PickerKindFfi::Move);
    let names: Vec<&str> = view.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["Receipts"], "the inbox is where the mail is");
    assert_eq!(
        view.rows[0].section.as_deref(),
        Some(postio_ui::pickers::ALL_FOLDERS_HEADING)
    );
    session.focus_picker_choose(view.rows[0].token);
    assert!(closes_the_picker(&session).await);
    let (text, kind, undoable, seconds) = toast(&session).await;
    assert_eq!(kind, ToastKindFfi::Completed, "{text}");
    assert!(undoable, "{text}");
    assert_eq!(
        seconds,
        Some(postio_ui::focus_target::TOAST_SECONDS),
        "the controller says how long, not the Mac"
    );

    // Moved once, it is Recent and `1` the next time.
    session.invoke("move");
    let view = picker(&session, |view| {
        view.rows.first().is_some_and(|row| row.key.is_some())
    })
    .await;
    assert_eq!(view.rows[0].name, "Receipts");
    assert_eq!(view.rows[0].key.as_deref(), Some("1"));
    assert_eq!(
        view.rows[0].section.as_deref(),
        Some(postio_ui::pickers::RECENT_HEADING)
    );
    session.invoke("back");
    assert!(closes_the_picker(&session).await, "Esc closes it");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn in_an_empty_filter_a_digit_or_space_is_the_pickers_and_otherwise_typing() {
    let session = an_inbox_and_receipts().await;
    cursor_on_the_first_row(&session).await;
    session.invoke("add_label");
    picker(&session, |_| true).await;

    assert_eq!(
        session.key(Some(" "), None, NONE, UiContext::List, true),
        KeyOutcomeFfi::Command {
            id: "picker_toggle".to_owned()
        },
        "nothing to type into yet: Space is the picker's"
    );
    session.focus_picker_typed("Re".to_owned());
    picker(&session, |view| view.typed == "Re").await;
    assert_eq!(
        session.key(Some(" "), None, NONE, UiContext::List, true),
        KeyOutcomeFfi::Unhandled,
        "a space in a name is typing"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_send_says_so_and_undo_takes_the_send_back() {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        test_support::account_with_inbox(&connection).await;
    }
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone()).with_blobs_for_test(blobs, scratch),
    )
    .expect("a session");
    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);

    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    assert_eq!(session.send_draft(draft).await, None, "queued");
    let (text, kind, undoable, seconds) = toast(&session).await;
    assert_eq!(text, postio_ui::sending::QUEUED_TO_SEND);
    assert_eq!(kind, ToastKindFfi::Completed);
    assert!(undoable);
    assert_eq!(seconds, Some(postio_ui::focus_target::TOAST_SECONDS));

    session.invoke("undo");
    let connection = database.connect().await.expect("a connection");
    let drafts = DraftRepository::new(&connection);
    let deadline =
        tokio::time::Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    loop {
        let listed = drafts
            .list_for_account(postio_model::ids::AccountId::new(1))
            .await
            .expect("the drafts");
        if listed
            .iter()
            .all(|draft| draft.state == postio_model::DraftState::Editing)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Undo cancelled the send: {:?}",
            listed.iter().map(|draft| draft.state).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    session.shutdown();
}
