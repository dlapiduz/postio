use chrono::{DateTime, Utc};
use postio_client::protocol::ClientKind;
use postio_core::{Command, Event};
use postio_model::MessageId;
use postio_model::calendar::{AgendaAnchor, AgendaEntry};
use postio_storage::repository::{CalendarRepository, FocusListQuery, ThreadRepository};

use crate::tests::{World, ahead, calendar, focus_caught_up, folder, invitation_mail, when};

fn agenda(world: &World) -> Vec<AgendaEntry> {
    world.rt.block_on(async {
        postio_session::calendar::agenda_page(
            world.database(),
            AgendaAnchor::From(Utc::now() - chrono::TimeDelta::days(1)),
            50,
            &chrono::Local,
            Utc::now(),
        )
        .await
        .expect("a page")
    })
}

fn local_event(world: &World, title: &str, start: DateTime<Utc>, minutes: i64) {
    let ics = calendar(
        "PUBLISH",
        &format!("{title}@example.com"),
        0,
        Utc::now(),
        &when(start, minutes),
    )
    .replace("SUMMARY:Roadmap review", &format!("SUMMARY:{title}"));
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let local = CalendarRepository::new(&connection)
            .local()
            .await
            .expect("local")
            .id;
        postio_session::calendar::store_object_on(
            &connection,
            postio_session::calendar::NewObject {
                calendar: local,
                ics: ics.as_bytes(),
                href: None,
                etag: None,
                my_partstat: None,
                source_message: None,
            },
            &chrono::Local,
            Utc::now(),
        )
        .await
        .expect("stored");
    });
}

fn marker_note(world: &World, message: MessageId) -> Option<String> {
    let inbox = folder(world, postio_model::MailboxRole::Inbox);
    let groups = world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        ThreadRepository::new(&connection)
            .focus_page_at(
                &FocusListQuery {
                    inboxes: vec![(world.account, inbox)],
                    limit: 20,
                    after: None,
                },
                0,
            )
            .await
            .expect("a page")
    });
    let row = groups
        .into_iter()
        .map(|group| group.row)
        .find(|row| {
            row.latest
                .as_ref()
                .is_some_and(|latest| latest.id == message)
        })
        .expect("the invitation's row");
    let marker = row.marker.expect("its marker");
    postio_ui::focus_row::marker_line(&marker, Utc::now(), &chrono::Local).note
}

fn answer(
    world: &World,
    client: &postio_client::Client,
    events: &async_channel::Receiver<postio_core::EventEnvelope>,
    command: Command,
) {
    world.send(client, command);
    let heard = world.hear(events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. }
                | Event::CommandRejected { .. }
                | Event::UndoPerformed { .. }
        )
    });
    assert!(!matches!(heard, Event::CommandRejected { .. }), "{heard:?}");
}

#[test]
fn an_invitation_overlapping_a_local_event_says_what_it_clashes_with() {
    let world = World::new();
    let start = ahead();
    local_event(
        &world,
        "Standup",
        start - chrono::TimeDelta::minutes(15),
        30,
    );
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "review@example.org",
            0,
            Utc::now(),
            &when(start, 45),
        ),
    );

    focus_caught_up(&world);

    assert_eq!(
        marker_note(&world, request).as_deref(),
        Some("Clashes with Standup")
    );
}

#[test]
fn accepting_puts_the_event_on_this_computer_and_undo_takes_it_off() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let start = ahead();
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "review@example.org",
            0,
            Utc::now(),
            &when(start, 45),
        ),
    );
    focus_caught_up(&world);

    let (client, events) = world.frontend(ClientKind::Focus);
    answer(
        &world,
        &client,
        &events,
        Command::AcceptInvite {
            message: Some(request),
        },
    );

    let shown = agenda(&world);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].title, "Roadmap review");
    assert_eq!(shown[0].start, start);
    assert_eq!(shown[0].calendar_name, "On this computer");
    assert_eq!(shown[0].my_partstat.as_deref(), Some("ACCEPTED"));
    assert_eq!(
        marker_note(&world, request),
        None,
        "its own event is no clash"
    );

    answer(&world, &client, &events, Command::Undo);
    assert!(
        agenda(&world).is_empty(),
        "the event is gone with the answer"
    );
}

#[test]
fn an_update_to_an_accepted_event_moves_it_and_says_so() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let request = crate::verbs_tests::invited(&world);
    let start = ahead();
    let moved = start + chrono::TimeDelta::hours(2);
    let stamp = Utc::now();
    let (client, events) = world.frontend(ClientKind::Focus);
    answer(
        &world,
        &client,
        &events,
        Command::AcceptInvite {
            message: Some(request),
        },
    );
    assert_eq!(agenda(&world)[0].start, start);

    let update = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "review@example.org",
            1,
            stamp + chrono::TimeDelta::hours(1),
            &when(moved, 30),
        ),
    );
    focus_caught_up(&world);

    let shown = agenda(&world);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].start, moved, "the calendar's copy moved");
    let note = marker_note(&world, update).expect("a note");
    assert!(note.starts_with("Moved to "), "{note}");
}

#[test]
fn a_tentative_answer_puts_the_event_on_the_calendar_as_tentative() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let request = crate::verbs_tests::invited(&world);
    let (client, events) = world.frontend(ClientKind::Focus);

    answer(
        &world,
        &client,
        &events,
        Command::TentativeInvite {
            message: Some(request),
        },
    );

    assert_eq!(
        crate::verbs_tests::answer_on(&world, request),
        Some(postio_model::listing::InviteAnswer::TentativePending)
    );
    let shown = agenda(&world);
    assert_eq!(shown[0].my_partstat.as_deref(), Some("TENTATIVE"));
    assert_eq!(
        postio_ui::calendar::agenda_rows(&shown, Utc::now(), &chrono::Local)
            .iter()
            .find_map(|row| match row {
                postio_ui::calendar::AgendaRow::Occurrence(row) =>
                    row.chip.as_ref().map(|chip| chip.label()),
                postio_ui::calendar::AgendaRow::Day { .. } => None,
            })
            .as_deref(),
        Some("Tentative")
    );
}
