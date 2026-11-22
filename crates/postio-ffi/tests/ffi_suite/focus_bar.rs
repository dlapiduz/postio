use postio_ffi::{BarModeFfi, DropdownRowKindFfi, DropdownStateFfi, UiEvent};

use crate::focus::{cursor_on_the_first_row, heard, inbox_of};

#[tokio::test(flavor = "multi_thread")]
async fn slash_opens_the_bar_and_in_lists_the_folders_counted() {
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("search");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenBar {
                mode: BarModeFfi::Search,
                text,
                select: None,
            } if text.is_empty()
        ))
        .await,
        "`/` opens the bar for search"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusPlacesChanged
        ))
        .await,
        "the places are read as it opens"
    );
    session.focus_bar_typed("in:In".to_owned());
    let mut folder = None;
    let mut operator = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusDropdown { view } if view.state == DropdownStateFfi::Operator => {
                operator = view.operator_prefix.clone();
                folder = view
                    .sections
                    .iter()
                    .flat_map(|section| &section.rows)
                    .find(|row| row.kind == DropdownRowKindFfi::Folder)
                    .cloned();
                folder.is_some()
            }
            _ => false,
        })
        .await,
        "the folders `in:` completes to"
    );
    let folder = folder.expect("seen");
    assert_eq!(folder.right.as_deref(), Some("2"), "{folder:?}");
    let said =
        |runs: &[postio_ffi::RunFfi]| runs.iter().map(|run| run.text.as_str()).collect::<String>();
    assert_eq!(
        (said(&folder.title), said(&folder.detail)),
        ("Inbox".to_owned(), "in:Inbox".to_owned()),
        "IMAP's INBOX reads Inbox here too"
    );
    assert_eq!(
        operator.as_deref(),
        Some("in:"),
        "drawn tertiary in the field"
    );
    let adds = folder.highlighted.expect("↩ adds, while highlighted");
    assert_eq!((adds.key.as_str(), adds.label.as_str()), ("Return", "adds"));
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folders_popover_lists_the_places_and_opens_one() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("go_to_folders");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenPlaces
        ))
        .await,
        "`g o` opens the popover"
    );
    // Filtered is a place only once today's count, a read of its own, has landed.
    let mut places = Vec::new();
    let wanted = ["Inbox", "Snoozed", "Filtered", "Flagged"];
    assert!(
        heard(&session, 10, |event| {
            if matches!(event, UiEvent::FocusPlacesChanged) {
                places = session.focus_places(String::new());
            }
            places.iter().map(|place| place.name.as_str()).eq(wanted)
        })
        .await,
        "and the places are read: {places:?}"
    );
    let inbox = &places[0];
    assert_eq!(inbox.count.as_deref(), Some("1"), "conversations, counted");
    assert_eq!(inbox.section, "Mailboxes");
    assert_eq!(inbox.command.as_deref(), Some("go_to_inbox"), "its key's");
    assert!(inbox.footer.contains("in:Inbox"), "{}", inbox.footer);
    assert_eq!(
        session.focus_places("snoo".to_owned()).len(),
        1,
        "filtered as typed"
    );

    let snoozed = places
        .iter()
        .find(|place| place.name == "Snoozed")
        .expect("Snoozed");
    session.focus_open_place(snoozed.token);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusPlace { name } if name == "Snoozed"
        ))
        .await,
        "the strip names the place"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusListChanged { total: 0 }
        ))
        .await,
        "and the list shows it"
    );
    session.shutdown();
}
