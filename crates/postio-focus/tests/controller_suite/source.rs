use crate::search_support::shown;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    Effect, FocusController, Input, Intent, Over, Policy, RowFacts, Rows, SourceVerb, SurfaceKind,
};
use postio_model::{MessageId, ThreadId};
use postio_ui::keymap::KeyContext;
use postio_ui::source::Tab;

struct List(Vec<RowFacts>);

impl List {
    fn of(len: i64) -> Self {
        List(
            (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: false,
                    threads: vec![ThreadId::new(500 + at)],
                    writes: false,
                })
                .collect(),
        )
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.0.get(position as usize).cloned()
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.0
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
}

fn linux() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Freedesktop))
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

/// The frontend opens what it was told to, and says so.
fn follow(focus: &mut FocusController, effects: &[Effect], rows: &List) {
    if shown(effects)
        .iter()
        .any(|intent| matches!(intent, Intent::OpenSource { .. }))
    {
        let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Source), rows);
    }
}

fn on_row_two() -> (FocusController, List) {
    let rows = List::of(4);
    let mut focus = linux();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    (focus, rows)
}

#[test]
fn v_on_a_row_opens_source_on_verification_over_the_list() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::OpenSource {
            message: MessageId::new(102),
            tab: Tab::Verification,
            over: Over::List,
        }]
    );
    follow(&mut focus, &effects, &rows);
    assert_eq!(focus.key_context(), KeyContext::Source);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Source));
}

#[test]
fn digits_switch_the_tabs() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    follow(&mut focus, &effects, &rows);
    for (command, tab) in [
        (CommandId::SourceTabHeaders, Tab::Headers),
        (CommandId::SourceTabRaw, Tab::Raw),
        (CommandId::SourceTabVerification, Tab::Verification),
    ] {
        assert!(focus.answers(command), "{command:?} is the surface's");
        assert_eq!(
            shown(&run(&mut focus, command, &rows)),
            vec![Intent::Source(SourceVerb::Tab(tab))],
            "{command:?}"
        );
    }
}

#[test]
fn esc_closes_the_source_back_to_the_same_row_with_nothing_moved() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    follow(&mut focus, &effects, &rows);
    let _ = run(&mut focus, CommandId::SourceTabRaw, &rows);
    let closed = run(&mut focus, CommandId::Back, &rows);
    let said = shown(&closed);
    assert!(
        said.contains(&Intent::CloseSurface(SurfaceKind::Source)),
        "{said:?}"
    );
    assert!(
        said.iter()
            .all(|intent| !matches!(intent, Intent::Cursor { .. } | Intent::ListToTop)),
        "the cursor and the scroll stay: {said:?}"
    );
    assert_eq!(focus.top_surface(), None);
    assert_eq!(focus.key_context(), KeyContext::List);
    let again = run(&mut focus, CommandId::ViewSource, &rows);
    assert_eq!(
        shown(&again),
        vec![Intent::OpenSource {
            message: MessageId::new(102),
            tab: Tab::Verification,
            over: Over::List,
        }],
        "the same row, and Verification again: one key, one tab"
    );
}

#[test]
fn v_in_the_open_message_stacks_the_source_and_esc_takes_one_rung() {
    let (mut focus, rows) = on_row_two();
    let opened = run(&mut focus, CommandId::OpenMessage, &rows);
    assert!(
        shown(&opened)
            .iter()
            .any(|intent| matches!(intent, Intent::OpenMessage { .. }))
    );
    let _ = focus.handle_on(Input::SurfaceOpened(SurfaceKind::Message), &rows);
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::OpenSource {
            message: MessageId::new(102),
            tab: Tab::Verification,
            over: Over::Message,
        }]
    );
    follow(&mut focus, &effects, &rows);
    let first = run(&mut focus, CommandId::Back, &rows);
    assert_eq!(
        shown(&first),
        vec![Intent::CloseSurface(SurfaceKind::Source)]
    );
    assert_eq!(
        focus.key_context(),
        KeyContext::Reader,
        "the message is still open"
    );
    let second = run(&mut focus, CommandId::Back, &rows);
    assert!(shown(&second).contains(&Intent::CloseSurface(SurfaceKind::Message)));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn a_list_letter_does_nothing_over_the_source() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    follow(&mut focus, &effects, &rows);
    for command in [
        CommandId::Archive,
        CommandId::NextMessage,
        CommandId::Delete,
    ] {
        assert!(
            run(&mut focus, command, &rows).is_empty(),
            "{command:?} reached the row behind the source"
        );
    }
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Source));
}

#[test]
fn copy_save_find_and_unfold_go_to_the_tab_that_has_them() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    follow(&mut focus, &effects, &rows);
    assert_eq!(
        shown(&run(&mut focus, CommandId::CopySource, &rows)),
        vec![Intent::Source(SourceVerb::Copy)]
    );
    assert_eq!(
        shown(&run(&mut focus, CommandId::SaveSource, &rows)),
        vec![Intent::Source(SourceVerb::Save)]
    );
    assert!(
        shown(&run(&mut focus, CommandId::UnfoldPart, &rows)).is_empty(),
        "nothing folds on Verification"
    );
    assert!(
        shown(&run(&mut focus, CommandId::FindInMessage, &rows)).is_empty(),
        "Verification has no find"
    );
    let _ = run(&mut focus, CommandId::SourceTabRaw, &rows);
    assert_eq!(
        shown(&run(&mut focus, CommandId::UnfoldPart, &rows)),
        vec![Intent::Source(SourceVerb::Unfold)]
    );
    assert_eq!(
        shown(&run(&mut focus, CommandId::FindInMessage, &rows)),
        vec![Intent::Source(SourceVerb::Find)]
    );
}

#[test]
fn a_source_the_frontend_closed_is_forgotten() {
    let (mut focus, rows) = on_row_two();
    let effects = run(&mut focus, CommandId::ViewSource, &rows);
    follow(&mut focus, &effects, &rows);
    let _ = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Source), &rows);
    assert_eq!(focus.top_surface(), None);
    assert!(
        !run(&mut focus, CommandId::NextMessage, &rows).is_empty(),
        "the list has its keys back"
    );
}
