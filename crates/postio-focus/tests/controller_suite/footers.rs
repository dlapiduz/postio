use crate::search_support::footer_of as footer;
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{Effect, FocusController, Input, Policy, RowFacts, Rows, SurfaceKind};
use postio_model::listing::{MessageSummary, ThreadSummary};
use postio_model::{DraftState, EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::FocusRow;
use postio_ui::scope::FooterView;

struct List {
    states: Vec<Option<DraftState>>,
}

impl List {
    fn of(states: &[Option<DraftState>]) -> Self {
        List {
            states: states.to_vec(),
        }
    }

    fn received(len: usize) -> Self {
        List {
            states: vec![None; len],
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.states.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.row(position).map(|row| RowFacts::of(&row))
    }
    fn position_of(&self, id: MessageId) -> Option<u32> {
        let at = id.get().checked_sub(100)?;
        (at >= 0 && (at as usize) < self.states.len()).then_some(at as u32)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        let state = *self.states.get(position as usize)?;
        let at = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();
        let id = MessageId::new(100 + i64::from(position));
        let thread = ThreadId::new(500 + i64::from(position));
        let subject = Some(format!("Subject {position}"));
        Some(FocusRow::conversation(ThreadSummary {
            id: Some(thread),
            representative: MessageSummary {
                id,
                thread: Some(thread),
                from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
                subject: subject.clone(),
                preview: None,
                received_at: at,
                seen: false,
                flagged: false,
                answered: false,
                send_state: state,
                send_at: None,
                has_attachments: false,
                thread_count: 1,
                to: Vec::new(),
            },
            subject,
            participants: Vec::new(),
            message_count: 1,
            unread_count: 1,
            flagged: false,
            has_attachments: false,
            last_at: at,
            marker: None,
            copies: Vec::new(),
            muted: None,
        }))
    }
}

fn linux() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Freedesktop))
}

fn mac() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

fn labels(footer: &FooterView) -> Vec<&str> {
    footer
        .keys
        .iter()
        .map(|(_, label)| label.as_str())
        .collect()
}

fn landed(focus: &mut FocusController, rows: &List) -> FooterView {
    footer(&focus.landed(rows, true)).expect("the row's footer")
}

#[test]
fn a_lone_message_from_the_inbox_says_its_own_verbs() {
    let rows = List::received(3);
    for mut focus in [linux(), mac()] {
        let row = landed(&mut focus, &rows);
        assert_eq!(
            labels(&row),
            ["move", "open", "reply", "forward", "archive"]
        );
        let effects = run(&mut focus, CommandId::OpenMessage, &rows);
        let window = footer(&effects).expect("the window's footer");
        assert_eq!(window.title, "Subject 0");
        assert_eq!(labels(&window), ["reply", "forward", "archive"]);
        let effects = run(&mut focus, CommandId::Back, &rows);
        assert_eq!(footer(&effects), Some(row), "the row's footer came back");
    }
}

#[test]
fn a_draft_on_its_way_says_the_verbs_that_settle_it() {
    for (state, want) in [
        (DraftState::Queued, &["cancel send", "edit"][..]),
        (DraftState::Failed, &["retry send", "edit"][..]),
        (
            DraftState::Unconfirmed,
            &["retry send", "mark as sent", "edit"][..],
        ),
    ] {
        let rows = List::of(&[Some(state)]);
        let mut focus = linux();
        let _ = landed(&mut focus, &rows);
        let effects = run(&mut focus, CommandId::OpenMessage, &rows);
        assert_eq!(focus.reading(), Some(MessageId::new(100)), "opened to read");
        let window = footer(&effects).expect("the window's footer");
        assert_eq!(window.title, "Subject 0", "{state:?}");
        assert_eq!(labels(&window), want, "{state:?}");
    }
}

#[test]
fn the_composer_draws_no_footer_and_gives_the_rows_back() {
    let rows = List::received(3);
    let mut focus = linux();
    let row = landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::Compose, &rows);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Composer));
    assert_eq!(footer(&effects), Some(FooterView::default()), "no footer");
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Composer), &rows);
    assert_eq!(footer(&effects), Some(row), "the row's footer came back");
}

#[test]
fn the_composer_over_a_window_gives_the_windows_footer_back() {
    let rows = List::received(3);
    let mut focus = linux();
    let _ = landed(&mut focus, &rows);
    let window = footer(&run(&mut focus, CommandId::OpenMessage, &rows)).expect("window");
    let effects = run(&mut focus, CommandId::Reply, &rows);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Composer));
    assert_eq!(footer(&effects), Some(FooterView::default()), "no footer");
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Composer), &rows);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Message));
    assert_eq!(
        footer(&effects),
        Some(window),
        "the window's footer came back"
    );
}

#[test]
fn settings_draws_no_footer() {
    let rows = List::received(3);
    let mut focus = linux();
    let row = landed(&mut focus, &rows);
    let effects = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Settings), &rows);
    assert_eq!(footer(&effects), Some(FooterView::default()), "no footer");
    // A sync landing behind it does not put the list's footer back up.
    let effects = focus.landed(&rows, false);
    assert!(
        footer(&effects).is_none_or(|footer| footer.title.is_empty()),
        "the list's footer came back under Settings: {:?}",
        footer(&effects)
    );
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Settings), &rows);
    assert_eq!(footer(&effects), Some(row), "the row's footer came back");
}

#[test]
fn the_capture_sheet_draws_no_footer() {
    let rows = List::received(3);
    let mut focus = linux();
    let row = landed(&mut focus, &rows);
    let effects = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Capture), &rows);
    assert_eq!(footer(&effects), Some(FooterView::default()), "no footer");
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Capture), &rows);
    assert_eq!(footer(&effects), Some(row), "the row's footer came back");
}

#[test]
fn the_command_bar_draws_no_footer() {
    let rows = List::received(3);
    let mut focus = linux();
    let row = landed(&mut focus, &rows);
    let effects = run(&mut focus, CommandId::CommandPalette, &rows);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Bar));
    assert_eq!(footer(&effects), Some(FooterView::default()), "no footer");
    let effects = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(focus.top_surface(), None, "Escape closed the bar");
    assert_eq!(footer(&effects), Some(row), "the row's footer came back");
}
