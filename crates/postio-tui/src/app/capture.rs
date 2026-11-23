//! Capture (`T` and `N`): the controller's sheet, typed into with this terminal's text fields.

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent};
use postio_core::CommandId;
use postio_focus::{Input as Told, Intent, SurfaceKind};
use postio_ui::keymap::{KeyContext, Outcome};
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus};
use crate::capture::{Field, Sheet};
use crate::surface::Part;

impl App {
    pub fn capture(&self) -> Option<&Sheet> {
        self.surfaces.capture.as_ref()
    }

    pub(super) fn show_capture_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::OpenCapture(view) => {
                self.surfaces.capture = Some(Sheet::new(*view, self.focus));
                self.focus = Focus::Capture;
            }
            Intent::Capture(view) => {
                if let Some(sheet) = self.surfaces.capture.as_mut() {
                    sheet.replace(*view);
                }
            }
            Intent::CloseSurface(SurfaceKind::Capture) => {
                if let Some(sheet) = self.surfaces.capture.take()
                    && self.focus == Focus::Capture
                {
                    self.focus = sheet.from();
                }
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub(super) fn capture_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(sheet) = self.surfaces.capture.as_ref() else {
            return Vec::new();
        };
        let (typing, choosing) = (sheet.typing(), sheet.choosing());
        if key.code == KeyCode::Esc {
            return self.capture_back();
        }
        match self.keys.press(key, KeyContext::Capture, typing) {
            Outcome::Command(id) if id == "back" => return self.capture_back(),
            Outcome::Command(id) => return self.command(&id),
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        let Some(sheet) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        if choosing {
            return self.project_chooser_key(key);
        }
        match (key.code, sheet.field()) {
            (KeyCode::Tab, _) => sheet.walk(1),
            (KeyCode::BackTab, _) => sheet.walk(-1),
            (KeyCode::Left | KeyCode::Up, Field::Due) => return self.step_due(-1),
            (KeyCode::Right | KeyCode::Down, Field::Due) => return self.step_due(1),
            (KeyCode::Enter, Field::Project) => {
                return self.tell(Told::Command(CommandId::CaptureChangeProject));
            }
            (_, Field::Text) => {
                let before = sheet.text().value().to_owned();
                sheet.text_mut().handle_event(&TerminalEvent::Key(*key));
                let text = sheet.text().value().to_owned();
                if text != before {
                    return self.tell(Told::CaptureTyped { text });
                }
            }
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    fn capture_back(&mut self) -> Vec<Effect> {
        let choosing = self.surfaces.capture.as_ref().is_some_and(Sheet::choosing);
        let command = if choosing {
            CommandId::CaptureChangeProject
        } else {
            CommandId::Back
        };
        self.tell(Told::Command(command))
    }

    fn step_due(&mut self, by: isize) -> Vec<Effect> {
        match self
            .surfaces
            .capture
            .as_ref()
            .and_then(|sheet| sheet.pick_by(by))
        {
            Some(day) => self.tell(Told::CaptureDue(day)),
            None => Vec::new(),
        }
    }

    fn project_chooser_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(sheet) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Up => sheet.choose_by(-1),
            KeyCode::Down => sheet.choose_by(1),
            KeyCode::Enter => {
                let at = sheet.at();
                return self.choose_project(at);
            }
            _ => {
                let before = sheet.filter().value().to_owned();
                sheet.filter_mut().handle_event(&TerminalEvent::Key(*key));
                let text = sheet.filter().value().to_owned();
                if text != before {
                    return self.tell(Told::CaptureFilter { text });
                }
            }
        }
        vec![Effect::Redraw]
    }

    fn choose_project(&mut self, at: usize) -> Vec<Effect> {
        let token = self
            .surfaces
            .capture
            .as_ref()
            .and_then(|sheet| sheet.view().projects.get(at))
            .map(|project| project.token);
        match token {
            Some(token) => self.tell(Told::CaptureProject(token)),
            None => Vec::new(),
        }
    }

    pub(super) fn capture_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let Some(sheet) = self.surfaces.capture.as_mut() else {
            return Vec::new();
        };
        match part {
            Part::CaptureCancel => self.tell(Told::Command(CommandId::Back)),
            Part::CaptureMode => self.tell(Told::Command(if index == 0 {
                CommandId::CaptureTask
            } else {
                CommandId::CaptureNote
            })),
            Part::CapturePick => {
                sheet.focus(Field::Due);
                match sheet.view().picks.get(index).map(|pick| pick.day) {
                    Some(day) => self.tell(Told::CaptureDue(day)),
                    None => Vec::new(),
                }
            }
            Part::CaptureProject => self.choose_project(index),
            Part::CaptureText => {
                sheet.focus(Field::Text);
                vec![Effect::Redraw]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_focus::{Reply, Request};
    use postio_model::MessageId;
    use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen};
    use postio_ui::capture::NO_VAULT;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, update};
    use crate::test_support::{
        alt, app, conversation, key, local, marked, places_with_features, press, screen,
        seed_places, serve_focus, serve_vault, show_focus, type_text,
    };

    fn due_day() -> chrono::NaiveDate {
        crate::test_support::now().date_naive() + chrono::Duration::days(2)
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

    fn inbox(vault: bool) -> App {
        let mut app = app((120, 40));
        app.clock_for_tests(crate::test_support::now());
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

    fn opened() -> App {
        let mut app = inbox(true);
        let effects = update(&mut app, press('T'));
        serve_vault(&mut app, effects, picture());
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

    fn flat(screen: &str) -> String {
        screen.replace("📅  ", "📅 ")
    }

    #[test]
    fn shift_t_opens_the_sheet_on_the_marker_sentence_and_reads_the_vault() {
        let mut app = inbox(true);
        let effects = update(&mut app, press('T'));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::Vault { subject, .. } if subject == "Atlas figures"
        )));
        assert_eq!(app.focus(), Focus::Capture);
        serve_vault(&mut app, effects, picture());
        let drawn = flat(&screen(120, 40, &app));
        assert!(drawn.contains("Task T   Note N"), "{drawn}");
        assert!(drawn.contains("From Ada Moreno · Atlas figures"), "{drawn}");
        assert!(
            drawn.contains("Please send the Atlas figures by Friday"),
            "{drawn}"
        );
        assert!(
            line_with(&drawn, "use the subject instead").contains("alt+s"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Project · suggested: the subject names Atlas"),
            "{drawn}"
        );
        assert!(
            drawn.contains("- [ ] Please send the Atlas figures by Friday"),
            "the exact line:\n{drawn}"
        );
    }

    #[test]
    fn with_no_vault_shift_t_and_shift_n_say_to_name_one() {
        for letter in ['T', 'N'] {
            let mut app = inbox(false);
            let effects = update(&mut app, press(letter));
            assert!(!asked(&effects, |_| true), "{effects:?}");
            assert_eq!(app.focus(), Focus::List);
            assert!(screen(120, 40, &app).contains(NO_VAULT));
        }
    }

    #[test]
    fn typing_and_alt_s_change_the_line_and_a_letter_in_the_text_is_typed() {
        let mut app = opened();
        type_text(&mut app, "TN");
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("Please send the Atlas figures by FridayTN"),
            "{drawn}"
        );
        assert!(drawn.contains("Add task"), "still a task");
        update(&mut app, alt('s'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("- [ ] Atlas figures [✉](postio://message/1)"),
            "{drawn}"
        );
        assert_eq!(app.capture().unwrap().text().value(), "Atlas figures");
    }

    #[test]
    fn the_due_day_has_quick_picks_and_shift_n_makes_it_a_note() {
        let mut app = opened();
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        assert!(
            !flat(&screen(120, 40, &app)).contains(&format!("{}", due_day().format("%Y-%m-%d"))),
            "a pick other than the mail's day moved the due day"
        );
        update(&mut app, press('N'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            drawn.contains("This exact entry will be appended"),
            "{drawn}"
        );
        assert!(!drawn.contains("📅") && !drawn.contains("Due "), "{drawn}");
        assert!(drawn.contains("Add note"), "{drawn}");
    }

    #[test]
    fn ctrl_p_lists_the_projects_and_choosing_one_moves_the_capture() {
        let mut app = opened();
        update(&mut app, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let drawn = flat(&screen(120, 40, &app));
        assert!(line_with(&drawn, "✓ Atlas").contains("1 open"), "{drawn}");
        assert!(
            drawn.contains("Garden") && drawn.contains("Inbox"),
            "{drawn}"
        );
        type_text(&mut app, "gar");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            line_with(&drawn, "Projects/Garden.md").contains("Garden"),
            "{drawn}"
        );
        assert!(
            !drawn.contains("suggested:"),
            "no longer the suggestion:\n{drawn}"
        );
    }

    #[test]
    fn ctrl_enter_writes_the_task_and_the_toast_says_where() {
        let mut app = opened();
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::CONTROL));
        let written = effects.iter().find_map(|effect| match effect {
            Effect::Focus {
                request: Request::CaptureTask { project, task, .. },
                ..
            } => Some((project.clone(), task.clone())),
            _ => None,
        });
        let (chosen, task) = written.unwrap_or_else(|| panic!("{effects:?}"));
        assert_eq!(chosen, Some(project("Atlas")));
        assert_eq!(task.text, "Please send the Atlas figures by Friday");
        assert_eq!(task.due, Some(due_day()));
        serve_focus(&mut app, effects, |request| match request {
            Request::CaptureTask { stamp, .. } => Some(Reply::Captured {
                stamp: *stamp,
                answer: Ok(()),
            }),
            _ => None,
        });
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
    fn a_vault_that_refuses_is_said_in_the_sheet_which_stays() {
        let mut app = opened();
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::CONTROL));
        serve_focus(&mut app, effects, |request| match request {
            Request::CaptureTask { stamp, .. } => Some(Reply::Captured {
                stamp: *stamp,
                answer: Err("The vault could not be written".into()),
            }),
            _ => None,
        });
        assert_eq!(app.focus(), Focus::Capture);
        assert!(screen(120, 40, &app).contains("The vault could not be written"));
    }

    #[test]
    fn escape_writes_nothing_and_in_the_chooser_closes_only_the_chooser() {
        let mut app = opened();
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !asked(&effects, |request| matches!(
                request,
                Request::CaptureTask { .. }
            )),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::List);
        let mut app = opened();
        update(&mut app, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Capture);
        assert!(!app.capture().unwrap().choosing());
    }

    #[test]
    fn every_part_is_a_click() {
        use crate::surface::Part;
        use crate::test_support::click;
        use crate::view::hit::Target;
        let mut app = opened();
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
            app.capture().unwrap().view().due,
            Some(crate::test_support::now().date_naive())
        );
        update(
            &mut app,
            click(Target::Command("capture_change_project"), false, false),
        );
        assert!(app.capture().unwrap().choosing());
        update(
            &mut app,
            click(Target::Surface(Part::CaptureProject, 1), false, false),
        );
        assert_eq!(app.capture().unwrap().view().project, "Garden");
        let effects = update(
            &mut app,
            click(Target::Command("capture_write"), false, false),
        );
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::CaptureTask { .. }
            )),
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
        update(&mut app, press('T'));
        let drawn = flat(&screen(120, 40, &app));
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("now"), "{drawn}");
    }
}
