//! The digest window: opening a delivery from its row, the summary's
//! references, the plain list, an email inside the window, and `A`, `D`
//! and `U`.
//!
//! The state is `crate::digest`; the drawing is `view::digest`.

use crossterm::event::KeyEvent;
use postio_core::{Command, MessageTarget};
use postio_model::{DeliveryId, MessageId};
use postio_ui::digest::Page;
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus, Tone};
use crate::ask::{Answer, Ask};
use crate::digest::{Stop, Window};
use crate::surface::Part;

impl App {
    /// The digest window, while one is open.
    pub fn digest(&self) -> Option<&Window> {
        self.surfaces.digest.as_ref()
    }

    /// Remember the surfaced rows among `rows`, as the host's read of them
    /// would have: the harness opens lists without that read.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn note_surfaced(&mut self, rows: &[postio_ui::focus_list::FocusRow]) {
        self.surfaced = rows
            .iter()
            .filter(|row| matches!(row, postio_ui::focus_list::FocusRow::Digest(_)))
            .cloned()
            .collect();
    }

    /// Whether the list's cursor is on a digest.
    pub(super) fn cursor_is_digest(&self) -> bool {
        self.row_at(self.cursor)
            .is_some_and(|row| row.kind == crate::row::Kind::Digest)
    }

    /// The column an email or the summary is laid out in, and how many rows
    /// the page shows at once.
    pub fn digest_geometry(&self) -> (u16, usize) {
        let area = ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1);
        let frame = crate::layout::open_frame(area);
        let inside = frame.width.saturating_sub(2);
        (
            crate::layout::column_width(frame.width).min(inside),
            usize::from(frame.height.saturating_sub(2)).saturating_sub(5),
        )
    }

    /// `Enter` on a digest's row: its window, over the inbox, asking for
    /// what it holds.
    pub(super) fn open_digest(&mut self) -> Vec<Effect> {
        let Some(row) = self.row_at(self.cursor) else {
            return Vec::new();
        };
        let delivery = DeliveryId::new(-row.id.get());
        let found = self.surfaced.iter().find_map(|surfaced| match surfaced {
            postio_ui::focus_list::FocusRow::Digest(digest) if digest.delivery == delivery => {
                Some(digest.clone())
            }
            _ => None,
        });
        let window = match found {
            Some(digest) => {
                let when = self
                    .features
                    .digests
                    .0
                    .iter()
                    .find(|rule| rule.name.trim() == digest.rule.trim())
                    .and_then(postio_ui::digest::rule_when);
                Window::new(
                    delivery,
                    &digest.rule,
                    digest.cadence,
                    digest.count,
                    digest.senders.len(),
                    digest.at,
                    when,
                )
            }
            // Not among what the host surfaced (yet): what the row says.
            None => Window::new(
                delivery,
                row.subject.as_str(),
                None,
                row.count,
                0,
                row.when,
                None,
            ),
        };
        self.surfaces.digest = Some(window);
        self.focus = Focus::Digest;
        vec![Effect::Ask(Ask::Digest(delivery)), Effect::Redraw]
    }

    /// Close the window: the list is as it was.
    fn close_digest(&mut self) -> Vec<Effect> {
        self.surfaces.digest = None;
        self.reading = None;
        self.focus = Focus::List;
        vec![Effect::Redraw]
    }

    /// The delivery arrived.
    pub(super) fn digest_answered(&mut self, answer: Answer) -> Vec<Effect> {
        let Answer::Digest {
            delivery,
            messages,
            summary,
        } = answer
        else {
            return Vec::new();
        };
        let Some(window) = self.surfaces.digest.as_mut() else {
            return Vec::new();
        };
        if window.delivery != delivery {
            return Vec::new();
        }
        let mut effects = Vec::new();
        let messages = match messages {
            Ok(messages) => messages,
            Err(reason) => {
                effects.extend(self.say_as(Tone::Failed, &reason, None));
                Vec::new()
            }
        };
        let summary = summary.unwrap_or_else(|reason| {
            tracing::warn!(%reason, "could not read a digest's summary");
            None
        });
        if let Some(window) = self.surfaces.digest.as_mut() {
            window.read_in(messages, summary);
        }
        effects.push(Effect::Redraw);
        effects
    }

    /// A key with the keyboard in the digest window.
    pub(super) fn digest_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Outcome::Command(id) = self.keys.press(key, KeyContext::Digest, false) else {
            return Vec::new();
        };
        self.digest_command(&id).unwrap_or_default()
    }

    /// What the window does for `id`, or `None` when it is not its own.
    pub(super) fn digest_command(&mut self, id: &str) -> Option<Vec<Effect>> {
        let (column, height) = self.digest_geometry();
        let window = self.surfaces.digest.as_mut()?;
        // The question over `D` takes the keyboard until it is answered.
        if window.stopping().is_some() {
            return Some(match id {
                "open_message" => self.confirm_stop(),
                "back" => {
                    window.stop_asked(None);
                    vec![Effect::Redraw]
                }
                _ => Vec::new(),
            });
        }
        let page = window.page();
        match id {
            "next_message" | "prev_message" => {
                let by = if id == "next_message" { 1 } else { -1 };
                match page {
                    Page::Email => {
                        let message = window.source_by(by)?;
                        return Some(self.read_email(message));
                    }
                    Page::List => window.step(by),
                    Page::Summary => {
                        let top = window.top().saturating_add_signed(by);
                        window.scroll_to(top);
                    }
                }
            }
            "first_message" if page == Page::List => window.go_to(0),
            "last_message" if page == Page::List => window.go_to(usize::MAX),
            "next_reference" | "prev_reference" => {
                if page != Page::Summary {
                    return Some(Vec::new());
                }
                window.step_reference(if id == "next_reference" { 1 } else { -1 });
            }
            "toggle_digest_summary" => {
                window.toggle();
            }
            "open_message" => {
                let message = match page {
                    Page::Summary => window
                        .focused_reference()
                        .map(|reference| reference.message),
                    Page::List => window.rows().get(window.cursor()).map(|row| row.id),
                    Page::Email => None,
                };
                let Some(message) = message else {
                    return Some(Vec::new());
                };
                window.open_email(message, page);
                return Some(self.read_email(message));
            }
            "back" => {
                if window.back() {
                    self.reading = None;
                    return Some(vec![Effect::Redraw]);
                }
                return Some(self.close_digest());
            }
            "go_to_inbox" => return Some(self.close_digest()),
            "archive_thread" => {
                let delivery = window.delivery;
                let mut effects = self.close_digest();
                effects.insert(
                    0,
                    Effect::Send(Command::ArchiveDigest {
                        delivery,
                        archived: true,
                    }),
                );
                return Some(effects);
            }
            "stop_digesting_sender" => {
                let found = window.focused().map(|row| (row.id, row.address.clone()));
                let Some((message, Some(sender))) = found else {
                    return Some(Vec::new());
                };
                window.stop_asked(Some(Stop {
                    message,
                    sender: postio_ui::terminal::SafeText::new(&sender),
                }));
            }
            "unsubscribe" => {
                let message = window.focused().map(|row| row.id)?;
                return Some(vec![Effect::Unsubscribe(message)]);
            }
            "quit" | "undo" | "cheat_sheet" => return Some(self.global(id)),
            _ => return None,
        }
        self.reveal_digest(column, height);
        Some(vec![Effect::Redraw])
    }

    /// `Enter` on the question over `D`: stop digesting, now.
    fn confirm_stop(&mut self) -> Vec<Effect> {
        let Some(stop) = self
            .surfaces
            .digest
            .as_mut()
            .and_then(|window| window.stopping().cloned())
        else {
            return Vec::new();
        };
        if let Some(window) = self.surfaces.digest.as_mut() {
            window.stop_asked(None);
        }
        vec![
            Effect::Send(Command::StopDigestingSender {
                target: MessageTarget::Messages(vec![stop.message]),
                stopped: true,
                kept: None,
            }),
            Effect::Redraw,
        ]
    }

    /// Read `message` for the window's email page: its row is the window's.
    fn read_email(&mut self, message: MessageId) -> Vec<Effect> {
        let Some(row) = self
            .surfaces
            .digest
            .as_ref()
            .and_then(|window| window.row_of(message))
            .cloned()
        else {
            return Vec::new();
        };
        self.reading = Some(crate::conversation::Reading {
            row: message,
            members: vec![crate::conversation::Member {
                id: row.id,
                from: row.from.clone(),
                address: row.address.clone(),
                when: row.when,
                body: None,
                held_back: Default::default(),
                source: None,
                original: false,
                reader_view: false,
                images_allowed: false,
                asked: true,
                has_attachments: row.attachment,
                parts: Vec::new(),
                to: Vec::new(),
                cc: Vec::new(),
            }],
            current: 0,
        });
        vec![Effect::Redraw, Effect::ReadBody(message)]
    }

    /// Keep what the keyboard is on in view.
    fn reveal_digest(&mut self, column: u16, height: usize) {
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
            Page::List => {
                let at = window.cursor();
                Some(if at < top {
                    at
                } else if at >= top + height {
                    at + 1 - height
                } else {
                    top
                })
            }
            Page::Email => None,
        };
        if let Some(wanted) = wanted {
            window.scroll_to(wanted);
        }
    }

    /// A click on a part of the digest window.
    pub(super) fn digest_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let (column, height) = self.digest_geometry();
        let Some(window) = self.surfaces.digest.as_mut() else {
            return Vec::new();
        };
        match part {
            Part::StopCancel => {
                window.stop_asked(None);
                return vec![Effect::Redraw];
            }
            Part::StopConfirm => return self.confirm_stop(),
            _ => {}
        }
        if window.stopping().is_some() {
            return Vec::new();
        }
        match part {
            Part::DigestTab => window.show(if index == 0 {
                Page::Summary
            } else {
                Page::List
            }),
            Part::DigestRow => {
                if window.cursor() == index {
                    return self.digest_command("open_message").unwrap_or_default();
                }
                window.go_to(index);
            }
            Part::DigestReference => {
                if window.reference() == Some(index) {
                    return self.digest_command("open_message").unwrap_or_default();
                }
                window.focus_reference(index);
            }
            _ => return Vec::new(),
        }
        self.reveal_digest(column, height);
        vec![Effect::Redraw]
    }

    /// The wheel over the window's page.
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
    use postio_model::MessageId;
    use postio_model::ids::DeliveryId;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::ask::{Answer, Ask};
    use crate::test_support::{
        app, ctrl, digest_row, held, key, newsletters_rule, places_with_features, press, screen,
        seed_places, show_focus, summary_of,
    };

    /// The inbox with one digest of fourteen messages from six senders.
    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut places = places_with_features();
        places.features.digests = crate::places::Rules(vec![newsletters_rule()]);
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
            ("Your harbor", "A dredging notice covers the east quay.", 21),
            (
                "Getting around",
                "The 8:10 train moves to 8:15 from the 5th.",
                22,
            ),
            ("Your town", "Bins are collected on Thursdays now.", 23),
        ])
    }

    fn answer(
        messages: Vec<postio_model::listing::MessageSummary>,
        summary: Option<postio_model::summary::DigestSummary>,
    ) -> Input {
        Input::Answer(Answer::Digest {
            delivery: DeliveryId::new(1),
            messages: Ok(messages),
            summary: Ok(summary),
        })
    }

    /// The digest window, open on its delivery, with a summary or not.
    fn opened(size: (u16, u16), with_summary: bool) -> App {
        let mut app = inbox(size);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut app, answer(messages(), with_summary.then(summary)));
        app
    }

    fn line_with<'a>(screen: &'a str, wanted: &str) -> &'a str {
        screen
            .lines()
            .find(|line| line.contains(wanted))
            .unwrap_or_else(|| panic!("no line holds {wanted:?} in\n{screen}"))
    }

    #[test]
    fn enter_on_a_digest_opens_its_window_and_asks_what_it_holds() {
        let mut app = inbox((120, 36));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Ask(Ask::Digest(DeliveryId::new(1)))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::Digest);
    }

    #[test]
    fn the_header_says_what_it_is_and_the_tab_row_what_it_holds() {
        let app = opened((120, 36), true);
        let drawn = screen(120, 36, &app);
        assert!(
            line_with(&drawn, "Weekly · Newsletters").contains("Archive all 14 A"),
            "{drawn}"
        );
        assert!(
            drawn.contains("14 messages from 6 senders · came due today 11:00"),
            "{drawn}"
        );
        let tabs = line_with(&drawn, "Summary  14 messages");
        assert!(tabs.contains("Tab"), "{tabs}");
        assert!(
            tabs.contains("Weekly, Sunday 09:00 · Edit rule and cadence d"),
            "{tabs}"
        );
    }

    #[test]
    fn it_opens_on_the_summary_when_there_is_one_with_each_statement_ending_in_its_reference() {
        let app = opened((120, 40), true);
        let drawn = screen(120, 40, &app);
        assert!(drawn.contains("Your harbor"), "{drawn}");
        assert!(
            line_with(&drawn, "October's tide tables are published.").ends_with_ref(1),
            "{drawn}"
        );
        assert!(
            line_with(&drawn, "The 8:10 train moves").contains("[3]"),
            "{drawn}"
        );
        assert!(
            drawn.contains(
                "Written on this computer by the local model from these 14 messages only."
            ),
            "{drawn}"
        );
        assert!(
            !drawn.contains("Collections move to Thursday"),
            "no list rows:\n{drawn}"
        );
    }

    trait EndsWithRef {
        fn ends_with_ref(&self, number: u32) -> bool;
    }
    impl EndsWithRef for &str {
        fn ends_with_ref(&self, number: u32) -> bool {
            self.trim_end()
                .trim_end_matches('│')
                .trim_end()
                .ends_with(&format!("[{number}]"))
        }
    }

    #[test]
    fn the_focused_reference_has_its_message_card_under_its_paragraph() {
        let mut app = opened((120, 40), true);
        let drawn = screen(120, 40, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.contains("October's tide tables are published."))
            .unwrap();
        assert!(
            lines[at + 1].contains("Harbor Weekly · Tide tables for October ·"),
            "{drawn}"
        );
        assert!(
            lines[at + 2].contains("The October tide tables are out"),
            "{drawn}"
        );
        assert!(lines[at + 3].contains("↵ open the full email"), "{drawn}");
        // `]` moves on: the card goes with it.
        update(&mut app, press(']'));
        update(&mut app, press(']'));
        let drawn = screen(120, 40, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.contains("The 8:10 train moves"))
            .unwrap();
        assert!(
            lines[at + 1].contains("Rail Notes · Timetable change"),
            "{drawn}"
        );
        assert!(
            drawn.matches("↵ open the full email").count() == 1,
            "{drawn}"
        );
        update(&mut app, press('['));
        assert_eq!(app.digest().unwrap().reference(), Some(1));
    }

    #[test]
    fn the_focused_reference_is_reversed_so_it_shows_without_colour() {
        use ratatui::style::Modifier;
        let app = opened((120, 40), true);
        let buffer = crate::test_support::buffer(120, 40, &app);
        let drawn = screen(120, 40, &app);
        let y = drawn
            .lines()
            .position(|line| line.contains("October's tide tables are published."))
            .unwrap() as u16;
        let x = (0..120)
            .find(|x| buffer[(*x, y)].symbol() == "[")
            .expect("the reference");
        assert!(buffer[(x + 1, y)].modifier.contains(Modifier::REVERSED));
        let other = drawn
            .lines()
            .position(|line| line.contains("The 8:10 train moves"))
            .unwrap() as u16;
        let x = (0..120)
            .find(|x| buffer[(*x, other)].symbol() == "[")
            .unwrap();
        assert!(!buffer[(x + 1, other)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn with_no_summary_it_opens_on_the_plain_list_and_tab_does_nothing() {
        let mut app = opened((120, 36), false);
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Harbor Weekly") && drawn.contains("Tide tables for October"),
            "{drawn}"
        );
        assert!(!drawn.contains("Summary"), "{drawn}");
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.digest().unwrap().page(), postio_ui::digest::Page::List);
    }

    #[test]
    fn tab_switches_between_the_summary_and_the_list() {
        let mut app = opened((120, 36), true);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Rail Notes") && drawn.contains("Collections move to Thursday"),
            "{drawn}"
        );
        assert!(!drawn.contains("Written on this computer"), "{drawn}");
        update(&mut app, press('j'));
        assert_eq!(app.digest().unwrap().cursor(), 1);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert!(screen(120, 36, &app).contains("Written on this computer"));
    }

    #[test]
    fn enter_on_a_reference_opens_its_email_in_the_window_and_escape_goes_back_to_it() {
        let mut app = opened((120, 40), true);
        update(&mut app, press(']'));
        update(&mut app, press(']'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(22))),
            "{effects:?}"
        );
        update(
            &mut app,
            Input::Body {
                message: MessageId::new(22),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("Hello.\nFrom the 5th the 8:10 runs at 8:15.\nThanks.".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        let drawn = screen(120, 40, &app);
        assert!(
            drawn.contains("Timetable change"),
            "the subject is the title:\n{drawn}"
        );
        assert!(drawn.contains("Source 2 of 3"), "{drawn}");
        assert!(
            drawn.contains("Cited as 3 in the summary; the passage is highlighted."),
            "{drawn}"
        );
        assert!(
            drawn.contains("the summary in this same window."),
            "{drawn}"
        );
        assert!(
            drawn.contains("From the 5th the 8:10 runs at 8:15."),
            "{drawn}"
        );
        assert!(drawn.contains("‹ Summary Esc"), "{drawn}");
        // The window stays; Escape goes back to the same reference.
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Digest);
        assert_eq!(app.digest().unwrap().reference(), Some(2));
        assert!(screen(120, 40, &app).contains("Written on this computer"));
        // And Escape again closes it.
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert!(app.digest().is_none());
    }

    #[test]
    fn j_and_k_walk_the_digests_sources_while_an_email_is_open() {
        let mut app = opened((120, 40), true);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let effects = update(&mut app, press('j'));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(22))),
            "{effects:?}"
        );
        assert!(screen(120, 40, &app).contains("Source 2 of 3"));
        let effects = update(&mut app, press('k'));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(21))),
            "{effects:?}"
        );
        assert!(screen(120, 40, &app).contains("Source 1 of 3"));
    }

    #[test]
    fn an_email_opened_from_the_list_goes_back_to_the_list() {
        let mut app = inbox((120, 40));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            answer(
                messages(),
                Some(summary_of(&[("Your harbor", "Tides are out.", 21)])),
            ),
        );
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, press('j'));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let drawn = screen(120, 40, &app);
        assert!(
            drawn.contains("Source 2 of 3") && !drawn.contains("Cited as"),
            "{drawn}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.digest().unwrap().page(), postio_ui::digest::Page::List);
    }

    #[test]
    fn the_cited_passage_is_marked_in_the_email_by_more_than_colour() {
        use ratatui::style::Modifier;
        let mut app = inbox((120, 40));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let mut cited = summary_of(&[("Getting around", "The train moves.", 22)]);
        cited.statements[0].reference.excerpt = "the 8:10 runs at 8:15.".into();
        update(&mut app, answer(messages(), Some(cited)));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Input::Body {
                message: MessageId::new(22),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("From the 5th the 8:10 runs at 8:15.".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        let drawn = screen(120, 40, &app);
        let y = drawn
            .lines()
            .position(|l| l.contains("From the 5th"))
            .unwrap() as u16;
        let x = drawn
            .lines()
            .nth(y as usize)
            .unwrap()
            .find("the 8:10")
            .unwrap();
        let x = drawn.lines().nth(y as usize).unwrap()[..x].chars().count() as u16;
        let buffer = crate::test_support::buffer(120, 40, &app);
        assert!(
            buffer[(x, y)].modifier.contains(Modifier::UNDERLINED),
            "{:?}",
            buffer[(x, y)]
        );
        assert!(!buffer[(x - 6, y)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_long_summary_scrolls_to_keep_the_focused_reference_in_view() {
        let mut app = inbox((120, 20));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let many: Vec<(&str, &str, i64)> = (0..30)
            .map(|at| ("Topic", "A statement that says something.", 21 + at % 3))
            .collect();
        update(&mut app, answer(messages(), Some(summary_of(&many))));
        for _ in 0..25 {
            update(&mut app, press(']'));
        }
        assert!(app.digest().unwrap().top() > 0);
        assert!(screen(120, 20, &app).contains("↵ open the full email"));
    }

    #[test]
    fn a_archives_the_whole_delivery_and_closes() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('A'));
        assert!(
            effects.contains(&Effect::Send(Command::ArchiveDigest {
                delivery: DeliveryId::new(1),
                archived: true
            })),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn d_asks_before_it_stops_digesting_the_sender() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('D'));
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Send(_))),
            "{effects:?}"
        );
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("Stop digesting harbor@example.com?"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Their mail comes to the inbox again"),
            "{drawn}"
        );
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Send(Command::StopDigestingSender {
                target: MessageTarget::Messages(vec![MessageId::new(21)]),
                stopped: true,
                kept: None,
            })),
            "{effects:?}"
        );
        // Escape leaves the question and sends nothing.
        update(&mut app, press('D'));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Send(_))));
        assert_eq!(
            app.focus(),
            Focus::Digest,
            "Escape answered the question, not the window"
        );
    }

    #[test]
    fn u_leaves_the_focused_messages_list_and_ctrl_z_undoes() {
        let mut app = opened((120, 36), true);
        let effects = update(&mut app, press('U'));
        assert!(
            effects.contains(&Effect::Unsubscribe(MessageId::new(21))),
            "{effects:?}"
        );
        let effects = update(&mut app, ctrl('z'));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Send(command) if command.id() == postio_core::CommandId::Undo)),
            "{effects:?}"
        );
    }

    #[test]
    fn the_windows_bottom_row_names_its_keys() {
        let app = opened((120, 40), true);
        let drawn = screen(120, 40, &app);
        let keys = line_with(&drawn, "next / previous reference");
        assert!(keys.contains("] [ next / previous reference"), "{keys}");
        assert!(keys.contains("↵ open"), "{keys}");
        assert!(keys.contains("Tab summary / messages"), "{keys}");
        assert!(keys.contains("D stop digesting the sender"), "{keys}");
    }

    #[test]
    fn every_part_is_a_click() {
        use crate::surface::Part;
        use crate::test_support::{click, hits_of};
        use crate::view::hit::Target;
        let mut app = opened((120, 40), true);
        let hits = hits_of(120, 40, &app);
        let on = |wanted: Target| {
            (0..40).any(|y| (0..120).any(|x| hits.at(x, y).is_some_and(|hit| hit.target == wanted)))
        };
        for command in [
            "archive_thread",
            "back",
            "digest_rule",
            "toggle_digest_summary",
            "stop_digesting_sender",
        ] {
            assert!(on(Target::Command(command)), "{command}");
        }
        assert!(on(Target::Surface(Part::DigestTab, 0)) && on(Target::Surface(Part::DigestTab, 1)));
        assert!(on(Target::Surface(Part::DigestReference, 3)));
        // A click on a reference focuses it, a second opens it.
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
            effects.contains(&Effect::ReadBody(MessageId::new(22))),
            "{effects:?}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        // The tabs switch pages.
        update(
            &mut app,
            click(Target::Surface(Part::DigestTab, 1), false, false),
        );
        assert_eq!(app.digest().unwrap().page(), postio_ui::digest::Page::List);
        update(
            &mut app,
            click(Target::Surface(Part::DigestRow, 2), false, false),
        );
        assert_eq!(app.digest().unwrap().cursor(), 2);
        let effects = update(
            &mut app,
            click(Target::Command("archive_thread"), false, false),
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Send(Command::ArchiveDigest { .. })))
        );
    }

    #[test]
    fn what_the_model_wrote_never_reaches_the_terminal_as_a_control() {
        let mut app = inbox((120, 36));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let hostile = summary_of(&[("To\u{1b}[2Jpic", "Say \u{1b}]0;owned\u{7}hello", 21)]);
        update(&mut app, answer(messages(), Some(hostile)));
        let drawn = screen(120, 36, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("hello"), "{drawn}");
    }

    #[test]
    fn a_digest_not_among_the_surfaced_rows_still_opens() {
        let mut app = app((120, 36));
        seed_places(&mut app, places_with_features());
        let rows = vec![digest_row(1, "Newsletters", 14, 6)];
        // Opened without the host's read of what is surfaced.
        let shown: Vec<crate::row::Row> = rows.into_iter().map(crate::row::Row::from).collect();
        crate::test_support::show_rows(&mut app, &shown);
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Ask(Ask::Digest(DeliveryId::new(1)))),
            "{effects:?}"
        );
        let _ = FocusRow::Digest;
    }
}
