//! Settings > Contacts across the boundary (specs/011-focus-contacts D7):
//! the section is in the Mac's list, the pane's words come from Rust, and a
//! click on a Suggestions switch reaches the store and comes back as the
//! redrawn pane.

use postio_ffi::{Session, SessionOptions, SyncModeFfi, SyncScopeFfi, SyncStatusFfi, UiEvent};
use postio_storage::test_support;

use crate::focus::heard;

async fn session() -> (std::sync::Arc<Session>, postio_storage::Store) {
    let database = test_support::memory().await;
    (
        Session::open(SessionOptions::in_memory_with(database.clone())).expect("a session"),
        database,
    )
}

#[test]
fn contacts_is_a_section_after_accounts() {
    let keys: Vec<String> = postio_ffi::settings_sections()
        .into_iter()
        .map(|section| section.key)
        .collect();
    let at = keys
        .iter()
        .position(|key| key == "contacts")
        .expect("listed");
    assert_eq!(keys[at - 1], "accounts");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pane_has_its_words_and_a_sync_section_on_the_mac() {
    let (session, _database) = session().await;
    let pane = session.contacts_pane(
        SyncScopeFfi::PeopleAndCompanies,
        SyncModeFfi::ReadOnly,
        SyncStatusFfi::NotOn,
        None,
    );
    assert_eq!(pane.intro_lead, "Your contacts live in Postio,");
    assert!(
        pane.intro_rest.contains("on this Mac next to your mail"),
        "{}",
        pane.intro_rest
    );
    let sync = pane.sync.expect("the Mac has a Sync with section");
    assert_eq!(sync.heading, "Sync with");
    assert_eq!(sync.account.title, "Contacts on this Mac (iCloud)");
    assert_eq!(sync.preview, "Turn on and preview");
    assert_eq!(sync.modes.len(), 2);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_a_switch_changes_the_store_and_redraws_the_pane() {
    let (session, database) = session().await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contacts_settings_opened())
        .await
        .expect("opened");
    let mut rows = Vec::new();
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContactsSettings { view } => {
                rows = view.rows.iter().map(|row| row.on).collect();
                true
            }
            _ => false,
        })
        .await,
        "the pane is drawn once the host has answered"
    );
    assert_eq!(rows, [false, true, true, true]);

    // The third row is "Read signatures for details".
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contacts_suggestion(2))
        .await
        .expect("clicked");
    let mut after = Vec::new();
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContactsSettings { view } => {
                after = view.rows.iter().map(|row| row.on).collect();
                true
            }
            _ => false,
        })
        .await,
        "redrawn"
    );
    assert_eq!(after, [false, true, false, true]);
    let connection = database.connect().await.unwrap();
    let stored = postio_storage::repository::ContactSuggestionRepository::new(&connection)
        .get()
        .await
        .unwrap();
    assert!(!stored.is_on(postio_model::Suggestion::ReadSignatures));
    session.shutdown();
}
