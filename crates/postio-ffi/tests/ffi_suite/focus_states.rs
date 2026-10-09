//! What Focus says about its own state, at the boundary
//! (specs/009-focus-macos T096, for the Mac's T099).
//!
//! The controller decides (`postio-focus`'s `tests/states.rs`); these assert
//! the Mac hears it through the session's events: the engine's sync events
//! become `FocusBanner` and `FocusSyncLabel`, a refused password names the
//! account whose credential the sheet asks for, an empty inbox becomes
//! `FocusEmpty` with only the shortcuts that lead somewhere, and a backfill
//! crosses typed rather than as `Other`.

use postio_core::{CommandId, ConnectionState, Event, FailureReason};
use postio_ffi::{Session, SessionOptions, SyncMarkFfi, UiEvent};
use postio_storage::test_support;

use crate::focus::heard;

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_reaches_the_mac_as_its_banner_and_its_label() {
    let session = session();
    session.emit_for_test(Event::ConnectionChanged {
        account: 1.into(),
        state: ConnectionState::Offline,
    });
    let mut banner = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusBanner { banner: Some(said) } => {
                banner = Some(said.clone());
                true
            }
            _ => false,
        })
        .await,
        "the offline banner is drawn"
    );
    let banner = banner.expect("seen");
    assert_eq!(banner.heading, "You're offline");
    assert!(!banner.error);
    let button = banner.button.expect("Retry now");
    assert_eq!(button.label, "Retry now");
    assert_eq!(button.command, CommandId::Refresh.to_string());
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusSyncLabel { text, mark: SyncMarkFfi::Offline } if text == "Offline"
        ))
        .await,
        "the toolbar's label says the same"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_password_names_the_account_whose_credential_to_ask_for() {
    let database = test_support::memory().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        test_support::account_with_inbox(&connection).await.0
    };
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.emit_for_test(Event::ConnectionChanged {
        account: account.id,
        state: ConnectionState::Failing {
            reason: FailureReason::Auth,
        },
    });
    let mut banner = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusBanner { banner: Some(said) } if said.account.is_some() => {
                banner = Some(said.clone());
                true
            }
            _ => false,
        })
        .await,
        "the sign-in banner is drawn once the account is known"
    );
    let banner = banner.expect("seen");
    assert_eq!(banner.account, Some(account.id.get()));
    assert_eq!(
        banner.heading,
        format!("Can't sign in to {}", account.incoming.host)
    );
    assert!(banner.error, "drawn in the error colour");
    let button = banner.button.expect("Update password…");
    assert_eq!(button.label, "Update password\u{2026}");
    assert_eq!(button.command, CommandId::UpdateCredential.to_string());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_progress_crosses_typed() {
    let session = session();
    session.emit_for_test(Event::BackfillProgress {
        account: 1.into(),
        done: 3,
        total: 10,
        footprint: None,
    });
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::BackfillProgress {
                account: 1,
                done: 3,
                total: 10
            }
        ))
        .await,
        "a backfill is no longer `Other`"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_inbox_says_why_with_only_the_shortcuts_that_exist() {
    let session = session();
    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);
    let mut page = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusEmpty { page: Some(said) } => {
                page = Some(said.clone());
                true
            }
            _ => false,
        })
        .await,
        "the empty inbox is drawn in the list's place"
    );
    let page = page.expect("seen");
    // No pass has finished, so it is not yet "empty": mail has not arrived.
    assert_eq!(page.heading, "Syncing your inbox\u{2026}");
    assert_eq!(
        page.shortcuts
            .iter()
            .map(|shortcut| shortcut.command.as_str())
            .collect::<Vec<_>>(),
        vec!["compose"],
        "nothing has been filtered or archived yet"
    );
    session.shutdown();
}
