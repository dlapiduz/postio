//! The digest window: the controller's view of one delivery, its summary and
//! its email, and the clicks that become inputs.

use postio_core::CommandId;
use postio_focus::{Host, Input as Told, Intent, SurfaceKind};
use postio_ui::digest::Page;
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};
use crate::digest::Window;
use crate::surface::Part;

impl App {
    pub fn digest(&self) -> Option<&Window> {
        self.surfaces.digest.as_ref()
    }

    pub(super) fn cursor_is_digest(&self) -> bool {
        self.row_at(self.cursor)
            .is_some_and(|row| row.kind == crate::row::Kind::Digest)
    }

    pub fn digest_geometry(&self) -> (u16, usize) {
        let area = ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1);
        let frame = crate::layout::open_frame(area);
        let inside = frame.width.saturating_sub(2);
        (
            crate::layout::column_width(frame.width).min(inside),
            usize::from(frame.height.saturating_sub(2)).saturating_sub(5),
        )
    }

    pub(super) fn show_digest_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::OpenDigest { .. } => {}
            Intent::Digest(view) => {
                if self.surfaces.digest.is_none() {
                    self.focus = Focus::Digest;
                }
                let email = view.email.is_some();
                match self.surfaces.digest.as_mut() {
                    Some(window) => window.replace(*view),
                    None => self.surfaces.digest = Some(Window::new(*view)),
                }
                if !email {
                    self.reading = None;
                }
                self.reveal_digest();
            }
            Intent::OpenMessage {
                message,
                host: Host::Digest,
                ..
            } => return Ok(self.read_digest_email(message)),
            Intent::CloseSurface(SurfaceKind::Digest) => {
                self.surfaces.digest = None;
                self.surfaces.confirm = None;
                self.reading = None;
                if self.focus == Focus::Digest {
                    self.focus = Focus::List;
                }
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    fn read_digest_email(&mut self, message: postio_model::MessageId) -> Vec<Effect> {
        let Some(line) = self.surfaces.digest.as_ref().and_then(|window| {
            window
                .view()
                .rows
                .iter()
                .find(|line| line.message == message)
        }) else {
            return Vec::new();
        };
        let row = crate::row::Row::from(line.summary.clone());
        self.reading = Some(crate::conversation::Reading {
            row: message,
            members: vec![crate::conversation::Member::from_row(&row)],
            current: 0,
            own: None,
        });
        vec![Effect::Redraw, Effect::ReadBody(message)]
    }

    pub(super) fn digest_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        match self.keys.press(key, KeyContext::Digest, false) {
            Outcome::Command(id) => self.command(&id),
            _ => Vec::new(),
        }
    }

    fn reveal_digest(&mut self) {
        let (column, height) = self.digest_geometry();
        let focus = crate::view::digest::focus_rows(self, column);
        let Some(window) = self.surfaces.digest.as_mut() else {
            return;
        };
        let height = height.max(1);
        let top = window.top();
        let wanted = match window.page() {
            Page::Summary => focus.map(|(start, end)| {
                if start < top {
                    start
                } else if end > top + height {
                    (end - height).min(start)
                } else {
                    top
                }
            }),
            Page::List => window.cursor().map(|at| {
                if at < top {
                    at
                } else if at >= top + height {
                    at + 1 - height
                } else {
                    top
                }
            }),
            Page::Email => None,
        };
        if let Some(wanted) = wanted {
            window.scroll_to(wanted);
        }
    }

    pub(super) fn digest_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let Some(window) = self.surfaces.digest.as_ref() else {
            return Vec::new();
        };
        match part {
            Part::DigestTab => {
                let page = if index == 0 {
                    Page::Summary
                } else {
                    Page::List
                };
                if window.page() == page {
                    return Vec::new();
                }
                self.tell(Told::Command(CommandId::ToggleDigestSummary))
            }
            // A second click on what is focused opens it.
            Part::DigestRow if window.cursor() == Some(index) => {
                self.tell(Told::Command(CommandId::OpenMessage))
            }
            Part::DigestRow => self.tell(Told::DigestPoint(index as u32)),
            Part::DigestReference if window.reference() == Some(index) => {
                self.tell(Told::Command(CommandId::OpenMessage))
            }
            Part::DigestReference => self.tell(Told::DigestReference(index as u32)),
            _ => Vec::new(),
        }
    }

    pub(super) fn digest_wheel(&mut self, lines: isize) {
        let (column, height) = self.digest_geometry();
        let extent = crate::view::digest::extent(self, column);
        if let Some(window) = self.surfaces.digest.as_mut() {
            let last = extent.saturating_sub(height.min(extent));
            window.scroll_to(window.top().saturating_add_signed(lines).min(last));
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::{Command, MessageTarget};
    use postio_focus::Request;
    use postio_model::MessageId;
    use postio_model::ids::DeliveryId;
    use postio_ui::digest::Page;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::test_support::{
        app, digest_row, held, key, newsletters_rule, places_with_features, press, screen,
        seed_places, serve_digest, show_focus, summary_of, with_model,
    };

    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        app.clock_for_tests(crate::test_support::now());
        let mut places = places_with_features();
        places.features.digests = crate::places::Rules(vec![newsletters_rule()]);
        places.features.section = crate::places::Section::new(with_model());
        seed_places(&mut app, places);
        show_focus(&mut app, vec![digest_row(1, "Newsletters", 14, 6)]);
        app
    }

    fn messages() -> Vec<postio_model::listing::MessageSummary> {
        vec![
            held(
                21,
                "Harbor Weekly",
                "Tide tables for October",
                "The October tide tables are out",
            ),
            held(
                22,
                "Rail Notes",
                "Timetable change",
                "From the 5th the 8:10 runs at 8:15",
            ),
            held(
                23,
                "Town Hall",
                "Bin collection",
                "Collections move to Thursday",
            ),
        ]
    }

    fn summary() -> postio_model::summary::DigestSummary {
        summary_of(&[
            ("Your harbor", "October's tide tables are published.", 21),
            (
                "Getting around",
                "The 8:10 train moves to 8:15 from the 5th.",
                22,
            ),
            ("Your town", "Bins are collected on Thursdays now.", 23),
        ])
    }

    fn opened(size: (u16, u16), with_summary: bool) -> App {
        let mut app = inbox(size);
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        serve_digest(&mut app, effects, messages(), with_summary.then(summary));
        app
    }

    fn email_body(message: i64) -> Input {
        Input::Body {
            message: MessageId::new(message),
            answer: Ok(postio_client::protocol::Body::Ready {
                body: postio_model::MessageBody {
                    text: Some("Hello.\nFrom the 5th the 8:10 runs at 8:15.\nThanks.".into()),
                    html: None,
                },
                encoding_problems: false,
            }),
        }
    }

    fn asked(effects: &[Effect], wanted: impl Fn(&Request) -> bool) -> bool {
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Focus { request, .. } if wanted(request)))
    }

    #[test]
    fn enter_on_a_digest_opens_its_window_on_the_summary_and_reads_the_delivery() {
        let mut app = inbox((120, 40));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::DigestRead { delivery, summary: true, .. } if delivery.get() == 1
        )));
        assert_eq!(app.focus(), Focus::Digest);
        serve_digest(&mut app, effects, messages(), Some(summary()));
        let drawn = screen(120, 40, &app);
        assert!(drawn.contains("Weekly · Newsletters"), "{drawn}");
        assert!(drawn.contains("Archive all 14"), "{drawn}");
        assert!(drawn.contains("Your harbor"), "{drawn}");
        assert!(
            drawn.contains("October's tide tables are published. [1]"),
            "{drawn}"
        );
        assert!(
            !drawn.contains("Collections move to Thursday"),
            "no list rows on the summary:\n{drawn}"
        );
    }

    #[test]
    fn brackets_step_the_references_and_tab_swaps_to_the_plain_list() {
        let mut app = opened((120, 40), true);
        update(&mut app, press(']'));
        assert_eq!(app.digest().unwrap().reference(), Some(1));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.digest().unwrap().page(), Page::List);
        let drawn = screen(120, 40, &app);
        assert!(
            drawn.contains("Rail Notes") && drawn.contains("Collections move to Thursday"),
            "{drawn}"
        );
        // The list's cursor followed the reference to its message.
        assert_eq!(app.digest().unwrap().cursor(), Some(1));
        update(&mut app, press('j'));
        assert_eq!(app.digest().unwrap().cursor(), Some(2));
    }

    #[test]
    fn without_a_summary_it_opens_on_the_plain_list() {
        let app = opened((120, 36), false);
        assert_eq!(app.digest().unwrap().page(), Page::List);
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Tide tables for October"), "{drawn}");
        assert!(!drawn.contains("Summary"), "{drawn}");
    }

    #[test]
    fn enter_on_a_reference_opens_its_email_in_the_window_and_escape_comes_back() {
        let mut app = opened((120, 40), true);
        update(&mut app, press(']'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(22))),
            "{effects:?}"
        );
        update(&mut app, email_body(22));
        let drawn = screen(120, 40, &app);
        assert!(drawn.contains("Timetable change"), "the subject: {drawn}");
        assert!(drawn.contains("Source 2 of 3"), "{drawn}");
        assert!(
            drawn.contains("From the 5th the 8:10 runs at 8:15."),
            "{drawn}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Digest);
        assert_eq!(app.digest().unwrap().reference(), Some(1));
        assert!(app.reading().is_none());
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert!(app.digest().is_none());
    }

    #[test]
    fn shift_e_archives_the_delivery_and_closes_the_window() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('e'));
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::Post(Command::ArchiveDigest { delivery, archived: true })
                    if *delivery == DeliveryId::new(1)
            )),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::List);
        assert!(app.digest().is_none());
    }

    #[test]
    fn d_asks_before_it_stops_digesting_and_only_return_does() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('D'));
        assert!(!asked(&effects, |_| true), "{effects:?}");
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Stop digesting harbor@example.com?"),
            "{drawn}"
        );
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            asked(&effects, |request| matches!(
                request,
                Request::Post(Command::StopDigestingSender {
                    target: MessageTarget::Messages(messages),
                    stopped: true,
                    ..
                }) if messages == &[MessageId::new(21)]
            )),
            "{effects:?}"
        );
        update(&mut app, press('D'));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!asked(&effects, |_| true), "{effects:?}");
        assert_eq!(app.focus(), Focus::Digest, "Escape answered the question");
    }

    #[test]
    fn shift_m_unsubscribes_through_the_controller() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('M'));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::Unsubscribe(message) if *message == MessageId::new(21)
        )));
    }

    #[test]
    fn every_part_is_a_click() {
        use crate::surface::Part;
        use crate::test_support::click;
        use crate::view::hit::Target;
        let mut app = opened((120, 40), true);
        update(
            &mut app,
            click(Target::Surface(Part::DigestReference, 2), false, false),
        );
        assert_eq!(app.digest().unwrap().reference(), Some(2));
        let effects = update(
            &mut app,
            click(Target::Surface(Part::DigestReference, 2), false, false),
        );
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(23))),
            "a second click opens it: {effects:?}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        update(
            &mut app,
            click(Target::Surface(Part::DigestTab, 1), false, false),
        );
        assert_eq!(app.digest().unwrap().page(), Page::List);
        update(
            &mut app,
            click(Target::Surface(Part::DigestRow, 2), false, false),
        );
        assert_eq!(app.digest().unwrap().cursor(), Some(2));
    }

    #[test]
    fn a_long_summary_scrolls_to_keep_the_focused_reference_in_view() {
        let mut app = inbox((120, 20));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let many: Vec<(&str, &str, i64)> = (0..30)
            .map(|at| ("Topic", "A statement that says something.", 21 + at % 3))
            .collect();
        serve_digest(&mut app, effects, messages(), Some(summary_of(&many)));
        for _ in 0..25 {
            update(&mut app, press(']'));
        }
        assert!(app.digest().unwrap().top() > 0);
        assert!(screen(120, 20, &app).contains("open the full email"));
    }

    #[test]
    fn what_the_model_wrote_never_reaches_the_terminal_as_a_control() {
        let mut app = inbox((120, 36));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let hostile = summary_of(&[("To\u{1b}[2Jpic", "Say \u{1b}]0;owned\u{7}hello", 21)]);
        serve_digest(&mut app, effects, messages(), Some(hostile));
        let drawn = screen(120, 36, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("hello"), "{drawn}");
    }
}
