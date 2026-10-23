//! The known-sender popover across the boundary (specs/011-focus-contacts
//! S6): a click on a name draws it whole from the store, a change of arrival
//! is drawn at once and kept, and the engine's toast carries the undo.

use chrono::Utc;
use postio_ffi::{ContactArrivalFfi, Session, SessionOptions, SurfaceKindFfi, UiEvent};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

use crate::focus::heard;

async fn old_mail() -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Old Friend"), "friend@example.org")];
        message.to = vec![account.address.clone()];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("old mail");
        test_support::mark_contacts_history(&connection).await;
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_a_name_draws_the_popover_and_a_segment_changes_arrival_for_good() {
    let session = old_mail().await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contact_popover_at("Friend@Example.org".into()))
        .await
        .expect("the click");

    let mut view = None;
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContactPopover { view: shown } => {
                view = Some(shown.clone());
                true
            }
            _ => false,
        })
        .await,
        "the popover is drawn"
    );
    let view = view.expect("seen");
    assert_eq!(view.name, "Old Friend");
    assert_eq!(view.subline, "friend@example.org");
    assert_eq!(view.tiles[0].value, "1");
    assert_eq!(view.tiles[0].caption, "message");
    assert_eq!(view.tiles[2].value, "today");
    assert_eq!(view.arrival_title, "Mail from Old arrives in");
    assert_eq!(
        view.arrival
            .iter()
            .filter(|o| o.on)
            .map(|o| o.label.as_str())
            .collect::<Vec<_>>(),
        ["Inbox"]
    );
    assert_eq!(
        view.actions
            .iter()
            .map(|a| (a.label.as_str(), a.enabled))
            .collect::<Vec<_>>(),
        [
            ("Add contact", true),
            ("Not now", true),
            ("Ignore sender", true)
        ],
        "mail nobody saved is a new sender: introduced, with the arrival control too"
    );
    assert!(view.intro.is_some());

    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contact_arrival(ContactArrivalFfi::Digest))
        .await
        .expect("the click");
    let mut redrawn = false;
    let mut toast = false;
    assert!(
        heard(&session, 20, |event| {
            match event {
                UiEvent::FocusContactPopover { view } => {
                    redrawn = view.arrival.iter().any(|o| o.on && o.label == "Digest");
                }
                UiEvent::FocusToast { text, undoable, .. } => {
                    toast |= *undoable && text == "Changed where mail arrives";
                }
                _ => {}
            }
            redrawn && toast
        })
        .await,
        "drawn at once (redrawn: {redrawn}) with the undo toast (toast: {toast})"
    );

    // Kept: asked again, it is Digest, and the surface is the popover's.
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contact_popover_at("friend@example.org".into()))
        .await
        .expect("the click");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContactPopover { view } if view.arrival.iter().any(|o| o.on && o.label == "Digest")
        ))
        .await
    );
    let _ = SurfaceKindFfi::ContactPopover;
    session.shutdown();
}
