use crate::search_support::{asks, shown};
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    CalendarView, Effect, FocusController, Input, Intent, NoRows, Placement, Policy, Reply,
    Request, SurfaceKind,
};
use postio_model::calendar::{AgendaAnchor, AgendaEntry, EditScope, EventDetail, EventStatus};
use postio_model::ids::{CalendarId, EventId};
use postio_ui::calendar::AgendaRow;
use postio_ui::calendar::editor::EditorField;
use postio_ui::keymap::KeyContext;

fn local(day: u32, hour: u32) -> DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 11, day, hour, 0, 0)
        .earliest()
        .expect("a local time")
        .to_utc()
}

fn linux() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
    let _ = focus.handle(Input::Keymap(postio_core::Keymap::resolve_on(
        &postio_config::KeyBindings::default(),
        Platform::Freedesktop,
    )));
    let noon = Local
        .with_ymd_and_hms(2026, 11, 24, 12, 0, 0)
        .earliest()
        .unwrap();
    let _ = focus.handle(Input::Clock(Some(noon)));
    focus
}

fn entry(id: i64, start: DateTime<Utc>, title: &str) -> AgendaEntry {
    AgendaEntry {
        event: EventId::new(id),
        calendar: CalendarId::new(1),
        calendar_name: "On this computer".to_owned(),
        colour: None,
        title: title.to_owned(),
        location: String::new(),
        start,
        end: start + Duration::minutes(30),
        all_day: false,
        floating: false,
        recurrence_id: None,
        status: EventStatus::Confirmed,
        my_partstat: None,
        queued: false,
    }
}

fn run(focus: &mut FocusController, command: CommandId) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), &NoRows)
}

fn frame(effects: &[Effect]) -> Option<CalendarView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Calendar(view) => Some(*view),
            _ => None,
        })
}

fn ringed(view: &CalendarView) -> String {
    match &view.rows[view.cursor.expect("a cursor")] {
        AgendaRow::Occurrence(row) => row.title.clone(),
        AgendaRow::Day { label, .. } => panic!("the ring is on a heading, {label}"),
    }
}

fn page_asked(effects: &[Effect]) -> (postio_focus::Ticket, AgendaAnchor, u64, Placement) {
    asks(effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::AgendaPage {
                anchor,
                stamp,
                placement,
                ..
            } => Some((ticket, anchor, stamp, placement)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a page is asked for: {effects:?}"))
}

fn answer(focus: &mut FocusController, effects: &[Effect], page: Vec<AgendaEntry>) -> Vec<Effect> {
    let (ticket, _, stamp, placement) = page_asked(effects);
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::AgendaPage {
                stamp,
                placement,
                answer: Ok(page),
            },
        ),
        &NoRows,
    )
}

fn today() -> Vec<AgendaEntry> {
    vec![
        entry(1, local(24, 9), "Standup"),
        entry(2, local(24, 14), "Review"),
        entry(3, local(25, 9), "Standup"),
        entry(4, local(26, 11), "Dentist"),
    ]
}

fn opened() -> (FocusController, CalendarView) {
    let mut focus = linux();
    let asked = run(&mut focus, CommandId::GoToCalendar);
    let view = frame(&answer(&mut focus, &asked, today())).expect("drawn");
    (focus, view)
}

#[test]
fn g_e_opens_agenda_on_today_with_the_ring_on_the_next_event_not_over() {
    let mut focus = linux();
    let asked = run(&mut focus, CommandId::GoToCalendar);
    let (_, anchor, _, placement) = page_asked(&asked);

    assert_eq!(anchor, AgendaAnchor::From(local(24, 0)));
    assert_eq!(placement, Placement::Replace);
    assert_eq!(focus.key_context(), KeyContext::Calendar);

    let view = frame(&answer(&mut focus, &asked, today())).expect("drawn");
    assert_eq!(ringed(&view), "Review", "Standup at 09:00 is over by noon");
}

#[test]
fn j_and_k_walk_the_occurrences_and_pass_over_headings() {
    let (mut focus, _) = opened();

    let view = frame(&run(&mut focus, CommandId::NextMessage)).expect("redrawn");
    assert_eq!(ringed(&view), "Standup");
    assert!(matches!(
        view.rows[view.cursor.unwrap() - 1],
        AgendaRow::Day { .. }
    ));
    let view = frame(&run(&mut focus, CommandId::PrevMessage)).expect("redrawn");
    assert_eq!(ringed(&view), "Review");
}

#[test]
fn the_walk_near_the_end_asks_for_the_next_page_and_moves_on_into_it() {
    let mut focus = linux();
    let asked = run(&mut focus, CommandId::GoToCalendar);
    let first: Vec<AgendaEntry> = (0..50)
        .map(|n| entry(n, local(24, 13) + Duration::hours(n), "Slot"))
        .collect();
    let mut drawn = answer(&mut focus, &asked, first);
    for _ in 0..45 {
        drawn = run(&mut focus, CommandId::NextMessage);
        if asks(&drawn).iter().any(|(_, request)| {
            matches!(
                request,
                Request::AgendaPage {
                    placement: Placement::After,
                    ..
                }
            )
        }) {
            break;
        }
    }
    let (_, anchor, _, placement) = page_asked(&drawn);
    assert_eq!(placement, Placement::After);
    assert!(matches!(anchor, AgendaAnchor::After(_)));

    let more = (50..60)
        .map(|n| entry(n, local(24, 13) + Duration::hours(n), "Later"))
        .collect();
    let _ = answer(&mut focus, &drawn, more);
    let view = frame(&run(&mut focus, CommandId::LastMessage)).expect("redrawn");
    assert_eq!(ringed(&view), "Later");
}

#[test]
fn brackets_move_a_week_and_t_comes_back_to_today() {
    let (mut focus, _) = opened();

    let (_, anchor, _, _) = page_asked(&run(&mut focus, CommandId::CalendarNext));
    let december = Local
        .with_ymd_and_hms(2026, 12, 1, 0, 0, 0)
        .earliest()
        .unwrap();
    assert_eq!(anchor, AgendaAnchor::From(december.to_utc()));
    let (_, anchor, _, _) = page_asked(&run(&mut focus, CommandId::CalendarToday));
    assert_eq!(anchor, AgendaAnchor::From(local(24, 0)));
}

#[test]
fn return_opens_the_event_and_escape_climbs_back_to_the_inbox() {
    let (mut focus, view) = opened();
    let ringed_before = ringed(&view);

    let asked = run(&mut focus, CommandId::OpenMessage);
    let [(ticket, Request::EventDetail { cursor, stamp })] = asks(&asked)[..] else {
        panic!("the event is read: {asked:?}");
    };
    assert_eq!(cursor.event, EventId::new(2));
    let detail = EventDetail {
        entry: entry(2, local(24, 14), "Review"),
        notes: "Bring the numbers.".to_owned(),
        organiser: None,
        attendees: Vec::new(),
        zone: None,
        repeat: None,
        minutes_before: Vec::new(),
        writable: true,
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::EventDetail {
                stamp,
                answer: Ok(Some(Box::new(detail))),
            },
        ),
        &NoRows,
    );
    let open = frame(&effects)
        .expect("redrawn")
        .open
        .expect("the event is open");
    assert_eq!(
        (open.title.as_str(), open.notes.as_str()),
        ("Review", "Bring the numbers.")
    );

    let view = frame(&run(&mut focus, CommandId::Back)).expect("redrawn");
    assert!(view.open.is_none());
    assert_eq!(ringed(&view), ringed_before);

    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Calendar)));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn a_mail_destination_leaves_calendar_for_it() {
    let (mut focus, _) = opened();

    let effects = run(&mut focus, CommandId::GoToInbox);

    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Calendar)));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn an_empty_range_says_so() {
    let mut focus = linux();
    let asked = run(&mut focus, CommandId::GoToCalendar);

    let view = frame(&answer(&mut focus, &asked, Vec::new())).expect("drawn");

    assert_eq!(view.empty.as_deref(), Some("Nothing on these weeks."));
    assert_eq!(view.cursor, None);
}

#[test]
fn slash_searches_events_names_what_it_ignored_and_escape_brings_the_agenda_back() {
    let (mut focus, _) = opened();

    assert!(shown(&run(&mut focus, CommandId::Search)).contains(&Intent::CalendarFocusFilter));
    let asked = focus.handle_on(
        Input::CalendarFilterTyped("dentist has:attachment".to_owned()),
        &NoRows,
    );
    let asks = asks(&asked);
    let [(ticket, Request::EventSearch { text, stamp, .. })] = &asks[..] else {
        panic!("the search is asked: {asked:?}");
    };
    let (ticket, stamp) = (*ticket, *stamp);
    assert_eq!(text, "dentist has:attachment");
    let found = frame(&focus.handle_on(
        Input::Reply(
            ticket,
            Reply::EventSearch {
                stamp,
                answer: Ok((
                    vec![entry(4, local(26, 11), "Dentist")],
                    vec!["has".to_owned()],
                )),
            },
        ),
        &NoRows,
    ))
    .expect("redrawn");
    assert_eq!(ringed(&found), "Dentist");
    assert_eq!(found.filter, "dentist has:attachment");
    assert_eq!(
        found.hint.as_deref(),
        Some("has: means nothing for an event, so it narrows nothing")
    );

    let back = frame(&run(&mut focus, CommandId::Back)).expect("redrawn");
    assert_eq!(back.filter, "");
    assert_eq!(ringed(&back), "Review", "the agenda is back where it was");
    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Calendar)));
}

#[test]
fn pop_out_moves_calendar_whole_into_its_own_window_and_pop_in_brings_it_back() {
    let (mut focus, _) = opened();
    let _ = run(&mut focus, CommandId::NextMessage);

    let out = run(&mut focus, CommandId::PopOut);
    let shown_out = shown(&out);
    assert!(shown_out.contains(&Intent::CalendarWindow(true)));
    assert!(shown_out.contains(&Intent::CloseSurface(SurfaceKind::Calendar)));
    assert!(shown_out.contains(&Intent::PresentWindow(postio_focus::Window::Calendar)));
    let view = frame(&out).expect("drawn in its window");
    assert!(view.popped);
    assert_eq!(ringed(&view), "Standup", "the ring came along");
    assert_eq!(focus.key_context(), KeyContext::CalendarWindow);

    let back = run(&mut focus, CommandId::PopIn);
    assert!(shown(&back).contains(&Intent::CalendarWindow(false)));
    let view = frame(&back).expect("drawn in the main window");
    assert!(!view.popped);
    assert_eq!(ringed(&view), "Standup");
    assert_eq!(focus.key_context(), KeyContext::Calendar);
}

#[test]
fn escape_at_the_calendar_window_s_root_closes_nothing() {
    let (mut focus, _) = opened();
    let _ = run(&mut focus, CommandId::PopOut);

    let effects = run(&mut focus, CommandId::Back);

    assert!(shown(&effects).is_empty(), "{effects:?}");
    assert_eq!(focus.key_context(), KeyContext::CalendarWindow);
}

#[test]
fn destinations_go_to_the_window_that_shows_them() {
    let (mut focus, _) = opened();
    let _ = run(&mut focus, CommandId::PopOut);

    let effects = run(&mut focus, CommandId::GoToInbox);
    assert!(shown(&effects).contains(&Intent::PresentWindow(postio_focus::Window::Main)));
    assert_eq!(
        focus.key_context(),
        KeyContext::List,
        "the main window has the keyboard"
    );

    let effects = run(&mut focus, CommandId::GoToCalendar);
    assert_eq!(
        shown(&effects),
        [Intent::PresentWindow(postio_focus::Window::Calendar)]
    );
    assert_eq!(focus.key_context(), KeyContext::CalendarWindow);
}

#[test]
fn close_window_pops_calendar_in_closes_a_detached_composer_and_quits_the_main_window() {
    let (mut focus, _) = opened();
    let _ = run(&mut focus, CommandId::PopOut);
    let effects = run(&mut focus, CommandId::CloseWindow);
    assert!(shown(&effects).contains(&Intent::CalendarWindow(false)));
    assert!(!shown(&effects).contains(&Intent::Quit));

    let _ = run(&mut focus, CommandId::Back);
    let effects = run(&mut focus, CommandId::CloseWindow);
    assert_eq!(shown(&effects), [Intent::Quit], "the main window's X quits");
}

#[test]
fn close_window_in_a_detached_composer_closes_that_composer() {
    let mut focus = linux();
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Composer), &NoRows);
    let _ = focus.handle_on(
        Input::WindowFocused(postio_focus::Window::Composer),
        &NoRows,
    );

    let effects = run(&mut focus, CommandId::CloseWindow);

    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Composer)));
    assert!(!shown(&effects).contains(&Intent::Quit));
}

fn posted(effects: &[Effect]) -> Vec<postio_core::Command> {
    asks(effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Post(command) => Some(command),
            _ => None,
        })
        .collect()
}

fn typed(focus: &mut FocusController, field: EditorField, text: &str) -> CalendarView {
    let effects = focus.handle_on(
        Input::EventEditorTyped {
            field,
            text: text.to_owned(),
        },
        &NoRows,
    );
    frame(&effects).expect("redrawn")
}

fn detail_of(item: AgendaEntry, repeat: bool) -> EventDetail {
    EventDetail {
        entry: item,
        notes: "Bring the numbers.".to_owned(),
        organiser: None,
        attendees: Vec::new(),
        zone: None,
        repeat: repeat.then(|| postio_model::calendar::Repeat {
            frequency: postio_model::calendar::Frequency::Weekly,
            interval: 1,
            weekdays: Vec::new(),
            until: None,
            count: None,
        }),
        minutes_before: Vec::new(),
        writable: true,
    }
}

fn opened_on(focus: &mut FocusController, detail: EventDetail) {
    let asked = run(focus, CommandId::OpenMessage);
    let [(ticket, Request::EventDetail { stamp, .. })] = asks(&asked)[..] else {
        panic!("the event is read: {asked:?}");
    };
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::EventDetail {
                stamp,
                answer: Ok(Some(Box::new(detail))),
            },
        ),
        &NoRows,
    );
}

#[test]
fn c_opens_a_new_event_at_the_next_hour_and_save_posts_it_and_follows_it() {
    let (mut focus, _) = opened();

    let view = frame(&run(&mut focus, CommandId::NewEvent)).expect("redrawn");
    let editor = view.editor.expect("the editor is open");
    assert_eq!(editor.heading, "New event");
    assert_eq!(
        editor.when.as_deref(),
        Ok("Tue 24 Nov, 13:00 \u{2013} 14:00")
    );
    assert_eq!(editor.scope, None);

    let view = frame(&run(&mut focus, CommandId::SaveEvent)).expect("redrawn");
    assert_eq!(
        view.editor.and_then(|editor| editor.error).as_deref(),
        Some("Give the event a title"),
        "nothing is posted without a title"
    );

    typed(&mut focus, EditorField::Title, "Dentist");
    let view = typed(&mut focus, EditorField::Ends, "until 2:30pm");
    assert_eq!(
        view.editor.expect("open").when.as_deref(),
        Ok("Tue 24 Nov, 13:00 \u{2013} 14:30")
    );
    let effects = run(&mut focus, CommandId::SaveEvent);
    let [
        postio_core::Command::SaveEvent {
            event: None,
            fields: Some(fields),
            scope: None,
            ..
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one save: {effects:?}");
    };
    assert_eq!(
        (fields.title.as_str(), fields.start),
        ("Dentist", local(24, 13))
    );
    assert_eq!(fields.end - fields.start, Duration::minutes(90));
    assert!(frame(&effects).expect("redrawn").editor.is_none());

    let heard = focus.handle_on(Input::Event(postio_core::Event::CalendarChanged), &NoRows);
    let mut with_new = today();
    with_new.insert(1, entry(9, local(24, 13), "Dentist"));
    let view = frame(&answer(&mut focus, &heard, with_new)).expect("redrawn");
    assert_eq!(ringed(&view), "Dentist", "the ring follows what was saved");
}

#[test]
fn an_unreadable_start_says_so_and_escape_leaves_without_saving() {
    let (mut focus, _) = opened();
    run(&mut focus, CommandId::NewEvent);
    typed(&mut focus, EditorField::Title, "Planning");

    let view = typed(&mut focus, EditorField::Starts, "whenever");
    assert_eq!(
        view.editor.expect("open").when,
        Err("Postio cannot read \u{201c}whenever\u{201d} as a start".to_owned())
    );
    let effects = run(&mut focus, CommandId::SaveEvent);
    assert!(posted(&effects).is_empty());

    let effects = run(&mut focus, CommandId::Back);
    assert!(posted(&effects).is_empty());
    let view = frame(&effects).expect("redrawn");
    assert!(view.editor.is_none());
    assert_eq!(ringed(&view), "Review", "the agenda is where it was");
}

#[test]
fn mod_e_edits_the_open_event_keeping_its_time_and_a_series_asks_which() {
    let (mut focus, _) = opened();
    opened_on(
        &mut focus,
        detail_of(entry(2, local(24, 14), "Review"), true),
    );

    let view = frame(&run(&mut focus, CommandId::EditEvent)).expect("redrawn");
    let editor = view.editor.expect("the editor is open");
    assert_eq!(editor.heading, "Edit event");
    assert_eq!(editor.scope, Some(EditScope::This));
    assert_eq!(
        editor.when.as_deref(),
        Ok("Tue 24 Nov, 14:00 \u{2013} 14:30")
    );

    focus.handle_on(Input::EventEditorScope(EditScope::All), &NoRows);
    typed(&mut focus, EditorField::Title, "Review (all hands)");
    let effects = run(&mut focus, CommandId::SaveEvent);
    let [
        postio_core::Command::SaveEvent {
            event: Some(target),
            fields: Some(fields),
            scope,
            ..
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one save: {effects:?}");
    };
    assert_eq!(target.occurrence.event, EventId::new(2));
    assert_eq!(*scope, Some(EditScope::All));
    assert_eq!(fields.title, "Review (all hands)");
    assert_eq!(
        (fields.start, fields.end),
        (local(24, 14), local(24, 14) + Duration::minutes(30))
    );
}

#[test]
fn save_and_invite_is_offered_only_with_people() {
    let (mut focus, _) = opened();
    run(&mut focus, CommandId::NewEvent);
    let view = typed(&mut focus, EditorField::Title, "Planning");
    let labels = |view: &CalendarView| -> Vec<String> {
        view.editor
            .as_ref()
            .expect("open")
            .keys
            .iter()
            .map(|hint| hint.label.clone())
            .collect()
    };
    assert_eq!(labels(&view), ["save", "cancel"]);
    assert!(posted(&run(&mut focus, CommandId::SaveAndSendEvent)).is_empty());

    let view = typed(&mut focus, EditorField::People, "Quinn <quinn@example.net>");
    assert_eq!(labels(&view), ["save", "save and invite", "cancel"]);
    let effects = run(&mut focus, CommandId::SaveAndSendEvent);
    let [
        postio_core::Command::SaveAndSendEvent {
            fields: Some(fields),
            ..
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one save and send: {effects:?}");
    };
    assert_eq!(fields.attendees[0].address, "quinn@example.net");
}

#[test]
fn v_moves_the_ringed_event_to_next_week_at_its_own_time() {
    let (mut focus, view) = opened();
    assert_eq!(ringed(&view), "Review");

    let view = frame(&run(&mut focus, CommandId::MoveEvent)).expect("redrawn");
    let mover = view.editor.expect("the move prompt");
    assert_eq!(mover.heading, "Move to\u{2026}");
    assert_eq!(mover.fields.len(), 1);

    let view = typed(&mut focus, EditorField::Starts, "next week");
    assert_eq!(
        view.editor.expect("open").when.as_deref(),
        Ok("Tue 1 Dec, 14:00 \u{2013} 14:30")
    );
    let effects = run(&mut focus, CommandId::OpenMessage);
    let [
        postio_core::Command::MoveEventTo {
            event,
            start,
            scope: None,
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one move: {effects:?}");
    };
    assert_eq!(event.occurrence.event, EventId::new(2));
    assert_eq!(*start, local(24, 14) + Duration::days(7));
}

#[test]
fn hash_deletes_a_single_event_at_once_and_asks_which_for_a_series() {
    let (mut focus, _) = opened();
    let effects = run(&mut focus, CommandId::Delete);
    let [postio_core::Command::DeleteEvent { event, scope: None }] = &posted(&effects)[..] else {
        panic!("one delete: {effects:?}");
    };
    assert_eq!(event.occurrence.event, EventId::new(2));

    let mut focus = linux();
    let asked = run(&mut focus, CommandId::GoToCalendar);
    let mut series = today();
    series[1].recurrence_id = Some("20261124T140000".to_owned());
    answer(&mut focus, &asked, series);
    let effects = run(&mut focus, CommandId::Delete);
    assert!(posted(&effects).is_empty(), "a series asks first");
    let picker = frame(&effects)
        .and_then(|view| view.editor)
        .expect("the picker");
    assert_eq!(picker.heading, "Delete event");
    assert_eq!(picker.scope, Some(EditScope::This));

    focus.handle_on(
        Input::EventEditorScope(EditScope::ThisAndFollowing),
        &NoRows,
    );
    let effects = run(&mut focus, CommandId::Delete);
    let [postio_core::Command::DeleteEvent { event, scope }] = &posted(&effects)[..] else {
        panic!("one delete: {effects:?}");
    };
    assert_eq!(event.recurrence_id.as_deref(), Some("20261124T140000"));
    assert_eq!(*scope, Some(EditScope::ThisAndFollowing));
}

#[test]
fn duplicate_posts_a_copy_of_the_ringed_event() {
    let (mut focus, _) = opened();
    let effects = run(&mut focus, CommandId::DuplicateEvent);
    assert_eq!(
        posted(&effects),
        [postio_core::Command::DuplicateEventOf {
            event: EventId::new(2)
        }]
    );
}

#[test]
fn undo_in_calendar_reaches_the_undo_stack() {
    let (mut focus, _) = opened();
    let effects = run(&mut focus, CommandId::Undo);
    assert_eq!(posted(&effects), [postio_core::Command::Undo]);
}
