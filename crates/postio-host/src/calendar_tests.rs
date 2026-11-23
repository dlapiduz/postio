use chrono::{DateTime, Utc};
use postio_client::protocol::ClientKind;
use postio_core::{Command, Event};
use postio_model::MessageId;
use postio_model::calendar::{AgendaAnchor, AgendaEntry, EditScope, EventFields, EventTarget};
use postio_storage::repository::{CalendarRepository, FocusListQuery, ThreadRepository};

use crate::tests::{
    World, ahead, calendar, egress, focus_caught_up, folder, invitation_mail, when,
};

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

fn stored(world: &World, ics: &str) {
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

fn daily_standup(world: &World, first: DateTime<Utc>) {
    stored(
        world,
        &format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Postio tests//EN\r\nBEGIN:VEVENT\r\n\
             UID:standup@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:{}\r\nDTEND:{}\r\n\
             RRULE:FREQ=DAILY;COUNT=4\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            crate::tests::ics_utc(first),
            crate::tests::ics_utc(first + chrono::TimeDelta::minutes(15)),
        ),
    );
}

fn fields(title: &str, start: DateTime<Utc>, minutes: i64) -> EventFields {
    EventFields {
        title: title.to_owned(),
        start,
        end: start + chrono::TimeDelta::minutes(minutes),
        all_day: false,
        location: String::new(),
        notes: String::new(),
        attendees: Vec::new(),
        minutes_before: None,
    }
}

fn target(entry: &AgendaEntry) -> EventTarget {
    EventTarget {
        occurrence: entry.cursor(),
        recurrence_id: entry.recurrence_id.clone(),
    }
}

fn changes(world: &World) -> Vec<String> {
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::sql::all(
            &connection,
            "SELECT kind FROM event_changes ORDER BY id",
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")
    })
}

fn queued_drafts(world: &World) -> Vec<postio_model::Draft> {
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::repository::DraftRepository::new(&connection)
            .list_for_account(world.account)
            .await
            .expect("drafts")
    })
}

fn refused(
    world: &World,
    client: &postio_client::Client,
    events: &async_channel::Receiver<postio_core::EventEnvelope>,
    command: Command,
) -> String {
    world.send(client, command);
    match world.hear(events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    }) {
        Event::CommandRejected { reason, .. } => reason,
        other => panic!("it ran: {other:?}"),
    }
}

#[test]
fn a_new_event_is_stored_and_logged_with_no_connection_and_undo_removes_it() {
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let start = ahead();

    answer(
        &world,
        &client,
        &events,
        Command::SaveEvent {
            event: None,
            calendar: None,
            fields: Some(fields("Dentist", start, 30)),
            scope: None,
        },
    );

    let shown = agenda(&world);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(
        (shown[0].title.as_str(), shown[0].start),
        ("Dentist", start)
    );
    assert!(!shown[0].queued, "On this computer waits on no server");
    assert_eq!(changes(&world), ["create"]);
    assert!(egress(&world).is_empty(), "{:?}", egress(&world));

    answer(&world, &client, &events, Command::Undo);
    assert!(agenda(&world).is_empty());
    assert_eq!(changes(&world), ["create", "delete"]);
}

#[test]
fn a_recurring_event_asks_which_occurrences_and_each_scope_does_what_it_says() {
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let first = ahead();
    daily_standup(&world, first);
    let days = agenda(&world);
    assert_eq!(days.len(), 4, "{days:?}");
    let later = chrono::TimeDelta::hours(1);

    let reason = refused(
        &world,
        &client,
        &events,
        Command::MoveEventTo {
            event: target(&days[1]),
            start: days[1].start + later,
            scope: None,
        },
    );
    assert_eq!(
        reason,
        "Choose this occurrence, this and following, or all of them"
    );

    answer(
        &world,
        &client,
        &events,
        Command::MoveEventTo {
            event: target(&days[1]),
            start: days[1].start + later,
            scope: Some(EditScope::This),
        },
    );
    let starts: Vec<_> = agenda(&world).iter().map(|entry| entry.start).collect();
    assert_eq!(
        starts,
        [
            days[0].start,
            days[1].start + later,
            days[2].start,
            days[3].start
        ]
    );

    answer(
        &world,
        &client,
        &events,
        Command::SaveEvent {
            event: Some(target(&days[2])),
            calendar: None,
            fields: Some(fields("Standup (new room)", days[2].start, 15)),
            scope: Some(EditScope::ThisAndFollowing),
        },
    );
    let titles: Vec<_> = agenda(&world)
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(
        titles,
        [
            "Standup",
            "Standup",
            "Standup (new room)",
            "Standup (new room)"
        ]
    );

    answer(
        &world,
        &client,
        &events,
        Command::DeleteEvent {
            event: target(&agenda(&world)[0]),
            scope: Some(EditScope::All),
        },
    );
    let left: Vec<_> = agenda(&world)
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(
        left,
        ["Standup (new room)", "Standup (new room)"],
        "the split series is its own"
    );
    assert!(egress(&world).is_empty());
}

#[test]
fn saving_with_attendees_sends_nothing() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let (client, events) = world.frontend(ClientKind::Focus);
    let mut planning = fields("Planning", ahead(), 60);
    planning.attendees = vec![postio_model::EmailAddress::new(
        None::<String>,
        "quinn@example.net",
    )];

    answer(
        &world,
        &client,
        &events,
        Command::SaveEvent {
            event: None,
            calendar: None,
            fields: Some(planning),
            scope: None,
        },
    );

    assert_eq!(agenda(&world).len(), 1);
    assert!(
        queued_drafts(&world).is_empty(),
        "{:?}",
        queued_drafts(&world)
    );
    assert!(egress(&world).is_empty());
}

#[test]
fn save_and_send_queues_one_invitation_behind_the_window_and_undo_takes_both_back() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let (client, events) = world.frontend(ClientKind::Focus);
    let mut planning = fields("Planning", ahead(), 60);
    planning.attendees = vec![postio_model::EmailAddress::new(
        None::<String>,
        "quinn@example.net",
    )];

    answer(
        &world,
        &client,
        &events,
        Command::SaveAndSendEvent {
            event: None,
            calendar: None,
            fields: Some(planning),
            scope: None,
        },
    );

    let drafts = queued_drafts(&world);
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let invitation = &drafts[0];
    assert_eq!(invitation.to[0].address, "quinn@example.net");
    let ics = invitation
        .calendar_reply
        .as_deref()
        .expect("its calendar part");
    assert!(
        ics.contains("METHOD:REQUEST") && ics.contains("SUMMARY:Planning"),
        "{ics}"
    );
    assert_eq!(
        postio_model::outgoing::CalendarMethod::of(ics.as_bytes()),
        postio_model::outgoing::CalendarMethod::Request
    );
    let not_before = world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::repository::OperationQueueRepository::new(&connection)
            .pending_for(postio_model::OperationTarget::Draft(invitation.id))
            .await
            .expect("a read")
            .expect("its send is queued")
            .next_attempt_at
            .expect("held back")
    });
    assert_eq!(
        (not_before - invitation.updated_at).num_seconds(),
        postio_core::command::RSVP_WINDOW.as_secs() as i64,
        "held back for exactly the undo window"
    );
    assert!(
        egress(&world).is_empty(),
        "nothing leaves inside the window"
    );

    answer(&world, &client, &events, Command::Undo);
    assert!(agenda(&world).is_empty());
    assert!(
        queued_drafts(&world)
            .iter()
            .all(|draft| draft.state != postio_model::DraftState::Queued),
        "{:?}",
        queued_drafts(&world)
    );
}

#[test]
fn a_deleted_event_comes_back_on_undo_and_a_duplicate_is_its_own() {
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    local_event(&world, "Dentist", ahead(), 30);
    let shown = agenda(&world);

    answer(
        &world,
        &client,
        &events,
        Command::DuplicateEventOf {
            event: shown[0].event,
        },
    );
    let both = agenda(&world);
    assert_eq!(both.len(), 2);
    assert_ne!(both[0].event, both[1].event);

    answer(
        &world,
        &client,
        &events,
        Command::DeleteEvent {
            event: target(&shown[0]),
            scope: None,
        },
    );
    assert_eq!(agenda(&world).len(), 1);

    answer(&world, &client, &events, Command::Undo);
    let back = agenda(&world);
    assert_eq!(back.len(), 2);
    assert!(back.iter().all(|entry| entry.title == "Dentist"));
}

#[test]
fn a_scheduling_server_gets_the_answer_as_its_event_and_no_mail_goes() {
    let world = World::new();
    crate::verbs_tests::sending(&world);
    let work = world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        CalendarRepository::new(&connection)
            .create(&postio_model::calendar::NewCalendar {
                account: Some(world.account),
                href: Some("work".to_owned()),
                name: "Work".to_owned(),
                colour: None,
                writable: true,
                schedules: true,
            })
            .await
            .expect("a calendar")
    });
    let request = crate::verbs_tests::invited(&world);
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

    assert!(queued_drafts(&world).is_empty(), "no reply by mail");
    assert_eq!(changes(&world), ["create"], "a PUT waits for sync");
    assert!(egress(&world).is_empty());
    let shown = agenda(&world);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].calendar, work);
    assert_eq!(shown[0].my_partstat.as_deref(), Some("ACCEPTED"));
    let object = world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        CalendarRepository::new(&connection)
            .held(shown[0].event)
            .await
            .expect("a read")
            .expect("held")
            .object
    });
    assert!(
        String::from_utf8_lossy(&object).contains("PARTSTAT=ACCEPTED"),
        "the object itself says so"
    );

    answer(&world, &client, &events, Command::Undo);
    assert!(
        agenda(&world).is_empty(),
        "undo takes it back before sync sends it"
    );
}
