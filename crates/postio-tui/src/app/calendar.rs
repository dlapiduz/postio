use postio_core::CommandId;
use postio_focus::{CalendarView, Input as Told, Intent, SurfaceKind};
use postio_model::calendar::EditScope;
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};

impl App {
    pub fn calendar(&self) -> Option<&CalendarView> {
        self.surfaces.calendar.as_deref()
    }

    pub(super) fn show_calendar_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::Calendar(view) => {
                if view.editor.is_none() {
                    self.surfaces.editor_field = 0;
                }
                self.surfaces.calendar = Some(view);
                self.focus = Focus::Calendar;
            }
            Intent::CloseSurface(SurfaceKind::Calendar) => {
                self.surfaces.calendar = None;
                if self.focus == Focus::Calendar {
                    self.focus = Focus::List;
                }
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub fn editor_field(&self) -> usize {
        self.surfaces.editor_field
    }

    fn editor_key(&mut self, key: &crossterm::event::KeyEvent) -> Option<Vec<Effect>> {
        use crossterm::event::{KeyCode, KeyModifiers};
        use postio_ui::calendar::editor::EditorField;
        let editor = self.calendar()?.editor.as_ref()?;
        let stops = editor.fields.len() + usize::from(editor.scope.is_some());
        if stops == 0 {
            return None;
        }
        let at = self.surfaces.editor_field.min(stops - 1);
        let field = editor.fields.get(at).map(|(field, _)| *field);
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let typed = |field: EditorField| {
            editor
                .fields
                .iter()
                .find(|(each, _)| *each == field)
                .map(|(_, text)| text.clone())
                .unwrap_or_default()
        };
        let edit = match (key.code, field) {
            (KeyCode::Tab, _) => {
                self.surfaces.editor_field = (at + 1) % stops;
                return Some(vec![Effect::Redraw]);
            }
            (KeyCode::BackTab, _) => {
                self.surfaces.editor_field = (at + stops - 1) % stops;
                return Some(vec![Effect::Redraw]);
            }
            (KeyCode::Left | KeyCode::Right, None) => {
                let scopes = [EditScope::This, EditScope::ThisAndFollowing, EditScope::All];
                let now = scopes
                    .iter()
                    .position(|scope| Some(*scope) == editor.scope)
                    .unwrap_or(0);
                let next = if key.code == KeyCode::Right {
                    (now + 1) % 3
                } else {
                    (now + 2) % 3
                };
                return Some(self.tell(Told::EventEditorScope(scopes[next])));
            }
            (KeyCode::Char(character), Some(field)) if plain => {
                let mut text = typed(field);
                text.push(character);
                (field, text)
            }
            (KeyCode::Backspace, Some(field)) => {
                let mut text = typed(field);
                text.pop();
                (field, text)
            }
            (KeyCode::Enter, Some(EditorField::Notes)) if plain => {
                let mut text = typed(EditorField::Notes);
                text.push('\n');
                (EditorField::Notes, text)
            }
            (KeyCode::Enter, Some(_)) if plain => {
                return Some(self.tell(Told::Command(CommandId::SaveEvent)));
            }
            _ => {
                return Some(
                    match self.keys.press(key, KeyContext::Calendar, field.is_some()) {
                        Outcome::Command(id) => match id.parse::<CommandId>() {
                            Ok(command) => self.tell(Told::Command(command)),
                            Err(_) => Vec::new(),
                        },
                        _ => Vec::new(),
                    },
                );
            }
        };
        Some(self.tell(Told::EventEditorTyped {
            field: edit.0,
            text: edit.1,
        }))
    }

    pub(super) fn calendar_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        if let Some(effects) = self.editor_key(key) {
            return effects;
        }
        match self.keys.press(key, KeyContext::Calendar, false) {
            Outcome::Command(id) => match id.parse::<CommandId>() {
                Ok(command) => self.tell(Told::Command(command)),
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use postio_focus::{Reply, Request};
    use postio_model::calendar::{AgendaEntry, EventDetail, EventStatus};
    use postio_model::ids::{CalendarId, EventId};

    use crate::app::{App, Focus, update};
    use crate::test_support::{app, key, local, now, press, screen, serve_focus};
    use crossterm::event::{KeyCode, KeyModifiers};

    fn entry(id: i64, day: u32, hour: u32, title: &str) -> AgendaEntry {
        let start = local(day, hour, 0);
        AgendaEntry {
            event: EventId::new(id),
            calendar: CalendarId::new(1),
            calendar_name: "On this computer".to_owned(),
            colour: None,
            title: title.to_owned(),
            location: "Room 2".to_owned(),
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

    fn agenda(app: &mut App) {
        update(app, press('g'));
        let effects = update(app, press('e'));
        serve_focus(app, effects, |request| match request {
            Request::AgendaPage {
                stamp, placement, ..
            } => Some(Reply::AgendaPage {
                stamp: *stamp,
                placement: *placement,
                answer: Ok(vec![
                    entry(1, 23, 9, "Standup"),
                    entry(2, 23, 15, "Review"),
                    entry(3, 24, 10, "Dentist"),
                ]),
            }),
            _ => None,
        });
    }

    #[test]
    fn g_e_draws_agenda_under_its_days_and_return_opens_an_event_that_escape_closes() {
        let mut app = app((100, 24));
        app.clock_for_tests(now());
        agenda(&mut app);

        assert_eq!(app.focus(), Focus::Calendar);
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("Today · Wednesday 23 September"), "{drawn}");
        assert!(drawn.contains("15:00   Review"), "{drawn}");
        assert!(
            drawn.contains("Tomorrow · Thursday 24 September"),
            "{drawn}"
        );

        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        serve_focus(&mut app, effects, |request| match request {
            Request::EventDetail { stamp, .. } => Some(Reply::EventDetail {
                stamp: *stamp,
                answer: Ok(Some(Box::new(EventDetail {
                    entry: entry(2, 23, 15, "Review"),
                    notes: "Bring the numbers.".to_owned(),
                    organiser: None,
                    attendees: Vec::new(),
                    zone: None,
                    repeat: None,
                    minutes_before: Vec::new(),
                    writable: true,
                }))),
            }),
            _ => None,
        });
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("Bring the numbers."), "{drawn}");

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!screen(100, 24, &app).contains("Bring the numbers."));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn slash_in_calendar_searches_and_return_on_an_event_opens_it_in_calendar() {
        let mut app = app((100, 24));
        app.clock_for_tests(now());
        agenda(&mut app);

        update(&mut app, press('/'));
        assert_eq!(app.focus(), Focus::Bar, "the one search box");
        for letter in "den".chars() {
            update(&mut app, press(letter));
        }
        let asked = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        serve_focus(&mut app, asked, |request| match request {
            Request::EventSearch { stamp, text, .. } => {
                assert_eq!(text, "den");
                Some(Reply::EventSearch {
                    stamp: *stamp,
                    answer: Ok((vec![entry(3, 24, 10, "Dentist")], Vec::new())),
                })
            }
            _ => None,
        });
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("Events"), "{drawn}");
        assert!(drawn.contains("Dentist"), "{drawn}");

        let asked = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        serve_focus(&mut app, asked, |request| match request {
            Request::EventDetail { stamp, .. } => Some(Reply::EventDetail {
                stamp: *stamp,
                answer: Ok(Some(Box::new(EventDetail {
                    entry: entry(3, 24, 10, "Dentist"),
                    notes: "Bring the card.".to_owned(),
                    organiser: None,
                    attendees: Vec::new(),
                    zone: None,
                    repeat: None,
                    minutes_before: Vec::new(),
                    writable: true,
                }))),
            }),
            _ => None,
        });
        assert_eq!(app.focus(), Focus::Calendar);
        assert!(screen(100, 24, &app).contains("Bring the card."));
    }

    #[test]
    fn c_opens_the_editor_tab_moves_between_fields_and_ctrl_s_saves() {
        let mut app = app((100, 24));
        app.clock_for_tests(now());
        agenda(&mut app);

        update(&mut app, press('c'));
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("New event"), "{drawn}");
        assert!(
            drawn.contains("\u{25b8} Title"),
            "the title takes the typing: {drawn}"
        );

        for letter in "Dentist".chars() {
            update(&mut app, press(letter));
        }
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("  Title   Dentist"), "{drawn}");
        assert!(drawn.contains("\u{25b8} Starts"), "{drawn}");

        let effects = update(&mut app, key(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let saved = effects.iter().any(|effect| {
            matches!(
                effect,
                crate::app::Effect::Focus {
                    request: Request::Post(postio_core::Command::SaveEvent {
                        fields: Some(fields),
                        ..
                    }),
                    ..
                } if fields.title == "Dentist"
            )
        });
        assert!(saved, "{effects:?}");
        assert!(!screen(100, 24, &app).contains("New event"));
    }
}
