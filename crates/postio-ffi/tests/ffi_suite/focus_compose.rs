use std::time::Duration;

use postio_ffi::{ComposerKindFfi, Session, SurfaceKindFfi, UiEvent};

use crate::focus::{cursor_on_the_first_row, heard, heard_within, inbox_of};

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
    // Unscaled: the autosave quiet period is 1.5 s, so a longer wait could not tell "now" from
    // "after it".
    assert!(
        heard_within(&session, Duration::from_secs(1), |event| matches!(
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
