//! Capture (`t` and `n`): a task or a note written into the person's vault.
//!
//! The state is `crate::capture`; the drawing is `view::capture`. `t` and
//! `n` open the sheet on the message under the cursor, or the one open; in
//! the sheet they switch it between a task and a note wherever a letter is
//! not being typed. `ctrl+Return` writes.

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent};
use postio_ui::capture::{self, Mode, Source};
use postio_ui::keymap::{KeyContext, Outcome};
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus, Tone};
use crate::ask::{Answer, Ask};
use crate::capture::{Field, Sheet};
use crate::surface::Part;

impl App {
    /// The capture sheet, while it is open.
    pub fn capture(&self) -> Option<&Sheet> {
        self.surfaces.capture.as_ref().map(|(sheet, _)| sheet)
    }

    /// What a capture of the message in front of the person is made from:
    /// the row under the cursor, or the open message. `None` where there is
    /// no one message: a digest, or nothing.
    fn capture_source(&self) -> Option<Source> {
        let now = chrono::Local::now();
        let (message, row) = if self.focus == Focus::Reader {
            let reading = self.reading.as_ref()?;
            let member = reading.members.get(reading.current)?;
            (member.id, self.row_of_open())
        } else {
            let row = self.row_at(self.cursor)?;
            if row.kind == crate::row::Kind::Digest {
                return None;
            }
            (row.id, Some(row))
        };
        let Some(row) = row.filter(|row| row.id == message) else {
            let title = self
                .row_of_open()
                .map(|row| row.subject.as_str().to_owned())
                .unwrap_or_default();
            return Some(Source {
                message,
                sender: String::new(),
                subject: title,
                when: String::new(),
                sentence: None,
                due: None,
            });
        };
        let marker = row.marker.as_ref();
        Some(Source {
            message,
            sender: row.from.as_str().to_owned(),
            subject: row.subject.as_str().to_owned(),
            when: postio_ui::row::timestamp(row.when, now),
            sentence: marker.and_then(|marker| marker.excerpt.clone()),
            due: marker.and_then(|marker| match marker.when {
                Some(postio_model::listing::MarkerWhen::Due(at))
                    if marker.kind == postio_model::listing::MarkerKind::Todo =>
                {
                    Some(at.with_timezone(&chrono::Local).date_naive())
                }
                _ => None,
            }),
        })
    }

    /// `t` and `n`: the sheet, over the message, reading the vault.
    pub(super) fn open_capture(&mut self, mode: Mode) -> Vec<Effect> {
        if !self.features.capture {
            return self.say(capture::NO_VAULT);
        }
        let Some(source) = self.capture_source() else {
            return Vec::new();
        };
        let subject = source.subject.clone();
        let sheet = Sheet::new(source, mode, chrono::Local::now().date_naive());
        self.surfaces.capture = Some((sheet, self.focus));
        self.focus = Focus::Capture;
        vec![Effect::Ask(Ask::Vault(subject)), Effect::Redraw]
    }

    /// Put the sheet away: the keyboard goes back where it was.
    fn close_capture(&mut self) -> Vec<Effect> {
        if let Some((_, from)) = self.surfaces.capture.take() {
            self.focus = from;
        }
        vec![Effect::Redraw]
    }

    /// A key with the keyboard in the sheet.
    pub(super) fn capture_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        // Escape gives up, as it does in any text field.
        if key.code == KeyCode::Esc {
            return self.capture_command("back").unwrap_or_default();
        }
        let typing = sheet.typing();
        match self.keys.press(key, KeyContext::Capture, typing) {
            Outcome::Command(id) => return self.capture_command(&id).unwrap_or_default(),
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        if sheet.chooser().is_some() {
            match key.code {
                KeyCode::Up => sheet.choose_by(-1),
                KeyCode::Down => sheet.choose_by(1),
                KeyCode::Enter => {
                    let at = sheet.chooser().map_or(0, |chooser| chooser.at);
                    sheet.choose(at);
                }
                _ => {
                    if let Some(chooser) = sheet.chooser_mut() {
                        let before = chooser.filter.value().to_owned();
                        chooser.filter.handle_event(&TerminalEvent::Key(*key));
                        if chooser.filter.value() != before {
                            chooser.at = 0;
                        }
                    }
                }
            }
            return vec![Effect::Redraw];
        }
        match (key.code, sheet.field()) {
            (KeyCode::Tab, _) => sheet.walk(1),
            (KeyCode::BackTab, _) => sheet.walk(-1),
            (KeyCode::Left, Field::Due) | (KeyCode::Up, Field::Due) => sheet.pick_by(-1),
            (KeyCode::Right, Field::Due) | (KeyCode::Down, Field::Due) => sheet.pick_by(1),
            (KeyCode::Enter, Field::Project) => sheet.toggle_chooser(),
            (_, Field::Text) => {
                sheet.say(None);
                sheet.text_mut().handle_event(&TerminalEvent::Key(*key));
            }
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    /// What the sheet does for `id`, or `None` when it is not its own.
    pub(super) fn capture_command(&mut self, id: &str) -> Option<Vec<Effect>> {
        let (sheet, _) = self.surfaces.capture.as_mut()?;
        match id {
            "capture_task" => sheet.set_mode(Mode::Task),
            "capture_note" => sheet.set_mode(Mode::Note),
            "capture_use_subject" => sheet.use_subject(),
            "capture_change_project" => sheet.toggle_chooser(),
            "capture_write" => return Some(self.capture_write()),
            "back" => {
                if sheet.chooser().is_some() {
                    sheet.toggle_chooser();
                } else {
                    return Some(self.close_capture());
                }
            }
            "undo" | "cheat_sheet" => return Some(self.global(id)),
            _ => return Some(Vec::new()),
        }
        Some(vec![Effect::Redraw])
    }

    /// `ctrl+Return`: append what the preview shows, through the host.
    fn capture_write(&mut self) -> Vec<Effect> {
        let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        let ask = match sheet.mode() {
            Mode::Task => Ask::CaptureTask {
                project: sheet.project().cloned(),
                task: sheet.task(),
            },
            Mode::Note if sheet.has_vault() => Ask::CaptureNote {
                note: sheet.note_path(),
                entry: sheet.entry(),
            },
            Mode::Note => {
                sheet.say(Some("The vault has not been read yet".to_owned()));
                return vec![Effect::Redraw];
            }
        };
        sheet.say(None);
        vec![Effect::Ask(ask), Effect::Redraw]
    }

    /// What the host answered the sheet.
    pub(super) fn capture_answered(&mut self, answer: Answer) -> Vec<Effect> {
        match answer {
            Answer::Vault(read) => {
                let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
                    return Vec::new();
                };
                match read {
                    Ok(picture) => sheet.read(picture),
                    Err(reason) => sheet.say(Some(reason)),
                }
            }
            Answer::Captured(Ok(_)) => {
                let Some(said) = self.capture().map(Sheet::said) else {
                    return Vec::new();
                };
                let mut effects = self.close_capture();
                effects.extend(self.say_as(Tone::Worked, &said, None));
                return effects;
            }
            Answer::Captured(Err(reason)) => {
                let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
                    return Vec::new();
                };
                sheet.say(Some(reason));
            }
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    /// A click on a part of the sheet.
    pub(super) fn capture_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        if part == Part::CaptureCancel {
            return self.close_capture();
        }
        let Some((sheet, _)) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        match part {
            Part::CaptureMode => {
                sheet.set_mode(if index == 0 { Mode::Task } else { Mode::Note });
            }
            Part::CapturePick => {
                sheet.focus(Field::Due);
                sheet.pick(index);
            }
            Part::CaptureProject => sheet.choose(index),
            Part::CaptureText => sheet.focus(Field::Text),
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_model::MessageId;
    use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen};
    use postio_ui::capture::NO_VAULT;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::ask::{Answer, Ask};
    use crate::test_support::{
        alt, app, conversation, key, local, marked, places_with_features, press, screen,
        seed_places, show_focus, type_text,
    };

    /// The day a to-do of the drawing is due: two days from today, so it is
    /// among the picks of any weekday.
    fn due_day() -> chrono::NaiveDate {
        chrono::Local::now().date_naive() + chrono::Duration::days(2)
    }

    fn project(name: &str) -> postio_vault::Project {
        postio_vault::Project {
            name: name.to_owned(),
            note: std::path::PathBuf::from(format!("Projects/{name}.md")),
        }
    }

    fn picture() -> postio_client::protocol::VaultPicture {
        postio_client::protocol::VaultPicture {
            projects: vec![project("Atlas"), project("Garden")],
            suggestion: Some(postio_vault::Suggestion {
                project: project("Atlas"),
                reason: postio_vault::Reason::NamedInSubject("atlas".into()),
            }),
            tasks_note: std::path::PathBuf::from("Tasks.md"),
            tasks: vec![postio_vault::CapturedTask {
                note: std::path::PathBuf::from("Projects/Atlas.md"),
                message: MessageId::new(9),
                text: "Earlier".into(),
                due: None,
                finished: false,
                done: None,
            }],
        }
    }

    /// The inbox: a to-do about Atlas that says when it is due, and a plain
    /// message.
    fn inbox(vault: bool) -> App {
        let mut app = app((120, 40));
        let mut places = places_with_features();
        places.features.capture = vault;
        seed_places(&mut app, places);
        let todo = MarkerSummary {
            kind: MarkerKind::Todo,
            when: Some(MarkerWhen::Due(
                due_day()
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            )),
            excerpt: Some("Please send the Atlas figures by Friday".into()),
            answer: None,
            cancelled: false,
        };
        show_focus(
            &mut app,
            vec![
                FocusRow::conversation(marked(
                    conversation(1, "Ada Moreno", "Atlas figures", "", local(23, 9, 0)),
                    todo,
                )),
                FocusRow::conversation(conversation(2, "Grace", "Lunch", "", local(23, 8, 0))),
            ],
        );
        app
    }

    fn opened(vault: bool) -> App {
        let mut app = inbox(vault);
        update(&mut app, press('t'));
        update(&mut app, Input::Answer(Answer::Vault(Ok(picture()))));
        app
    }

    fn line_with<'a>(screen: &'a str, wanted: &str) -> &'a str {
        screen
            .lines()
            .find(|line| line.contains(wanted))
            .unwrap_or_else(|| panic!("no line holds {wanted:?} in\n{screen}"))
    }

    /// The screen with each wide mark's second cell taken out, as a person
    /// reads it.
    fn flat(screen: &str) -> String {
        screen.replace("📅  ", "📅 ")
    }

    fn ctrl_return() -> Input {
        key(KeyCode::Enter, KeyModifiers::CONTROL)
    }

    #[test]
    fn t_opens_the_sheet_on_the_marker_sentence_and_reads_the_vault() {
        let mut app = inbox(true);
        let effects = update(&mut app, press('t'));
        assert!(
            effects.contains(&Effect::Ask(Ask::Vault("Atlas figures".into()))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Capture);
        let drawn = flat(&screen(120, 40, &app));
        assert!(drawn.contains("Capture"), "{drawn}");
        assert!(drawn.contains("Task t   Note n"), "{drawn}");
        assert!(drawn.contains("From Ada Moreno · Atlas figures"), "{drawn}");
        assert!(
            drawn.contains("Please send the Atlas figures by Friday"),
            "{drawn}"
        );
        assert!(
            drawn.contains("The sentence from the mail, as written"),
            "{drawn}"
        );
        assert!(
            line_with(&drawn, "use the subject instead").contains("alt+s"),
            "{drawn}"
        );
    }

    #[test]
    fn with_no_vault_t_and_n_say_to_name_one() {
        for letter in ['t', 'n'] {
            let mut app = inbox(false);
            let effects = update(&mut app, press(letter));
            assert!(
                !effects.iter().any(|e| matches!(e, Effect::Ask(_))),
                "{effects:?}"
            );
            assert_eq!(app.focus(), Focus::List);
            assert!(screen(120, 40, &app).contains(NO_VAULT));
        }
    }

    #[test]
    fn the_project_is_suggested_with_its_reason_and_the_preview_is_the_exact_line() {
        let app = opened(true);
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("Project · suggested: the subject names Atlas"),
            "{drawn}"
        );
        let named = line_with(&drawn, "Projects/Atlas.md");
        assert!(
            named.contains("Atlas") && named.contains("Change ctrl+p"),
            "{named}"
        );
        assert!(
            drawn.contains("This exact line will be appended"),
            "{drawn}"
        );
        // The line is wrapped to the frame; it is one line in the note.
        assert!(
            drawn.contains("- [ ] Please send the Atlas figures by Friday"),
            "{drawn}"
        );
        let tail = format!(
            "[✉](postio://message/1) 📅 {}",
            due_day().format("%Y-%m-%d")
        );
        assert!(drawn.contains(&tail), "{drawn}");
        assert!(
            drawn.contains("Plain markdown, Obsidian Tasks format, written on this computer."),
            "{drawn}"
        );
        assert!(drawn.contains("goes in Atlas, due"), "{drawn}");
    }

    #[test]
    fn the_due_day_has_quick_picks_and_choosing_one_changes_the_line() {
        let mut app = opened(true);
        let drawn = flat(&screen(120, 40, &app));
        let picks =
            postio_ui::capture::quick_picks(chrono::Local::now().date_naive(), Some(due_day()));
        let row = line_with(&drawn, "None");
        for pick in &picks {
            assert!(row.contains(&pick.words), "{} in {row}", pick.words);
        }
        assert!(
            drawn.contains(&postio_ui::capture::due_label(Some(due_day()))),
            "{drawn}"
        );
        // Tab to the picks, and move to None.
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::End, KeyModifiers::NONE));
        for _ in 0..picks.len() {
            update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        }
        let drawn = flat(&screen(120, 40, &app));
        assert!(drawn.contains("No due date"), "{drawn}");
        assert!(!drawn.contains('📅'), "{drawn}");
    }

    #[test]
    fn n_makes_it_a_note_with_no_due_day_and_its_own_words() {
        let mut app = opened(true);
        // Tab off the text, where a letter is typed, and press n.
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('n'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("This exact entry will be appended"),
            "{drawn}"
        );
        assert!(
            drawn.contains("- Please send the Atlas figures by Friday [✉](postio://message/1)"),
            "{drawn}"
        );
        assert!(!drawn.contains("📅") && !drawn.contains("Due "), "{drawn}");
        assert!(drawn.contains("Add note"), "{drawn}");
        assert!(
            drawn.contains("Plain markdown, written on this computer. It goes in Atlas's note."),
            "{drawn}"
        );
        // The text has the keyboard now, and a letter there is typed: walk on.
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('t'));
        assert!(screen(120, 40, &app).contains("Add task"));
    }

    #[test]
    fn a_letter_in_the_text_is_typed_not_run() {
        let mut app = opened(true);
        type_text(&mut app, "tn");
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("Please send the Atlas figures by Fridaytn"),
            "{drawn}"
        );
        assert!(drawn.contains("Add task"), "still a task");
    }

    #[test]
    fn alt_s_puts_the_subject_in_the_texts_place() {
        let mut app = opened(true);
        update(&mut app, alt('s'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("- [ ] Atlas figures [✉](postio://message/1)"),
            "{drawn}"
        );
        assert_eq!(app.capture().unwrap().text().value(), "Atlas figures");
    }

    #[test]
    fn ctrl_p_lists_the_projects_and_the_inbox_and_choosing_one_moves_the_capture() {
        let mut app = opened(true);
        update(&mut app, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let drawn = flat(&screen(120, 40, &app));
        assert!(line_with(&drawn, "✓ Atlas").contains("1 open"), "{drawn}");
        assert!(
            drawn.contains("Garden") && drawn.contains("Inbox"),
            "{drawn}"
        );
        assert!(drawn.contains("Tasks.md (no project)"), "{drawn}");
        // Typing narrows it; Return chooses.
        type_text(&mut app, "gar");
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            !drawn.contains("Inbox") || !drawn.contains("Tasks.md (no project)"),
            "{drawn}"
        );
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            line_with(&drawn, "Projects/Garden.md").contains("Garden"),
            "{drawn}"
        );
        assert!(drawn.contains("goes in Garden"), "{drawn}");
        assert!(
            drawn
                .lines()
                .any(|line| line.contains("│ Project ") && !line.contains("suggested")),
            "{drawn}"
        );
        assert!(
            !drawn.contains("suggested:"),
            "no longer the suggestion:\n{drawn}"
        );
    }

    #[test]
    fn ctrl_enter_writes_the_task_to_its_project_and_the_toast_says_where() {
        let mut app = opened(true);
        let effects = update(&mut app, ctrl_return());
        let asked = effects.iter().find_map(|e| match e {
            Effect::Ask(Ask::CaptureTask { project, task }) => {
                Some((project.clone(), task.clone()))
            }
            _ => None,
        });
        let (project_chosen, task) = asked.unwrap_or_else(|| panic!("{effects:?}"));
        assert_eq!(project_chosen, Some(project("Atlas")));
        assert_eq!(task.text, "Please send the Atlas figures by Friday");
        assert_eq!(task.message, MessageId::new(1));
        assert_eq!(task.due, Some(due_day()));
        update(
            &mut app,
            Input::Answer(Answer::Captured(Ok(postio_vault::Captured {
                note: "Projects/Atlas.md".into(),
                line: "- [ ] x".into(),
            }))),
        );
        assert_eq!(app.focus(), Focus::List);
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains(&format!(
                "Task added to Atlas · due {}",
                due_day().format("%a")
            )),
            "{drawn}"
        );
    }

    #[test]
    fn alt_enter_writes_too_and_a_note_goes_to_the_projects_note() {
        let mut app = opened(true);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('n'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::ALT));
        let asked = effects.iter().find_map(|e| match e {
            Effect::Ask(Ask::CaptureNote { note, entry }) => Some((note.clone(), entry.clone())),
            _ => None,
        });
        let (note, entry) = asked.unwrap_or_else(|| panic!("{effects:?}"));
        assert_eq!(note, std::path::PathBuf::from("Projects/Atlas.md"));
        assert_eq!(entry.text, "Please send the Atlas figures by Friday");
        update(
            &mut app,
            Input::Answer(Answer::Captured(Ok(postio_vault::Captured {
                note,
                line: "- x".into(),
            }))),
        );
        assert!(screen(120, 40, &app).contains("Note added to Atlas"));
    }

    #[test]
    fn a_vault_that_refuses_is_said_in_the_sheet_which_stays() {
        let mut app = opened(true);
        update(&mut app, ctrl_return());
        update(
            &mut app,
            Input::Answer(Answer::Captured(Err(
                "The vault could not be written".into()
            ))),
        );
        assert_eq!(app.focus(), Focus::Capture);
        assert!(screen(120, 40, &app).contains("The vault could not be written"));
    }

    #[test]
    fn escape_writes_nothing_and_goes_back_where_it_was() {
        let mut app = opened(true);
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::CaptureTask { .. })))
        );
        assert_eq!(app.focus(), Focus::List);
        // Escape in the chooser closes only the chooser.
        let mut app = opened(true);
        update(&mut app, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Capture);
        assert!(app.capture().unwrap().chooser().is_none());
    }

    #[test]
    fn a_marked_to_do_offers_task_before_snooze_only_with_a_vault() {
        let with = screen(120, 40, &inbox(true));
        let row = line_with(&with, "Task t");
        assert!(
            row.contains("Snooze s") && row.find("Task t") < row.find("Snooze s"),
            "{row}"
        );
        let without = screen(120, 40, &inbox(false));
        assert!(!without.contains("Task t"), "{without}");
        assert!(without.contains("Snooze s"), "{without}");
    }

    #[test]
    fn t_from_the_open_message_captures_that_message() {
        let mut app = inbox(true);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let effects = update(&mut app, press('t'));
        assert!(
            effects.contains(&Effect::Ask(Ask::Vault("Atlas figures".into()))),
            "{effects:?}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.focus(),
            Focus::Reader,
            "back in the message it came from"
        );
    }

    #[test]
    fn every_part_is_a_click() {
        use crate::surface::Part;
        use crate::test_support::{click, hits_of};
        use crate::view::hit::Target;
        let mut app = opened(true);
        let hits = hits_of(120, 40, &app);
        let on = |wanted: Target| {
            (0..40).any(|y| (0..120).any(|x| hits.at(x, y).is_some_and(|hit| hit.target == wanted)))
        };
        for part in [
            Part::CaptureMode,
            Part::CapturePick,
            Part::CaptureText,
            Part::CaptureCancel,
        ] {
            assert!(
                (0..2).any(|index| on(Target::Surface(part, index))),
                "{part:?}"
            );
        }
        for command in [
            "capture_change_project",
            "capture_use_subject",
            "capture_write",
        ] {
            assert!(on(Target::Command(command)), "{command}");
        }
        update(
            &mut app,
            click(Target::Surface(Part::CaptureMode, 1), false, false),
        );
        assert!(screen(120, 40, &app).contains("Add note"));
        update(
            &mut app,
            click(Target::Surface(Part::CaptureMode, 0), false, false),
        );
        update(
            &mut app,
            click(Target::Surface(Part::CapturePick, 0), false, false),
        );
        assert_eq!(
            app.capture().unwrap().due(),
            Some(chrono::Local::now().date_naive())
        );
        update(
            &mut app,
            click(Target::Command("capture_change_project"), false, false),
        );
        assert!(app.capture().unwrap().chooser().is_some());
        update(
            &mut app,
            click(Target::Surface(Part::CaptureProject, 1), false, false),
        );
        assert_eq!(app.capture().unwrap().place(), "Garden");
        let effects = update(
            &mut app,
            click(Target::Command("capture_write"), false, false),
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::CaptureTask { .. }))),
            "{effects:?}"
        );
        update(
            &mut app,
            click(Target::Surface(Part::CaptureCancel, 0), false, false),
        );
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn what_a_sender_wrote_never_reaches_the_terminal_as_a_control() {
        let mut app = inbox(true);
        let hostile = MarkerSummary {
            kind: MarkerKind::Todo,
            when: None,
            excerpt: Some("Pay \u{1b}[2J\u{7}now".into()),
            answer: None,
            cancelled: false,
        };
        show_focus(
            &mut app,
            vec![FocusRow::conversation(marked(
                conversation(
                    5,
                    "Eve\u{1b}[31m",
                    "Bad\u{1b}]0;x\u{7}",
                    "",
                    local(23, 9, 0),
                ),
                hostile,
            ))],
        );
        update(&mut app, press('t'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("now"), "{drawn}");
    }
}
