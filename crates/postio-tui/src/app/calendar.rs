use postio_core::CommandId;
use postio_focus::{CalendarView, Input as Told, Intent, SurfaceKind};
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};

impl App {
    pub fn calendar(&self) -> Option<&CalendarView> {
        self.surfaces.calendar.as_deref()
    }

    pub fn calendar_typing(&self) -> bool {
        self.surfaces.calendar_typing.is_some()
    }

    pub(super) fn show_calendar_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::Calendar(view) => {
                self.surfaces.calendar = Some(view);
                self.focus = Focus::Calendar;
            }
            Intent::CalendarFocusFilter => {
                let typed = self
                    .calendar()
                    .map(|view| view.filter.clone())
                    .unwrap_or_default();
                self.surfaces.calendar_typing = Some(typed);
            }
            Intent::CloseSurface(SurfaceKind::Calendar) => {
                self.surfaces.calendar_typing = None;
                self.surfaces.calendar = None;
                if self.focus == Focus::Calendar {
                    self.focus = Focus::List;
                }
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub(super) fn calendar_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        if let Some(typed) = self.surfaces.calendar_typing.as_mut() {
            match key.code {
                KeyCode::Char(character) => typed.push(character),
                KeyCode::Backspace => {
                    typed.pop();
                }
                KeyCode::Enter => {
                    self.surfaces.calendar_typing = None;
                    return vec![Effect::Redraw];
                }
                KeyCode::Esc => {
                    self.surfaces.calendar_typing = None;
                    return self.tell(Told::Command(CommandId::Back));
                }
                _ => return Vec::new(),
            }
            let text = typed.clone();
            return self.tell(Told::CalendarFilterTyped(text));
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
    fn slash_types_a_search_that_narrows_agenda_and_escape_clears_it() {
        let mut app = app((100, 24));
        app.clock_for_tests(now());
        agenda(&mut app);

        update(&mut app, press('/'));
        let mut asked = Vec::new();
        for letter in "den".chars() {
            asked = update(&mut app, press(letter));
        }
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
        assert!(drawn.contains("/ den"), "{drawn}");
        assert!(
            drawn.contains("Dentist") && !drawn.contains("Review"),
            "{drawn}"
        );

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        let drawn = screen(100, 24, &app);
        assert!(drawn.contains("Review"), "the agenda is back:\n{drawn}");
        assert_eq!(app.focus(), Focus::Calendar);
    }
}
