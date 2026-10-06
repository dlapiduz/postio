//! Filtered (`g f`) and the sweep's question: what the keys, the clicks and
//! the host's answers do to the view.
//!
//! The state is `crate::filtered`; the drawing is `view::filtered`.

use crossterm::event::KeyEvent;
use postio_core::{Command, MessageTarget};
use postio_ui::filtered;
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus, Tone};
use crate::ask::{Answer, Ask};
use crate::surface::Part;

impl App {
    /// The Filtered view, while it is the window's body.
    pub fn filtered(&self) -> Option<&crate::filtered::Filtered> {
        self.surfaces.filtered.as_ref()
    }

    /// The sweep's question while it is up: how many it would move.
    pub fn sweep(&self) -> Option<u32> {
        self.surfaces.sweep
    }

    /// What the strip counts, once read.
    pub fn focus_counts(&self) -> Option<postio_client::protocol::FocusCounts> {
        self.counts
    }

    /// How many rows Filtered shows at once: the body less the tab line and
    /// the footer.
    pub fn filtered_height(&self) -> usize {
        usize::from(self.window().list.height.saturating_sub(2))
    }

    /// `g f`: Filtered in place of the strip and the list, read from its
    /// first page.
    pub(super) fn go_to_filtered(&mut self) -> Vec<Effect> {
        let mut view = crate::filtered::Filtered::new();
        let asks = view.reload();
        self.surfaces.filtered = Some(view);
        self.focus = Focus::Filtered;
        let mut effects: Vec<Effect> = asks.into_iter().map(Effect::Ask).collect();
        effects.push(Effect::Redraw);
        effects
    }

    /// Back to the inbox, as it was.
    fn leave_filtered(&mut self) -> Vec<Effect> {
        self.surfaces.filtered = None;
        self.surfaces.sweep = None;
        self.focus = Focus::List;
        let mut effects = vec![Effect::Redraw];
        if !matches!(self.scope, Some(postio_model::ListScope::Focus(_))) {
            effects.extend(self.command("go_to_inbox"));
        }
        effects
    }

    /// A key with the keyboard in Filtered.
    pub(super) fn filtered_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let outcome = match self.keys.press(key, KeyContext::Filtered, false) {
            // `F` is the inbox list's key; Filtered's strip offers it too.
            Outcome::Unhandled => match self.keys.press(key, KeyContext::List, false) {
                Outcome::Command(id) if id == "sweep_inbox" => Outcome::Command(id),
                _ => Outcome::Unhandled,
            },
            other => other,
        };
        let Outcome::Command(id) = outcome else {
            return Vec::new();
        };
        self.filtered_command(&id).unwrap_or_else(|| {
            // Whatever else is asked of the window takes it from Filtered.
            let mut effects = self.leave_filtered();
            effects.extend(self.command(&id));
            effects
        })
    }

    /// What Filtered does for `id`, or `None` when it is not Filtered's.
    pub(super) fn filtered_command(&mut self, id: &str) -> Option<Vec<Effect>> {
        self.surfaces.filtered.as_ref()?;
        let height = self.filtered_height();
        let view = self.surfaces.filtered.as_mut()?;
        match id {
            "next_message" => view.step(1),
            "prev_message" => view.step(-1),
            "first_message" => view.go_to(0),
            "last_message" => view.last(),
            "restore_filtered" => {
                let Some(message) = view.focused().map(|item| item.row.id) else {
                    return Some(Vec::new());
                };
                return Some(vec![Effect::Send(Command::RestoreFiltered {
                    target: MessageTarget::Messages(vec![message]),
                    restored: true,
                })]);
            }
            // Reading a filtered message is the open message's, which is
            // aimed at the inbox's rows; Filtered does not open it.
            "open_message" => return Some(Vec::new()),
            "back" | "go_to_inbox" => return Some(self.leave_filtered()),
            "sweep_inbox" => return Some(self.ask_sweep()),
            "quit" | "undo" | "cheat_sheet" => return Some(self.global(id)),
            other => {
                let index = filtered::TAB_COMMANDS
                    .iter()
                    .position(|tab| tab.as_str() == other)?;
                let asks = view.select_tab(index);
                let mut effects: Vec<Effect> = asks.into_iter().map(Effect::Ask).collect();
                effects.push(Effect::Redraw);
                return Some(effects);
            }
        }
        view.reveal(height);
        Some(vec![Effect::Redraw])
    }

    /// `id` as the window runs it, from a surface that leaves the window
    /// where it is.
    fn global(&mut self, id: &str) -> Vec<Effect> {
        match id {
            "cheat_sheet" => self.open_keys(),
            _ => self.send(id),
        }
    }

    /// `F`: say how much of the inbox the rules would file away, and ask.
    pub(super) fn ask_sweep(&mut self) -> Vec<Effect> {
        vec![Effect::Ask(Ask::SweepPreview)]
    }

    /// A key with the sweep's question up: `Return` sweeps, `Esc` does not.
    pub(super) fn sweep_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Outcome::Command(id) = self.keys.press(key, KeyContext::Filtered, false) else {
            return Vec::new();
        };
        match id.as_str() {
            "open_message" => {
                self.surfaces.sweep = None;
                vec![Effect::Send(Command::SweepInbox), Effect::Redraw]
            }
            "back" => {
                self.surfaces.sweep = None;
                vec![Effect::Redraw]
            }
            _ => Vec::new(),
        }
    }

    /// What the host answered a surface.
    pub(super) fn answered(&mut self, answer: Answer) -> Vec<Effect> {
        let height = self.filtered_height();
        match answer {
            Answer::FilteredTabs(Ok(reasons)) => {
                if let Some(view) = self.surfaces.filtered.as_mut() {
                    view.counted(&reasons);
                }
            }
            Answer::FilteredTabs(Err(reason)) => {
                tracing::warn!(%reason, "could not read Filtered's tabs");
            }
            Answer::FilteredPage {
                generation,
                offset,
                rows,
            } => {
                let Some(view) = self.surfaces.filtered.as_mut() else {
                    return Vec::new();
                };
                match rows {
                    Ok(rows) => {
                        if view.landed(generation, offset, rows) {
                            view.reveal(height);
                        }
                    }
                    Err(reason) => {
                        view.failed();
                        return self.say_as(Tone::Failed, &reason, None);
                    }
                }
            }
            Answer::SweepPreview(Ok(0)) => return self.say(filtered::SWEEP_NOTHING),
            Answer::SweepPreview(Ok(count)) => self.surfaces.sweep = Some(count),
            Answer::SweepPreview(Err(reason)) => return self.say_as(Tone::Failed, &reason, None),
        }
        vec![Effect::Redraw]
    }

    /// Filtered reads itself again when the mail moves under it: a restore,
    /// its undo, mail filed away.
    pub(super) fn filtered_hears(&mut self, event: &postio_core::Event) -> Vec<Effect> {
        use postio_core::Event;
        let Some(view) = self.surfaces.filtered.as_mut() else {
            return Vec::new();
        };
        if !matches!(
            event,
            Event::MessageListChanged { .. }
                | Event::UndoPerformed { .. }
                | Event::ActionCompleted { .. }
        ) {
            return Vec::new();
        }
        view.reload().into_iter().map(Effect::Ask).collect()
    }

    /// The next page of Filtered, once its last row read is in view.
    pub(super) fn filtered_fetches(&mut self) -> Vec<Effect> {
        let height = self.filtered_height();
        self.surfaces
            .filtered
            .as_mut()
            .and_then(|view| view.wants_more(height))
            .map(|ask| vec![Effect::Ask(ask)])
            .unwrap_or_default()
    }

    /// A click on a part of Filtered.
    pub(super) fn filtered_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let height = self.filtered_height();
        match part {
            Part::SweepCancel => {
                self.surfaces.sweep = None;
                return vec![Effect::Redraw];
            }
            Part::SweepConfirm => {
                self.surfaces.sweep = None;
                return vec![Effect::Send(Command::SweepInbox), Effect::Redraw];
            }
            _ => {}
        }
        let Some(view) = self.surfaces.filtered.as_mut() else {
            return Vec::new();
        };
        match part {
            Part::FilteredTab => {
                let asks = view.select_tab(index);
                let mut effects: Vec<Effect> = asks.into_iter().map(Effect::Ask).collect();
                effects.push(Effect::Redraw);
                effects
            }
            Part::FilteredRow => {
                view.go_to(index);
                view.reveal(height);
                vec![Effect::Redraw]
            }
            Part::SweepCancel | Part::SweepConfirm => Vec::new(),
        }
    }

    /// The wheel over Filtered's rows.
    pub(super) fn filtered_wheel(&mut self, lines: isize) {
        let height = self.filtered_height();
        if let Some(view) = self.surfaces.filtered.as_mut() {
            view.scroll(lines, height);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::{Command, MessageTarget};
    use postio_model::MessageId;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::ask::{Answer, Ask};
    use crate::test_support::{
        app, conversation, drawing_counts, filtered_row, key, local, places_with_features, press,
        screen, seed_places, serve_filtered, show_focus,
    };

    const TABS: [(&str, u32); 3] = [("notification", 88), ("spam", 12), ("promotion", 41)];

    /// The inbox of the drawing, counted, with Filtered not yet open.
    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
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
        update(&mut app, Input::FocusCounts(drawing_counts()));
        app
    }

    /// Five filed messages: three today, two yesterday.
    fn filed() -> Vec<postio_client::protocol::FilteredRow> {
        vec![
            filtered_row(11, "Forge", "notification", 23, 11, 2),
            filtered_row(12, "Ledger", "notification", 23, 10, 40),
            filtered_row(13, "Promo", "promotion", 23, 9, 5),
            filtered_row(14, "Forge", "notification", 22, 16, 0),
            filtered_row(15, "Rates", "spam", 22, 8, 15),
        ]
    }

    /// Filtered, open and read.
    fn open_filtered(size: (u16, u16)) -> App {
        let mut app = inbox(size);
        update(&mut app, press('g'));
        let effects = update(&mut app, press('f'));
        serve_filtered(&mut app, effects, &TABS, &filed());
        app
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
        assert!(
            effects.contains(&Effect::Ask(Ask::FilteredTabs)),
            "{effects:?}"
        );
        assert!(
            effects.contains(&Effect::Ask(Ask::FilteredPage {
                generation: 1,
                reason: None,
                offset: 0
            })),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Filtered);
    }

    #[test]
    fn the_strip_and_the_tabs_replace_the_inbox_strip() {
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
    }

    #[test]
    fn rows_sit_under_their_day_with_the_reason_pill_before_the_time() {
        let app = open_filtered((120, 30));
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("Today · Wednesday 23 September · 3"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Yesterday · Tuesday 22 September · 2"),
            "{drawn}"
        );
        let row = line_with(&drawn, "Filtered 11");
        let pill = row.find("notification · Forge").expect("the pill");
        let time = row.find("11:02").expect("the time");
        assert!(pill < time, "{row}");
        assert!(row.contains('▌'), "the keyboard is on the first row: {row}");
    }

    #[test]
    fn a_tab_key_reads_that_reason_and_the_tab_on_screen_is_bold_and_underlined() {
        let mut app = open_filtered((120, 30));
        let effects = update(&mut app, press('4'));
        assert!(
            effects.contains(&Effect::Ask(Ask::FilteredPage {
                generation: 2,
                reason: Some("notification".into()),
                offset: 0
            })),
            "{effects:?}"
        );
        serve_filtered(&mut app, effects, &TABS, &filed());
        let drawn = screen(120, 30, &app);
        assert!(
            !drawn.contains("Filtered 13"),
            "only notifications:\n{drawn}"
        );
        assert!(drawn.contains("Filtered 14"), "{drawn}");
        // The tab on screen is marked by more than a colour.
        let buffer = crate::test_support::buffer(120, 30, &app);
        let tabs: String = (0..120)
            .map(|x| buffer[(x, 2)].symbol().to_owned())
            .collect();
        let at = tabs.find("4 Notifications 88").unwrap();
        let cell = &buffer[(at as u16 + 2, 2)];
        assert!(
            cell.modifier.contains(ratatui::style::Modifier::BOLD)
                && cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
            "{cell:?}"
        );
        let other = &buffer[(0, 2)];
        assert!(
            !other
                .modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }

    #[test]
    fn j_and_k_walk_the_rows_over_the_day_headings() {
        let mut app = open_filtered((120, 30));
        for _ in 0..3 {
            update(&mut app, press('j'));
        }
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Filtered 14").contains('▌'), "{drawn}");
        update(&mut app, press('k'));
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Filtered 13").contains('▌'), "{drawn}");
    }

    #[test]
    fn r_restores_the_focused_row_and_the_hosts_words_are_the_toast() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('j'));
        let effects = update(&mut app, press('R'));
        assert!(
            effects.contains(&Effect::Send(Command::RestoreFiltered {
                target: MessageTarget::Messages(vec![MessageId::new(12)]),
                restored: true,
            })),
            "{effects:?}"
        );
        // The host says what it did; Filtered reads itself again.
        let effects = update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Restored 1 message".into(),
                undoable: true,
            }),
        );
        assert!(
            effects.contains(&Effect::Ask(Ask::FilteredTabs)),
            "{effects:?}"
        );
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("✓ Restored 1 message · Undo ctrl+z"),
            "{drawn}"
        );
    }

    #[test]
    fn the_list_is_read_again_when_the_mail_moves_under_it() {
        use postio_core::Event;
        for event in [
            Event::MessageListChanged {
                account: postio_model::AccountId::new(1),
                mailbox: postio_model::MailboxId::new(1),
            },
            Event::UndoPerformed {
                description: "Undid".into(),
            },
        ] {
            let mut app = open_filtered((120, 30));
            update(&mut app, press('j'));
            let effects = update(&mut app, Input::Host(event));
            assert!(
                effects.contains(&Effect::Ask(Ask::FilteredTabs)),
                "{effects:?}"
            );
            let reread = serve_filtered(&mut app, effects, &TABS, &filed());
            assert!(reread >= 2);
            let drawn = screen(120, 30, &app);
            assert!(
                line_with(&drawn, "Filtered 12").contains('▌'),
                "the keyboard stays on its message:\n{drawn}"
            );
        }
    }

    #[test]
    fn f_shows_what_a_sweep_would_move_and_return_confirms_it() {
        let mut app = open_filtered((120, 30));
        let effects = update(&mut app, press('F'));
        assert_eq!(effects, vec![Effect::Ask(Ask::SweepPreview)]);
        update(&mut app, Input::Answer(Answer::SweepPreview(Ok(2))));
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Sweep the inbox?"), "{drawn}");
        assert!(
            drawn.contains("2 messages in the inbox would move to Filtered, each with its reason."),
            "{drawn}"
        );
        assert!(drawn.contains("One ctrl+z puts them back."), "{drawn}");
        assert!(drawn.contains("Move 2 to Filtered"), "{drawn}");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Send(Command::SweepInbox)),
            "{effects:?}"
        );
        assert!(app.sweep().is_none());
    }

    #[test]
    fn escape_puts_the_sweeps_question_away_and_sends_nothing() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('F'));
        update(&mut app, Input::Answer(Answer::SweepPreview(Ok(2))));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "{effects:?}"
        );
        assert!(app.sweep().is_none());
        assert_eq!(app.focus(), Focus::Filtered, "still in Filtered");
    }

    #[test]
    fn a_sweep_that_would_move_nothing_says_so_instead_of_asking() {
        let mut app = open_filtered((120, 30));
        update(&mut app, press('F'));
        update(&mut app, Input::Answer(Answer::SweepPreview(Ok(0))));
        assert!(app.sweep().is_none());
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("Nothing in the inbox would be filtered"),
            "{drawn}"
        );
    }

    #[test]
    fn f_asks_the_same_from_the_inbox() {
        let mut app = inbox((120, 30));
        let effects = update(&mut app, press('F'));
        assert_eq!(effects, vec![Effect::Ask(Ask::SweepPreview)]);
    }

    #[test]
    fn return_on_a_row_opens_nothing_and_leaves_the_view_where_it_is() {
        let mut app = open_filtered((120, 30));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(app.focus(), Focus::Filtered);
    }

    #[test]
    fn a_click_on_a_tab_reads_it_and_on_a_row_moves_the_keyboard() {
        use crate::surface::Part;
        use crate::test_support::{click, hits_of};
        use crate::view::hit::Target;
        let mut app = open_filtered((120, 30));
        let hits = hits_of(120, 30, &app);
        // The tab line is the row under the strip: find its third tab.
        let tab = (0..120)
            .find_map(|x| {
                hits.at(x, 2)
                    .filter(|hit| hit.target == Target::Surface(Part::FilteredTab, 2))
            })
            .expect("a click target for the Promotions tab");
        let _ = tab;
        let effects = update(
            &mut app,
            click(Target::Surface(Part::FilteredTab, 2), false, false),
        );
        assert!(
            effects.contains(&Effect::Ask(Ask::FilteredPage {
                generation: 2,
                reason: Some("promotion".into()),
                offset: 0
            })),
            "{effects:?}"
        );
        serve_filtered(&mut app, effects, &TABS, &filed());
        update(
            &mut app,
            click(Target::Surface(Part::FilteredRow, 0), false, false),
        );
        assert_eq!(app.filtered().unwrap().cursor(), 0);
    }

    #[test]
    fn the_strips_back_and_sweep_and_the_footers_restore_are_clicks() {
        use crate::test_support::hits_of;
        use crate::view::hit::Target;
        let app = open_filtered((120, 30));
        let hits = hits_of(120, 30, &app);
        let on_row = |y: u16, wanted: Target| {
            (0..120).any(|x| hits.at(x, y).is_some_and(|hit| hit.target == wanted))
        };
        assert!(on_row(1, Target::Command("go_to_inbox")));
        assert!(on_row(1, Target::Command("sweep_inbox")));
        assert!(
            on_row(29 - 1, Target::Command("restore_filtered")),
            "the footer's R"
        );
    }

    #[test]
    fn the_footer_names_what_the_keys_do_without_offering_open() {
        let app = open_filtered((120, 30));
        let drawn = screen(120, 30, &app);
        let footer = drawn.lines().nth(28).unwrap();
        assert!(
            footer.contains("R restore + never filter sender"),
            "{footer}"
        );
        assert!(footer.contains("1–7 reason tabs"), "{footer}");
        assert!(footer.contains("g i inbox"), "{footer}");
        assert!(!footer.contains("open"), "{footer}");
    }

    #[test]
    fn a_sweeps_buttons_are_clicks() {
        use crate::surface::Part;
        use crate::test_support::click;
        use crate::view::hit::Target;
        let mut app = open_filtered((120, 30));
        update(&mut app, press('F'));
        update(&mut app, Input::Answer(Answer::SweepPreview(Ok(2))));
        let effects = update(
            &mut app,
            click(Target::Surface(Part::SweepConfirm, 0), false, false),
        );
        assert!(
            effects.contains(&Effect::Send(Command::SweepInbox)),
            "{effects:?}"
        );
        update(&mut app, press('F'));
        update(&mut app, Input::Answer(Answer::SweepPreview(Ok(2))));
        let effects = update(
            &mut app,
            click(Target::Surface(Part::SweepCancel, 0), false, false),
        );
        assert!(!effects.iter().any(|e| matches!(e, Effect::Send(_))));
        assert!(app.sweep().is_none());
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
        let tabs = drawn.lines().nth(2).unwrap();
        assert!(tabs.contains("1 All 141"), "{tabs}");
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
            assert!(
                drawn.lines().nth(1).unwrap().contains("186 filtered today"),
                "{drawn}"
            );
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
        let asked = serve_filtered(&mut app, effects, &TABS, &many);
        assert_eq!(asked, 2, "the tabs and one page: the rest is out of view");
        assert_eq!(app.filtered().unwrap().items().len(), 50);
        // Walking to the end of what is read asks for the next fifty.
        let mut more = Vec::new();
        for _ in 0..49 {
            more.extend(update(&mut app, press('j')));
        }
        assert!(
            more.contains(&Effect::Ask(Ask::FilteredPage {
                generation: 1,
                reason: None,
                offset: 50
            })),
            "{more:?}"
        );
        serve_filtered(&mut app, more, &TABS, &many);
        assert_eq!(app.filtered().unwrap().items().len(), 100);
    }

    #[test]
    fn a_failed_read_is_said() {
        let mut app = inbox((120, 30));
        update(&mut app, press('g'));
        update(&mut app, press('f'));
        update(
            &mut app,
            Input::Answer(Answer::FilteredPage {
                generation: 1,
                offset: 0,
                rows: Err("The store is busy".into()),
            }),
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("✕ The store is busy"), "{drawn}");
    }
}
