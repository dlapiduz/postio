//! Writing and replying at the boundary (specs/009-focus-macos T073, for
//! the Mac's T079).
//!
//! The rules are the controller's (`postio-focus`'s `tests/compose.rs`);
//! these assert the Mac hears them through the session over a real store:
//! the writing verbs open the composer as `FocusComposer`, the driver
//! honours the controller's autosave timer so a burst of edits is one
//! `FocusSaveDraft`, Esc saves at once and the toast says so once the save
//! has landed, a send verb on mail that is no draft says so, and with no
//! account `c` offers to add one.

use std::time::Duration;

use postio_ffi::{ComposerKindFfi, Session, SessionOptions, SurfaceKindFfi, UiEvent};

use crate::focus::{cursor_on_the_first_row, heard, inbox_of};

/// The composer the controller opened, as the Mac hears it.
async fn composer_opened(session: &Session) -> Option<(ComposerKindFfi, Option<i64>)> {
    let mut opened = None;
    heard(session, 5, |event| match event {
        UiEvent::FocusComposer { kind, message } => {
            opened = Some((*kind, *message));
            true
        }
        _ => false,
    })
    .await;
    opened
}

#[tokio::test(flavor = "multi_thread")]
async fn c_and_e_open_the_composer_through_the_controller() {
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;
    let first = session.focus_row_at(0).expect("the first row").id;

    session.invoke("compose");
    assert_eq!(
        composer_opened(&session).await,
        Some((ComposerKindFfi::New, None)),
        "c writes a new message"
    );
    session.invoke("back");
    session.invoke("reply");
    assert_eq!(
        composer_opened(&session).await,
        Some((ComposerKindFfi::Reply, Some(first))),
        "e answers the cursor's row"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_edits_is_one_save_once_the_quiet_period_has_passed() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;
    session.invoke("compose");
    assert!(composer_opened(&session).await.is_some());

    for _ in 0..3 {
        session.focus_composer_edited();
    }
    let mut saves = Vec::new();
    // Past the quiet period, with room for a slow machine.
    let _ = heard(&session, 5, |event| {
        if let UiEvent::FocusSaveDraft { composition } = event {
            saves.push(*composition);
        }
        false
    })
    .await;
    assert_eq!(
        saves,
        vec![1],
        "three edits, one save, of the composition open"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn esc_saves_at_once_and_the_toast_says_the_draft_was_kept() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;
    session.invoke("compose");
    assert!(composer_opened(&session).await.is_some());
    session.focus_composer_edited();

    session.invoke("back");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: SurfaceKindFfi::Composer
            }
        ))
        .await,
        "Esc closes the composer"
    );
    assert!(
        heard(&session, 1, |event| matches!(
            event,
            UiEvent::FocusSaveDraft { composition: 1 }
        ))
        .await,
        "and saves it now, not after the quiet period"
    );
    session.focus_draft_saved(1, true, None);
    let mut said = None;
    assert!(
        heard(&session, 5, |event| match event {
            UiEvent::FocusToast { text, .. } => {
                said = Some(text.clone());
                true
            }
            _ => false,
        })
        .await
    );
    let said = said.unwrap_or_default();
    assert!(
        said.starts_with("Draft saved locally "),
        "the toast says the draft was kept: {said:?}"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_the_send_of_mail_that_is_no_draft_says_so() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;
    session.invoke("cancel_send");
    let mut said = None;
    assert!(
        heard(&session, 5, |event| match event {
            UiEvent::FocusToast { text, .. } => {
                said = Some(text.clone());
                true
            }
            _ => false,
        })
        .await
    );
    assert_eq!(
        said.as_deref(),
        Some(postio_ui::focus_target::NOT_BEING_SENT),
        "the host was asked which draft, and there was none"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_account_c_offers_to_add_one() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusListChanged { .. }
        ))
        .await,
        "the inbox opened, and the controller knows there is no account"
    );
    session.invoke("compose");
    let mut offer = None;
    assert!(
        heard(&session, 5, |event| match event {
            UiEvent::FocusOffer {
                text,
                label,
                command,
            } => {
                offer = Some((text.clone(), label.clone(), command.clone()));
                true
            }
            _ => false,
        })
        .await
    );
    assert_eq!(
        offer,
        Some((
            postio_ui::focus_target::NO_ACCOUNT_TO_WRITE_FROM.to_owned(),
            postio_ui::focus_target::ADD_ACCOUNT.to_owned(),
            "add_account".to_owned()
        ))
    );
    session.shutdown();
}

#[test]
fn the_autosave_waits_what_the_widgets_composer_waited() {
    assert_eq!(postio_focus::AUTOSAVE, Duration::from_millis(1500));
}
