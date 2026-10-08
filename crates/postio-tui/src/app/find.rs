//! Find in the open message: the field at the bottom of the frame, its
//! matches over the rendered lines, and stepping between them.
//!
//! Matches are never stored: they are computed from the lines the message
//! draws (spec 006 FR-018, `postio_ui::find`), so they follow a fold opened,
//! a resize and a step through the thread. What is kept is the query and
//! which match the field is on.

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use postio_ui::find::{Match, count_label, first_from, matches, step};
use postio_ui::keymap::Outcome;
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus};
use crate::view::open as draw;

/// The find field and where it is.
#[derive(Debug, Default)]
pub struct Find {
    input: tui_input::Input,
    current: Option<usize>,
}

impl Find {
    /// What is typed.
    pub fn query(&self) -> &str {
        self.input.value()
    }

    /// Which match the field is on, from 0.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// Where the caret is, in characters.
    pub fn caret(&self) -> usize {
        self.input.visual_cursor()
    }

    /// What the field says of `total` matches.
    pub fn says(&self, total: usize) -> String {
        count_label(self.query(), self.current, total)
    }
}

impl App {
    /// The find field, while it is open over the message.
    pub fn find(&self) -> Option<&Find> {
        self.open.find.as_ref()
    }

    /// The rows the find field takes from the message's column.
    pub fn find_rows(&self) -> u16 {
        u16::from(self.open.find.is_some())
    }

    /// The rendered rows find looks through: the message's column, or its
    /// source while that is shown.
    fn find_text(&self) -> Vec<String> {
        let (outer, inner, _) = self.open_geometry();
        match &self.open.raw {
            Some(raw) => raw.text.as_ref().map_or_else(Vec::new, |text| {
                draw::raw_lines(text, usize::from(inner).saturating_sub(2).max(1))
            }),
            None => {
                let column = crate::layout::column_width(outer).min(inner);
                draw::document(
                    self,
                    &crate::theme::Theme::plain(),
                    column,
                    chrono::Local::now(),
                )
                .lines
                .iter()
                .map(|line| line.line.to_string())
                .collect()
            }
        }
    }

    /// Every match of what is typed.
    fn find_matches(&self) -> Vec<Match> {
        match &self.open.find {
            Some(find) => matches(&self.find_text(), find.query()),
            None => Vec::new(),
        }
    }

    /// How many matches there are, for the field to say.
    pub fn find_total(&self) -> usize {
        self.find_matches().len()
    }

    /// `find_in_message`: the field opens, with what was typed kept.
    pub(super) fn open_find(&mut self) -> Vec<Effect> {
        if self.reading.is_none() || self.focus != Focus::Reader {
            return Vec::new();
        }
        self.open.find.get_or_insert_with(Find::default);
        self.show_current();
        vec![Effect::Redraw]
    }

    /// Close the field and its marks.
    pub(super) fn close_find(&mut self) -> Vec<Effect> {
        self.open.find = None;
        vec![Effect::Redraw]
    }

    /// `find_next` and `find_previous`: the field is opened when it is not.
    pub(super) fn find_step(&mut self, forward: bool) -> Vec<Effect> {
        if self.open.find.is_none() {
            return self.open_find();
        }
        let found = self.find_matches();
        if let Some(find) = self.open.find.as_mut() {
            find.current = step(find.current, found.len(), forward);
        }
        self.show_current();
        vec![Effect::Redraw]
    }

    /// The query changed: the match to show is the first from where the
    /// person is reading.
    fn restart_find(&mut self) {
        let found = self.find_matches();
        let from = self.reader_top;
        if let Some(find) = self.open.find.as_mut() {
            find.current = first_from(&found, from);
        }
        self.show_current();
    }

    /// Scroll so the match the field is on is in view.
    fn show_current(&mut self) {
        let found = self.find_matches();
        let Some(at) = self
            .open
            .find
            .as_ref()
            .and_then(|find| find.current)
            .map(|at| at.min(found.len().saturating_sub(1)))
        else {
            return;
        };
        let Some(line) = found.get(at).map(|found| found.line) else {
            return;
        };
        let (length, shown) = self.open_extent();
        if line < self.reader_top || line >= self.reader_top + shown {
            let last = length.saturating_sub(shown.min(length));
            self.reader_top = line.saturating_sub(shown / 3).min(last);
        }
    }

    /// A key while the field has the keyboard.
    pub(super) fn find_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let typing = true;
        match self.keys.press(key, self.key_context(), typing) {
            Outcome::Command(id) => match id.as_str() {
                "find_next" => return self.find_step(true),
                "find_previous" => return self.find_step(false),
                "back" => return self.close_find(),
                _ => return Vec::new(),
            },
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        match key.code {
            KeyCode::Esc => return self.close_find(),
            KeyCode::Enter => return self.find_step(!key.modifiers.contains(KeyModifiers::SHIFT)),
            _ => {}
        }
        let Some(find) = self.open.find.as_mut() else {
            return Vec::new();
        };
        let before = find.input.value().to_owned();
        find.input.handle_event(&TerminalEvent::Key(*key));
        if find.input.value() != before {
            self.restart_find();
        }
        vec![Effect::Redraw]
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_model::{EmailAddress, MessageId};
    use ratatui::style::Modifier;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::test_support::{
        app, buffer, conversation, ctrl, key, local, open_message, places, press, screen,
        seed_places, show_rows, type_text,
    };

    /// A note of forty lines with the word three times, on lines 3, 12 and 33
    /// of the body.
    fn long_note() -> String {
        (0..40)
            .map(|line| match line {
                3 => "The Harbor survey is in".to_owned(),
                12 => "ask the harbor master".to_owned(),
                33 => "and HARBOR dues, last".to_owned(),
                n => format!("Line {n} of the note."),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn opened(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut contents = places();
        if size.0 >= 128 {
            contents.features.reading = postio_config::Reading::Pane;
        }
        seed_places(&mut app, contents);
        show_rows(
            &mut app,
            &[crate::row::Row::from(
                postio_ui::focus_list::FocusRow::conversation(conversation(
                    1,
                    "Lena Park",
                    "Survey",
                    "",
                    local(23, 9, 0),
                )),
            )],
        );
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        open_message(
            &mut app,
            MessageId::new(1),
            vec![EmailAddress::new(None::<String>, "you@example.com")],
            postio_model::MessageBody {
                text: Some(long_note()),
                html: None,
            },
        );
        app
    }

    fn rows(app: &App, size: (u16, u16)) -> Vec<String> {
        screen(size.0, size.1, app)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The row the find field is on: the last inside the frame.
    fn field(app: &App, size: (u16, u16)) -> String {
        rows(app, size)[usize::from(size.1) - 3].clone()
    }

    /// The columns of `word` on the screen row `y`, whose cells carry `modifier`.
    fn marked(app: &App, size: (u16, u16), word: &str, modifier: Modifier) -> Vec<(u16, u16)> {
        let drawn = buffer(size.0, size.1, app);
        let mut found = Vec::new();
        for y in 0..size.1 {
            let line: Vec<String> = (0..size.0)
                .map(|x| drawn[(x, y)].symbol().to_owned())
                .collect();
            let wanted: Vec<String> = word.chars().map(String::from).collect();
            for x in 0..line.len().saturating_sub(wanted.len() - 1) {
                if line[x..x + wanted.len()].to_vec() == wanted
                    && (0..wanted.len() as u16)
                        .all(|at| drawn[(x as u16 + at, y)].modifier.contains(modifier))
                {
                    found.push((x as u16, y));
                }
            }
        }
        found
    }

    const FRAME: (u16, u16) = (100, 24);

    #[test]
    fn the_find_key_opens_a_one_line_field_at_the_bottom_of_the_frame() {
        let mut app = opened(FRAME);
        assert!(!field(&app, FRAME).contains('⌕'), "closed at first");
        update(&mut app, ctrl('f'));
        let line = field(&app, FRAME);
        assert!(line.contains('⌕'), "{line}");
        assert!(line.contains("Esc"), "closing is hinted: {line}");
        assert_eq!(app.focus(), Focus::Reader);
    }

    #[test]
    fn typing_marks_every_match_whatever_its_case_and_the_field_counts_them() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harbor");
        assert!(
            field(&app, FRAME).contains("harbor"),
            "{}",
            field(&app, FRAME)
        );
        assert!(
            field(&app, FRAME).contains("1 of 3"),
            "{}",
            field(&app, FRAME)
        );
        // Without colour a match is reversed, the one the field is on besides
        // underlined; the body's own words are not marked.
        let reversed = marked(&app, FRAME, "Harbor", Modifier::REVERSED);
        assert_eq!(
            reversed.len(),
            1,
            "the first match is on screen: {reversed:?}"
        );
        let current = marked(&app, FRAME, "Harbor", Modifier::UNDERLINED);
        assert_eq!(current, reversed, "and it is the current one");
        assert!(marked(&app, FRAME, "Line 5", Modifier::REVERSED).is_empty());
    }

    #[test]
    fn enter_and_the_find_keys_step_between_matches_scrolling_each_into_view() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harbor");
        assert!(!screen(FRAME.0, FRAME.1, &app).contains("ask the harbor master"));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            field(&app, FRAME).contains("2 of 3"),
            "{}",
            field(&app, FRAME)
        );
        assert!(screen(FRAME.0, FRAME.1, &app).contains("ask the harbor master"));
        assert_eq!(
            marked(&app, FRAME, "harbor", Modifier::UNDERLINED).len(),
            1,
            "the current match moved"
        );
        // ctrl+g and F3 go on; shift+F3 and shift+Enter go back.
        update(&mut app, ctrl('g'));
        assert!(field(&app, FRAME).contains("3 of 3"));
        assert!(screen(FRAME.0, FRAME.1, &app).contains("and HARBOR dues"));
        update(&mut app, key(KeyCode::F(3), KeyModifiers::NONE));
        assert!(field(&app, FRAME).contains("1 of 3"), "wraps round");
        update(&mut app, key(KeyCode::F(3), KeyModifiers::SHIFT));
        assert!(field(&app, FRAME).contains("3 of 3"), "and back");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(field(&app, FRAME).contains("2 of 3"));
    }

    #[test]
    fn the_field_says_when_there_are_no_matches_and_marks_nothing() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "zzz");
        assert!(
            field(&app, FRAME).contains("No matches"),
            "{}",
            field(&app, FRAME)
        );
        assert!(marked(&app, FRAME, "Line 1", Modifier::REVERSED).is_empty());
        // Stepping with nothing to step to changes nothing and does not fail.
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(field(&app, FRAME).contains("No matches"));
    }

    #[test]
    fn backspace_edits_the_query_and_the_matches_follow() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harborx");
        assert!(field(&app, FRAME).contains("No matches"));
        update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(field(&app, FRAME).contains("1 of 3"));
    }

    #[test]
    fn escape_closes_the_field_and_the_marks_and_leaves_the_message_open() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harbor");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!field(&app, FRAME).contains('⌕'), "{}", field(&app, FRAME));
        assert!(marked(&app, FRAME, "Harbor", Modifier::REVERSED).is_empty());
        assert_eq!(app.focus(), Focus::Reader, "the message is still open");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_ne!(app.focus(), Focus::Reader, "the second Escape closes it");
    }

    #[test]
    fn typing_in_the_field_is_not_a_command() {
        let mut app = opened(FRAME);
        update(&mut app, ctrl('f'));
        let effects = type_text(&mut app, "ae");
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Send(_))),
            "a and e are letters here, not Archive and Reply: {effects:?}"
        );
        assert!(field(&app, FRAME).contains("ae"));
    }

    #[test]
    fn find_next_and_previous_open_the_field_when_it_is_closed() {
        let mut app = opened(FRAME);
        update(&mut app, key(KeyCode::F(3), KeyModifiers::NONE));
        assert!(field(&app, FRAME).contains('⌕'));
        assert_ne!(press('x'), ctrl('x'));
    }

    #[test]
    fn beside_the_list_the_pane_finds_the_same_way() {
        let size = (140, 30);
        let mut app = opened(size);
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harbor");
        let bottom = rows(&app, size)[usize::from(size.1) - 2].clone();
        assert!(
            bottom.contains('⌕') && bottom.contains("1 of 3"),
            "{bottom}"
        );
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen(size.0, size.1, &app).contains("ask the harbor master"));
    }

    #[test]
    fn find_looks_through_the_source_while_it_is_shown() {
        let mut app = opened(FRAME);
        update(&mut app, press('v'));
        update(
            &mut app,
            Input::Source {
                message: MessageId::new(1),
                raw: Ok(b"From: lena@example.org\nSubject: Survey\n\nthe Harbor\n".to_vec()),
            },
        );
        update(&mut app, ctrl('f'));
        type_text(&mut app, "harbor");
        assert!(
            field(&app, FRAME).contains("1 of 1"),
            "{}",
            field(&app, FRAME)
        );
        assert_eq!(marked(&app, FRAME, "Harbor", Modifier::REVERSED).len(), 1);
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            app.raw().is_some(),
            "the first Escape closes the field only"
        );
    }
}
