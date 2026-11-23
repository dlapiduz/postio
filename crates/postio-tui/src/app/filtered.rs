//! Filtered (`g f`): the controller's view of it, and the keys and clicks
//! that become its inputs.

use postio_core::CommandId;
use postio_focus::{Host, Input as Told, Intent, SurfaceKind};
use postio_ui::filtered::TAB_COMMANDS;
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};
use crate::filtered::Filtered;
use crate::surface::Part;

impl App {
    pub fn filtered(&self) -> Option<&Filtered> {
        self.surfaces.filtered.as_ref()
    }

    pub fn focus_counts(&self) -> Option<postio_client::protocol::FocusCounts> {
        self.counts
    }

    pub fn filtered_height(&self) -> usize {
        usize::from(self.window().list.height.saturating_sub(2))
    }

    pub(super) fn show_filtered_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        let height = self.filtered_height();
        match intent {
            Intent::ShowFiltered => {
                self.surfaces.filtered = Some(Filtered::opening());
                self.focus = Focus::Filtered;
            }
            Intent::Filtered(view) => {
                let shown = self.surfaces.filtered.get_or_insert_with(Filtered::opening);
                shown.replace(*view);
                shown.reveal(height);
            }
            Intent::FilteredFocus(at) => {
                if let Some(shown) = self.surfaces.filtered.as_mut() {
                    shown.focus(at);
                    shown.reveal(height);
                }
            }
            Intent::FilteredSelection { selected, bar } => {
                if let Some(shown) = self.surfaces.filtered.as_mut() {
                    shown.select(&selected, bar);
                }
            }
            // Filtered draws no conversation open inside it.
            Intent::FilteredSplice { .. } | Intent::FilteredUnsplice { .. } => {}
            Intent::CloseSurface(SurfaceKind::Filtered) => {
                self.surfaces.filtered = None;
                self.surfaces.confirm = None;
                if self.focus == Focus::Filtered {
                    self.focus = Focus::List;
                }
            }
            // The message opens in the frame over Filtered, and stepping
            // through it steps the rows.
            Intent::OpenMessage {
                message,
                host: Host::Own,
                ..
            } if self.over_filtered() => {
                let Some(line) = self.surfaces.filtered.as_ref().and_then(|shown| {
                    shown
                        .view()
                        .rows
                        .iter()
                        .find(|line| line.message == message)
                }) else {
                    return Ok(Vec::new());
                };
                let row = crate::row::Row::from(line.summary.clone());
                return Ok(self.open_for_itself(row, Focus::Filtered));
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub(super) fn over_filtered(&self) -> bool {
        self.surfaces.filtered.is_some()
            && match self.focus {
                Focus::Filtered => true,
                Focus::Reader => self
                    .reading
                    .as_ref()
                    .and_then(|reading| reading.own.as_ref())
                    .is_some_and(|own| own.back == Focus::Filtered),
                _ => false,
            }
    }

    pub(super) fn step_filtered_open(&mut self, by: isize) -> Vec<Effect> {
        let command = if by > 0 {
            CommandId::NextMessage
        } else {
            CommandId::PrevMessage
        };
        self.tell(Told::Command(command))
    }

    pub(super) fn filtered_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        let outcome = match self.keys.press(key, KeyContext::Filtered, false) {
            // `F` is the inbox list's key; Filtered's strip offers it too.
            Outcome::Unhandled => match self.keys.press(key, KeyContext::List, false) {
                Outcome::Command(id) if id == "sweep_inbox" => Outcome::Command(id),
                _ => Outcome::Unhandled,
            },
            other => other,
        };
        match outcome {
            Outcome::Command(id) => self.command(&id),
            _ => Vec::new(),
        }
    }

    pub(super) fn filtered_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        match part {
            Part::FilteredTab => match TAB_COMMANDS.get(index) {
                Some(command) => self.tell(Told::Command(*command)),
                None => Vec::new(),
            },
            Part::FilteredRow => self.tell(Told::FilteredPoint(index as u32)),
            _ => Vec::new(),
        }
    }

    pub(super) fn filtered_wheel(&mut self, lines: isize) {
        let height = self.filtered_height();
        if let Some(shown) = self.surfaces.filtered.as_mut() {
            shown.scroll(lines, height);
        }
    }

    pub(super) fn filtered_fetches(&mut self) -> Vec<Effect> {
        let height = self.filtered_height();
        let Some(shown) = self.surfaces.filtered.as_mut() else {
            return Vec::new();
        };
        if !shown.wants_more(height) {
            return Vec::new();
        }
        shown.asked_more();
        self.tell(Told::FilteredMore)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::{Command, MessageTarget};
    use postio_focus::{Reply, Request};
    use postio_model::MessageId;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::test_support::{
        app, conversation, drawing_counts, filtered_row, key, local, now, places_with_features,
        press, screen, seed_places, serve_filtered, serve_focus, show_focus,
    };

    const TABS: [(&str, u32); 3] = [("notification", 88), ("spam", 12), ("promotion", 41)];

    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        app.clock_for_tests(now());
        seed_places(&mut app, places_with_features());
        show_focus(
            &mut app,
            vec![FocusRow::conversation(conversation(
                1,
                "Ada",
                "Hello",
                "",
                local(23, 9, 0),
            ))],
        );
        app.counts_for_tests(drawing_counts());
        app
    }

    fn filed() -> Vec<postio_client::protocol::FilteredRow> {
        vec![
            filtered_row(11, "Forge", "notification", 23, 11, 2),
            filtered_row(12, "Ledger", "notification", 23, 10, 40),
            filtered_row(13, "Promo", "promotion", 23, 9, 5),
            filtered_row(14, "Forge", "notification", 22, 16, 0),
            filtered_row(15, "Rates", "spam", 22, 8, 15),
        ]
    }

    fn open_filtered(size: (u16, u16)) -> App {
        let mut app = inbox(size);
        update(&mut app, press('g'));
        let effects = update(&mut app, press('f'));
        serve_filtered(&mut app, effects, &TABS, &filed());
        app
    }

    fn asked(effects: &[Effect], wanted: impl Fn(&Request) -> bool) -> bool {
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Focus { request, .. } if wanted(request)))
    }

    fn line_with<'a>(screen: &'a str, wanted: &str) -> &'a str {
        screen
            .lines()
            .find(|line| line.contains(wanted))
            .unwrap_or_else(|| panic!("no line holds {wanted:?} in\n{screen}"))
    }

    #[test]
    fn g_f_reads_the_tabs_and_the_first_page_and_takes_the_keyboard() {
        let mut app = inbox((120, 30));
        update(&mut app, press('g'));
        let effects = update(&mut app, press('f'));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::FilteredTabs
        )));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::Filtered {
                reason: None,
                offset: 0,
                ..
            }
        )));
        assert_eq!(app.focus(), Focus::Filtered);
    }

    #[test]
    fn the_strip_the_tabs_and_the_rows_under_their_days_replace_the_inbox() {
        let app = open_filtered((120, 30));
        let drawn = screen(120, 30, &app);
        let strip = drawn.lines().nth(1).unwrap();
        assert!(
            strip.contains(
                "‹ Inbox g i   Filtered · 186 today   Nothing here is deleted automatically"
            ),
            "{strip}"
        );
        assert!(strip.trim_end().ends_with("Sweep the inbox… F"), "{strip}");
        let tabs = drawn.lines().nth(2).unwrap();
        assert!(
            tabs.contains("1 All 141  2 Spam 12  3 Promotions 41  4 Notifications 88"),
            "{tabs}"
        );
        assert!(
            !drawn.contains("Hello"),
            "the inbox's rows are gone:\n{drawn}"
        );
        assert!(
            drawn.contains("Today · Wednesday 23 September · 3"),
            "{drawn}"
        );
        let row = line_with(&drawn, "Filtered 11");
        let pill = row.find("notification · Forge").expect("the pill");
        assert!(pill < row.find("11:02").expect("the time"), "{row}");
        assert!(row.contains('▌'), "the keyboard is on the first row: {row}");
    }

    #[test]
    fn j_walks_the_rows_over_the_day_headings_and_a_tab_key_reads_its_reason() {
        let mut app = open_filtered((120, 30));
        for _ in 0..3 {
            update(&mut app, press('j'));
        }
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Filtered 14").contains('▌'), "{drawn}");
        let effects = update(&mut app, press('4'));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::Filtered { reason: Some(reason), offset: 0, .. } if reason == "notification"
        )));
        serve_filtered(&mut app, effects, &TABS, &filed());
        let drawn = screen(120, 30, &app);
        assert!(
            !drawn.contains("Filtered 13"),
            "only notifications:\n{drawn}"
        );
        let buffer = crate::test_support::buffer(120, 30, &app);
        let tabs: String = (0..120)
            .map(|x| buffer[(x, 2)].symbol().to_owned())
            .collect();
        let at = tabs.find("4 Notifications 88").unwrap();
        let cell = &buffer[(at as u16 + 2, 2)];
        assert!(
            cell.modifier.contains(ratatui::style::Modifier::BOLD)
                && cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
            "the tab on screen is marked by more than a colour: {cell:?}"
        );
    }

    #[test]
    fn r_restores_the_focused_row_through_the_controller() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('j'));
        let effects = update(&mut app, press('R'));
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::Post(Command::RestoreFiltered {
                    target: MessageTarget::Messages(messages),
                    restored: true,
                }) if messages == &[MessageId::new(12)]
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn f_asks_what_a_sweep_would_move_and_return_confirms_it() {
        let mut app = open_filtered((120, 30));
        let effects = update(&mut app, press('F'));
        let effects = serve_focus(&mut app, effects, |request| {
            matches!(request, Request::SweepPreview).then_some(Reply::SweepPreview(Ok(2)))
        });
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Sweep the inbox?"), "{drawn}");
        assert!(drawn.contains("Move 2 to Filtered"), "{drawn}");
        let _ = effects;
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::Post(Command::SweepInbox)
            )),
            "{effects:?}"
        );
        assert!(app.confirm().is_none());
    }

    #[test]
    fn escape_puts_the_sweeps_question_away_and_sends_nothing() {
        let mut app = open_filtered((120, 30));
        let effects = update(&mut app, press('F'));
        serve_focus(&mut app, effects, |request| {
            matches!(request, Request::SweepPreview).then_some(Reply::SweepPreview(Ok(2)))
        });
        assert!(app.confirm().is_some());
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!asked(&effects, |_| true), "{effects:?}");
        assert!(app.confirm().is_none());
        assert_eq!(app.focus(), Focus::Filtered);
    }

    #[test]
    fn a_click_on_a_tab_reads_it_a_row_moves_the_keyboard_and_the_sweeps_buttons_answer() {
        use crate::surface::Part;
        use crate::test_support::click;
        use crate::view::hit::Target;
        let mut app = open_filtered((120, 30));
        let effects = update(
            &mut app,
            click(Target::Surface(Part::FilteredTab, 2), false, false),
        );
        assert!(asked(&effects, |request| matches!(
            request,
            Request::Filtered { reason: Some(reason), .. } if reason == "promotion"
        )));
        serve_filtered(&mut app, effects, &TABS, &filed());
        update(
            &mut app,
            click(Target::Surface(Part::FilteredRow, 0), false, false),
        );
        assert_eq!(app.filtered().unwrap().cursor(), Some(0));
        let effects = update(&mut app, press('F'));
        serve_focus(&mut app, effects, |request| {
            matches!(request, Request::SweepPreview).then_some(Reply::SweepPreview(Ok(2)))
        });
        let effects = update(
            &mut app,
            click(Target::Surface(Part::ConfirmYes, 0), false, false),
        );
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::Post(Command::SweepInbox)
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn what_a_sender_wrote_into_a_name_or_a_subject_never_reaches_the_terminal() {
        let mut app = inbox((120, 30));
        let mut hostile = filtered_row(21, "\u{1b}[31mEvil", "notification", 23, 9, 0);
        hostile.message.subject = Some("\u{1b}]0;owned\u{7}Subject".into());
        update(&mut app, press('g'));
        let effects = update(&mut app, press('f'));
        serve_filtered(&mut app, effects, &TABS, &[hostile]);
        let drawn = screen(120, 30, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("Subject"), "{drawn}");
    }

    #[test]
    fn a_narrow_terminal_keeps_the_tab_on_screen_named_and_drops_the_note_first() {
        let app = open_filtered((70, 24));
        let drawn = screen(70, 24, &app);
        let strip = drawn.lines().nth(1).unwrap();
        assert!(strip.contains("‹ Inbox g i"), "{strip}");
        assert!(strip.contains("Sweep the inbox… F"), "{strip}");
        assert!(!strip.contains("Nothing here"), "{strip}");
        assert!(drawn.lines().nth(2).unwrap().contains("1 All 141"));
    }

    #[test]
    fn g_i_and_escape_go_back_to_the_inbox() {
        for leave in [
            vec![press('g'), press('i')],
            vec![key(KeyCode::Esc, KeyModifiers::NONE)],
        ] {
            let mut app = open_filtered((120, 30));
            for input in leave {
                update(&mut app, input);
            }
            assert_eq!(app.focus(), Focus::List);
            let drawn = screen(120, 30, &app);
            assert!(drawn.contains("Hello"), "the inbox as it was:\n{drawn}");
        }
    }

    #[test]
    fn only_the_rows_in_view_are_read_fifty_at_a_time() {
        let mut app = inbox((120, 20));
        let many: Vec<_> = (0..120)
            .map(|id| filtered_row(100 + id, "Forge", "notification", 23, 11, 2))
            .collect();
        update(&mut app, press('g'));
        let effects = update(&mut app, press('f'));
        let seen = serve_filtered(&mut app, effects, &TABS, &many);
        let pages = seen
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::Focus {
                        request: Request::Filtered { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(pages, 1, "the first page only: the rest is out of view");
        assert_eq!(app.filtered().unwrap().view().rows.len(), 50);
        // Walking to the end of what is read asks for the next fifty.
        let mut more = Vec::new();
        for _ in 0..49 {
            more.extend(update(&mut app, press('j')));
        }
        assert!(
            asked(&more, |request| matches!(
                request,
                Request::Filtered { offset: 50, .. }
            )),
            "{more:?}"
        );
        serve_filtered(&mut app, more, &TABS, &many);
        assert_eq!(app.filtered().unwrap().view().rows.len(), 100);
    }

    #[test]
    fn return_opens_the_message_in_the_frame_over_filtered_and_escape_comes_back_to_its_row() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('j'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(12))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Reader);
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains('╭'), "a frame:\n{drawn}");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Filtered);
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Filtered 12").starts_with('▌'), "{drawn}");
        // The controller took the message off its stack: keys are Filtered's again.
        update(&mut app, press('j'));
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Filtered 13").contains('▌'), "{drawn}");
    }

    #[test]
    fn a_picker_over_the_open_message_is_about_it_and_snoozes_it() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('j'));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut app, press('b'));
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Ledger · Filtered 12"), "{drawn}");
        let effects = update(&mut app, press('2'));
        let aims: Vec<_> = crate::test_support::asked(&effects)
            .into_iter()
            .filter_map(|(_, request)| match request {
                Request::Send {
                    command: Command::Snooze { .. },
                    aims,
                    ..
                } => Some(aims),
                _ => None,
            })
            .collect();
        assert_eq!(
            aims,
            vec![vec![MessageTarget::Messages(vec![MessageId::new(12)])]],
            "{effects:?}"
        );
    }

    #[test]
    fn j_in_the_open_message_steps_through_filtered() {
        let mut app = open_filtered((120, 30));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let effects = update(&mut app, press('j'));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(12))),
            "{effects:?}"
        );
    }

    #[test]
    fn the_list_is_read_again_when_the_mail_moves_under_it() {
        use postio_core::Event;
        let mut app = open_filtered((120, 30));
        update(&mut app, press('j'));
        let effects = update(
            &mut app,
            Input::Host(Event::UndoPerformed {
                description: "Undid".into(),
            }),
        );
        assert!(asked(&effects, |request| matches!(
            request,
            Request::FilteredTabs
        )));
        serve_filtered(&mut app, effects, &TABS, &filed());
        let drawn = screen(120, 30, &app);
        assert!(
            line_with(&drawn, "Filtered 12").contains('▌'),
            "the keyboard stays on its message:\n{drawn}"
        );
    }
}
