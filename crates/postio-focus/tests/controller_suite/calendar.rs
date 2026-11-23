use crate::search_support::{asks, shown};
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    CalendarView, Effect, FocusController, Input, Intent, NoRows, Placement, Policy, Reply,
    Request, SurfaceKind,
};
use postio_model::calendar::{AgendaAnchor, AgendaEntry, EventDetail, EventStatus};
use postio_model::ids::{CalendarId, EventId};
use postio_ui::calendar::AgendaRow;
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
