//! The Contacts view across the boundary (specs/011-focus-contacts S7):
//! `g c` draws a frame and the rows the table asks for, the card follows the
//! cursor, a tab and the filter narrow the list, and Escape unwinds.

use postio_ffi::{ContactTabFfi, Session, SessionOptions, SurfaceKindFfi, UiEvent};
use postio_model::EmailAddress;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

use crate::focus::heard;

async fn with_people() -> std::sync::Arc<Session> {
    with_people_in(test_support::memory().await).await
}

async fn with_people_in(database: postio_storage::Store) -> std::sync::Arc<Session> {
    {
        let connection = database.connect().await.expect("a connection");
        let contacts = ContactRepository::new(&connection);
        for (name, email) in [
            ("Ada Moreno", "ada@example.org"),
            ("Adam Okoro", "adam@example.net"),
            ("Ben Adeyemi", "ben@example.net"),
        ] {
            contacts
                .create(Some(name), &[EmailAddress::new(None::<String>, email)])
                .await
                .expect("a person");
        }
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

async fn row_when_ready(
    session: &std::sync::Arc<Session>,
    index: u32,
) -> postio_ffi::ContactRowViewFfi {
    for _ in 0..200 {
        let s = session.clone();
        let found = tokio::task::spawn_blocking(move || s.focus_contacts_row(index))
            .await
            .expect("the read");
        if let Some(row) = found {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("row {index} never landed");
}

#[tokio::test(flavor = "multi_thread")]
async fn g_c_draws_the_frame_rows_in_letters_and_the_card_of_the_cursor() {
    let session = with_people().await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.invoke("open_contacts"))
        .await
        .expect("g c");

    let mut counted = None;
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContacts { view } if view.loaded => {
                counted = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "the frame is drawn once the counts are read"
    );
    let view = counted.expect("seen");
    assert_eq!(view.total, 3);
    assert_eq!(view.strip.total, "3");
    assert_eq!(view.strip.tabs.len(), 5);
    assert!(view.strip.review.is_none());

    let first = row_when_ready(&session, 0).await;
    assert_eq!(
        (first.name.as_str(), first.letter.as_deref()),
        ("Ada Moreno", Some("A"))
    );
    assert_eq!(first.avatar.text, "AM");
    let third = row_when_ready(&session, 2).await;
    assert_eq!(
        (third.name.as_str(), third.letter.as_deref()),
        ("Ben Adeyemi", Some("B"))
    );

    let mut card = None;
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContactsCard { card: Some(shown) } => {
                card = Some(shown.clone());
                true
            }
            _ => false,
        })
        .await,
        "the card of the first row"
    );
    let card = card.expect("seen");
    assert_eq!(card.name, "Ada Moreno");
    assert_eq!(card.addresses[0].address, "ada@example.org");
    assert_eq!(card.tiles.len(), 4);
    assert_eq!(card.months.bars.len(), 12);

    let s = session.clone();
    tokio::task::spawn_blocking(move || s.invoke("next_message"))
        .await
        .expect("j");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContactsCursor { index: Some(1) }
        ))
        .await
    );

    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contacts_filter("ben".into()))
        .await
        .expect("typed");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContacts { view } if view.total == 1 && view.filter == "ben"
        ))
        .await,
        "the list is the filter's"
    );
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.focus_contacts_tab(ContactTabFfi::Companies))
        .await
        .expect("a tab");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContacts { view } if view.strip.tabs.iter().any(|t| t.on && t.title == "Companies")
        ))
        .await
    );

    let s = session.clone();
    tokio::task::spawn_blocking(move || {
        s.invoke("back");
        s.invoke("back");
    })
    .await
    .expect("escape");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: SurfaceKindFfi::Contacts
            }
        ))
        .await,
        "Escape clears the filter, then leaves"
    );
    session.shutdown();
}

/// Specs/011-focus-contacts S8, across the boundary: Cmd-E draws the form on
/// the person under the cursor, a change is reported, Esc arms, Cmd-S saves
/// as one command, and the card says what was saved.
#[tokio::test(flavor = "multi_thread")]
async fn cmd_e_edits_the_person_and_cmd_s_saves_what_was_typed() {
    use postio_ffi::EditChangeFfi;

    let database = test_support::memory().await;
    let session = with_people_in(database.clone()).await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || s.invoke("open_contacts"))
        .await
        .expect("g c");
    // The table asks for its rows, which is what reads the first card.
    row_when_ready(&session, 0).await;
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContactsCard { card: Some(_) }
        ))
        .await,
        "the card is read before it is edited"
    );

    let s = session.clone();
    tokio::task::spawn_blocking(move || s.invoke("contact_edit"))
        .await
        .expect("cmd e");
    let mut opened = None;
    assert!(
        heard(&session, 20, |event| match event {
            UiEvent::FocusContactEdit { view: Some(view) } => {
                opened = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "the form is drawn"
    );
    let form = opened.expect("seen");
    assert_eq!(form.title, "Editing Ada Moreno");
    assert_eq!(form.addresses[0].text, "ada@example.org");

    let s = session.clone();
    tokio::task::spawn_blocking(move || {
        s.focus_contact_edit_change(EditChangeFfi::Title {
            text: "Finance lead".into(),
        });
        s.invoke("back");
    })
    .await
    .expect("typed, then Esc");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContactEdit { view: Some(view) } if view.armed.is_some()
        ))
        .await,
        "the first Esc on an edited form arms"
    );
    assert!(session.focus_contact_edit().is_some(), "and the form stays");

    let s = session.clone();
    tokio::task::spawn_blocking(move || s.invoke("contact_save"))
        .await
        .expect("cmd s");
    assert!(
        heard(&session, 20, |event| matches!(
            event,
            UiEvent::FocusContactEdit { view: None }
        ))
        .await,
        "Save puts the form away"
    );
    // The save is a command on the engine's own task: it lands shortly.
    let mut title = None;
    for _ in 0..100 {
        let connection = database.connect().await.expect("a connection");
        title = ContactRepository::new(&connection)
            .by_address("ada@example.org")
            .await
            .expect("a read")
            .and_then(|saved| saved.title);
        if title.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(title.as_deref(), Some("Finance lead"));
}
