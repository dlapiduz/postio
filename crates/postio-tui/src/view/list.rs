//! The message list: day headings and Focus's rows. Only the rows in view are drawn, from the
//! resident pages; a row whose page is on its way is blank, and `App` asks for it (never a whole
//! mailbox).

use chrono::{DateTime, Local, NaiveDate};
use postio_core::CommandId;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::row::{Kind, Row};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_ui::terminal::SafeText;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heading {
    Day(NaiveDate),
    Text(String),
}

pub struct Visible<'a> {
    pub row: Option<&'a Row>,
    pub cursor: bool,
    pub selected: bool,
    pub heading: Option<Heading>,
}

const SENDER_X: u16 = 3;
const WIDE: u16 = 100;
const SENDER_WIDE: u16 = 20;
const SENDER_NARROW: u16 = 16;
const SENDER_BESIDE: u16 = 12;
const MARGIN: usize = 0;

pub fn subject_x(width: u16, beside: bool) -> u16 {
    SENDER_X + sender_width(width, beside) + 1
}

/// A row whose page is not here yet is one line.
pub fn lines_of(row: Option<&Row>) -> u16 {
    1 + u16::from(row.is_some_and(Row::two_lines))
}

fn sender_width(width: u16, beside: bool) -> u16 {
    if beside {
        SENDER_BESIDE
    } else if width >= WIDE {
        SENDER_WIDE
    } else {
        SENDER_NARROW
    }
}

/// `first` is the list position of the first row; `hint` gives the key this terminal sends for a
/// command.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    beside: bool,
    rows: &[Visible],
    first: u32,
    theme: &Theme,
    now: DateTime<Local>,
    hint: &dyn Fn(CommandId) -> Option<String>,
    capture: bool,
    hits: &mut Hits,
) {
    let mut y = area.y;
    let end = area.y + area.height;
    for (offset, visible) in rows.iter().enumerate() {
        if let Some(heading) = &visible.heading {
            if y >= end {
                break;
            }
            let text = match heading {
                Heading::Day(day) => postio_ui::focus_row::day_heading(*day, now.date_naive()),
                Heading::Text(text) => text.clone(),
            };
            frame.render_widget(
                Line::styled(
                    format!(" {}", fit(&text, usize::from(area.width).saturating_sub(1))),
                    theme.style(Role::Text).add_modifier(Modifier::BOLD),
                ),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
        }
        let height = lines_of(visible.row);
        if y + height > end {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, height);
        if visible.selected && visible.row.is_some() {
            frame
                .buffer_mut()
                .set_style(rect, theme.style(Role::Surface));
        }
        if let Some(row) = visible.row {
            let bar = Span::styled(
                if visible.cursor { "▌" } else { " " },
                theme.style(Role::Focus),
            );
            frame.render_widget(
                first_line(
                    row,
                    visible,
                    bar.clone(),
                    usize::from(area.width),
                    beside,
                    theme,
                    now,
                ),
                Rect::new(area.x, y, area.width, 1),
            );
            if height == 2 {
                frame.render_widget(
                    second_line(
                        row,
                        bar,
                        usize::from(area.width),
                        beside,
                        theme,
                        now,
                        hint,
                        capture,
                    ),
                    Rect::new(area.x, y + 1, area.width, 1),
                );
            }
        }
        let position = first.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
        hits.add(rect, Target::Row(position));
        if height == 2
            && let Some(row) = visible.row
        {
            let mut x = area.x
                + u16::try_from(
                    usize::from(area.width)
                        .saturating_sub(MARGIN + answers_width(row, now, hint, capture)),
                )
                .unwrap_or(0);
            for (command, text) in answers(row, now, hint, capture) {
                let w = u16::try_from(text.width()).unwrap_or(u16::MAX);
                hits.add(
                    Rect::new(x, y + 1, w, 1),
                    Target::RowAction(position, command.as_str()),
                );
                x += w + 2;
            }
        }
        y += height;
    }
}

fn mark<'a>(row: &Row, visible: &Visible, theme: &Theme) -> Span<'a> {
    if visible.selected {
        return Span::styled("✓", selection_mark(theme));
    }
    match row.kind {
        Kind::Digest => Span::styled("≡", theme.style(Role::Text)),
        Kind::Reminder => Span::styled("↺", theme.style(Role::Accent)),
        Kind::Message if row.unread => Span::styled(
            "●",
            theme.style(if row.marker.is_some() {
                Role::Accent
            } else {
                Role::Text
            }),
        ),
        Kind::Message => Span::raw(" "),
    }
}

fn trailing(row: &Row, now: DateTime<Local>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(state) = row.send_state {
        parts.push(postio_ui::row::send_state_word(state).to_owned());
    }
    if row.attachment {
        parts.push("⎘".to_owned());
    }
    if let Some(count) = postio_ui::focus_row::count_badge(row.count) {
        parts.push(count);
    }
    let time = format!("{:>5}", postio_ui::row::timestamp(row.when, now));
    if parts.is_empty() {
        time
    } else {
        format!("{}  {time}", parts.join(" "))
    }
}

fn first_line<'a>(
    row: &Row,
    visible: &Visible,
    bar: Span<'a>,
    width: usize,
    beside: bool,
    theme: &Theme,
    now: DateTime<Local>,
) -> Line<'a> {
    let emphasis = if row.unread {
        theme.style(Role::Unread)
    } else {
        theme.style(Role::Text)
    };
    let sender_room = usize::from(sender_width(
        u16::try_from(width).unwrap_or(u16::MAX),
        beside,
    ));
    let sender = fit(row.from.as_str(), sender_room - 1);
    let sender_pad = sender_room.saturating_sub(sender.width());
    let trail = trailing(row, now);
    let used = usize::from(SENDER_X) + sender_room;
    let room = width.saturating_sub(used + trail.width() + 2 + MARGIN);

    let subject_style = if row.kind == Kind::Digest {
        theme.style(Role::Text).add_modifier(Modifier::BOLD)
    } else {
        emphasis
    };
    let subject = fit(row.subject.as_str(), room);
    let mut left = room.saturating_sub(subject.width());
    let mut spans = vec![
        bar,
        mark(row, visible, theme),
        Span::raw(" "),
        Span::styled(sender, emphasis),
        Span::raw(" ".repeat(sender_pad)),
        Span::styled(subject, subject_style),
    ];
    // The first line gives way first, then the pills: the sender, subject
    // and time always stay.
    for pill in row.labels.iter().take(postio_ui::focus_row::MAX_PILLS) {
        let need = 2 + pill.name.as_str().width();
        if need > left {
            break;
        }
        left -= need;
        spans.push(Span::raw(" "));
        spans.push(Span::styled("●", theme.label(pill.colour)));
        spans.push(Span::styled(
            pill.name.as_str().to_owned(),
            theme.style(Role::Dim),
        ));
    }
    if !row.preview.as_str().is_empty() && left >= 4 {
        let first = format!(" {}", fit(row.preview.as_str(), left - 1));
        left -= first.width();
        spans.push(Span::styled(first, theme.style(Role::Dim)));
    }
    spans.push(Span::raw(" ".repeat(left + 2)));
    spans.push(Span::styled(
        trail,
        if row.unread {
            theme.style(Role::Unread)
        } else {
            theme.style(Role::Dim)
        },
    ));
    Line::from(spans)
}

/// Nothing once the marker has a status.
fn answers(
    row: &Row,
    now: DateTime<Local>,
    hint: &dyn Fn(CommandId) -> Option<String>,
    capture: bool,
) -> Vec<(CommandId, String)> {
    let Some(marker) = &row.marker else {
        return Vec::new();
    };
    let line = postio_ui::focus_row::marker_line(marker, now.with_timezone(&chrono::Utc), &Local)
        .capturing(capture);
    if line.status.is_some() {
        return Vec::new();
    }
    line.actions
        .iter()
        .filter_map(|(command, word)| Some((*command, format!("{word} {}", hint(*command)?))))
        .collect()
}

fn answers_width(
    row: &Row,
    now: DateTime<Local>,
    hint: &dyn Fn(CommandId) -> Option<String>,
    capture: bool,
) -> usize {
    let answers = answers(row, now, hint, capture);
    answers.iter().map(|(_, text)| text.width()).sum::<usize>()
        + 2 * answers.len().saturating_sub(1)
}

#[allow(clippy::too_many_arguments)]
fn second_line<'a>(
    row: &Row,
    bar: Span<'a>,
    width: usize,
    beside: bool,
    theme: &Theme,
    now: DateTime<Local>,
    hint: &dyn Fn(CommandId) -> Option<String>,
    capture: bool,
) -> Line<'a> {
    let Some(marker) = &row.marker else {
        return Line::from(bar);
    };
    let line = postio_ui::focus_row::marker_line(marker, now.with_timezone(&chrono::Utc), &Local)
        .capturing(capture);
    let start = if !beside && u16::try_from(width).unwrap_or(u16::MAX) >= WIDE {
        usize::from(SENDER_X + SENDER_WIDE)
    } else {
        usize::from(SENDER_X)
    };
    let right: String = match line.status {
        Some(status) => status.to_owned(),
        None => answers(row, now, hint, capture)
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("  "),
    };
    let accent = theme.style(marker_role(&line));
    let room = width.saturating_sub(start + MARGIN + right.width() + 2);
    let mut spans = vec![bar, Span::raw(" ".repeat(start - 1))];
    let mut used = 0;
    let mut push = |spans: &mut Vec<Span<'a>>, text: String, style: Style| {
        let cut = fit(&text, room.saturating_sub(used));
        used += cut.width();
        spans.push(Span::styled(cut, style));
    };
    push(
        &mut spans,
        line.chip.to_owned(),
        accent.add_modifier(Modifier::BOLD),
    );
    if let Some(date) = &line.date {
        push(&mut spans, format!("  {date}"), accent);
    }
    if let Some(quote) = &line.quote {
        let quote = SafeText::new(quote);
        push(
            &mut spans,
            format!("  “{}”", quote.as_str()),
            accent.add_modifier(Modifier::ITALIC),
        );
    }
    let pad = width.saturating_sub(start + used + right.width() + MARGIN);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, theme.style(Role::Dim)));
    Line::from(spans)
}

/// The selection's colour on the mark itself; reversed without colour.
fn selection_mark(theme: &Theme) -> Style {
    let selection = theme.style(Role::Selection);
    match selection.bg {
        Some(colour) => Style::default().fg(colour).add_modifier(Modifier::BOLD),
        None => selection,
    }
}

pub(super) fn marker_role(line: &postio_ui::focus_row::MarkerLine) -> Role {
    if line.automated {
        Role::Dim
    } else {
        Role::Accent
    }
}

#[cfg(test)]
mod tests {
    use postio_model::listing::{Cadence, MarkerKind, MarkerSummary, MarkerWhen};
    use postio_ui::focus_list::{Conversation, Digest, FocusRow};

    use crate::app::App;
    use crate::test_support::{
        app, conversation, label, local, marked, places, screen, seed_places, show_focus, unread,
    };

    fn invite() -> MarkerSummary {
        MarkerSummary {
            kind: MarkerKind::Invite,
            when: Some(MarkerWhen::Event {
                starts_at: local(29, 10, 0),
                ends_at: local(29, 10, 45),
            }),
            excerpt: None,
            answer: None,
            cancelled: false,
            notice: false,
        }
    }

    fn with_labels(summary: postio_model::listing::ThreadSummary, names: &[&str]) -> FocusRow {
        FocusRow::Conversation(Conversation {
            summary,
            labels: names
                .iter()
                .enumerate()
                .map(|(at, name)| label(at as i64 + 1, name))
                .collect(),
        })
    }

    fn digest() -> FocusRow {
        FocusRow::Digest(Digest {
            delivery: postio_model::ids::DeliveryId::new(1),
            rule: "Newsletters".into(),
            cadence: Some(Cadence::Weekly),
            count: 14,
            senders: Vec::new(),
            summary_line: Some("Summary of 14 messages from 6 senders".into()),
            at: local(23, 10, 0),
        })
    }

    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        let mut grace = unread(marked(
            conversation(
                1,
                "Grace Oyelaran",
                "Invitation: Harbor design review",
                "Tue 29 Sep 10:00-10:45, Room 3B",
                local(23, 11, 2),
            ),
            invite(),
        ));
        grace.representative.send_state = None;
        let mut ada = unread(conversation(
            3,
            "Ada Moreno",
            "Re: Atlas Q3 budget, final numbers",
            "Hi, the final Q3 numbers are in",
            local(23, 9, 51),
        ));
        ada.message_count = 3;
        ada.has_attachments = true;
        show_focus(
            &mut app,
            vec![
                with_labels(grace, &["Harbor"]),
                digest(),
                with_labels(ada, &["Atlas", "Budget"]),
                FocusRow::conversation(conversation(
                    4,
                    "Tomás Reyes",
                    "Atlas staffing plan for Q4",
                    "Sharing the draft",
                    local(23, 9, 40),
                )),
                FocusRow::conversation(conversation(
                    5,
                    "Marco Ruiz",
                    "Cabinet order: please sign",
                    "Attached the final order",
                    local(22, 14, 31),
                )),
            ],
        );
        app
    }

    fn lines(drawn: &str) -> Vec<&str> {
        drawn.lines().collect()
    }

    #[test]
    fn rows_are_one_line_or_two_under_day_headings_with_no_rules() {
        let drawn = screen(120, 36, &inbox((120, 36)));
        let lines = lines(&drawn);
        assert!(
            lines[2].contains("Today · Wednesday 23 September"),
            "the day heading starts the list:\n{drawn}"
        );
        assert!(lines[3].contains("Grace Oyelaran"), "{drawn}");
        assert!(
            lines[3].contains("Invitation: Harbor design review"),
            "{drawn}"
        );
        assert!(lines[3].contains("●Harbor"), "its pill:\n{drawn}");
        assert!(lines[3].trim_end().ends_with("11:02"), "its time:\n{drawn}");
        assert!(
            lines[4].contains("Invite  Tue 29 Sep · 10:00–10:45"),
            "the chip and date under the subject:\n{drawn}"
        );
        assert!(
            lines[4].contains("Accept y  Decline Y"),
            "the answers, with the keys the keymap gives:\n{drawn}"
        );
        assert!(lines[5].contains("≡ Weekly · digest"), "{drawn}");
        assert!(lines[5].contains("Newsletters · 14 messages"), "{drawn}");
        assert!(lines[5].contains("Summary of 14 messages"), "{drawn}");
        assert!(lines[5].trim_end().ends_with("14  10:00"), "{drawn}");
        assert!(lines[6].contains("● Ada Moreno"), "unread:\n{drawn}");
        assert!(lines[6].contains("●Atlas ●Budget"), "{drawn}");
        assert!(lines[6].contains("⎘ 3  09:51"), "{drawn}");
        assert!(lines[7].contains("Tomás Reyes"), "no second line:\n{drawn}");
        assert!(!lines[7].contains('●'), "read, so no unread mark:\n{drawn}");
        assert!(
            lines[8].contains("Yesterday · Tuesday 22 September"),
            "{drawn}"
        );
        assert!(lines[9].contains("Marco Ruiz"), "{drawn}");
        assert!(!drawn.contains('─'), "no rule between rows:\n{drawn}");
    }

    #[test]
    fn the_trailing_column_ends_at_the_right_edge_on_short_rows_and_long() {
        for width in [50u16, 80, 120, 200] {
            let drawn = screen(width, 36, &inbox((width, 36)));
            let mut seen = 0;
            for line in drawn.lines().filter(|line| {
                ["Grace", "Weekly", "Ada Moreno", "Tomás", "Marco"]
                    .iter()
                    .any(|who| line.contains(who))
            }) {
                seen += 1;
                assert_eq!(
                    line.trim_end().chars().count(),
                    usize::from(width),
                    "the time ends at column {} on every row ({width} wide):\n{drawn}",
                    width - 1
                );
            }
            assert!(seen >= 5, "{drawn}");
        }
    }

    fn at_word(drawn: &str, word: &str) -> (u16, u16) {
        let (y, line) = drawn
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains(word))
            .unwrap_or_else(|| panic!("no {word} on:\n{drawn}"));
        let x = line[..line.find(word).unwrap()].chars().count();
        (u16::try_from(x).unwrap(), u16::try_from(y).unwrap())
    }

    fn click_at(app: &mut App, x: u16, y: u16) -> Vec<crate::app::Effect> {
        let hit = crate::test_support::hits_of(120, 36, app)
            .at(x, y)
            .expect("something is drawn there");
        crate::app::update(
            app,
            crate::app::Input::Pointer(crate::app::Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        )
    }

    #[test]
    fn a_notice_s_to_do_is_drawn_in_the_dim_ink() {
        use crate::caps::{Background, Colour};
        use crate::theme::{Role, Theme};
        let mut app = inbox((120, 36));
        let mut bill = conversation(9, "Harbour Bank", "Your statement", "", local(23, 8, 0));
        bill.marker = Some(MarkerSummary {
            kind: MarkerKind::Todo,
            when: None,
            excerpt: Some("A payment of $142.18 is due.".into()),
            answer: None,
            cancelled: false,
            notice: true,
        });
        show_focus(&mut app, vec![FocusRow::conversation(bill)]);
        let theme = Theme::new(Colour::TrueColor, Background::Dark, &Default::default()).0;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::view::draw(frame, &app, &theme, crate::test_support::now());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (x, y) = at_word(&screen(120, 36, &app), "To-do");
        let chip = &buffer[(x, y)];
        assert_eq!(Some(chip.fg), theme.style(Role::Dim).fg, "the chip");
        assert_ne!(Some(chip.fg), theme.style(Role::Accent).fg);
    }

    #[test]
    fn a_click_on_reply_opens_the_reply_to_that_row() {
        let mut app = inbox((120, 36));
        let mut ada = unread(conversation(9, "Ines Calvo", "Budget", "", local(23, 8, 0)));
        ada.marker = Some(MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you approve?".into()),
            answer: None,
            cancelled: false,
            notice: false,
        });
        show_focus(&mut app, vec![FocusRow::conversation(ada)]);
        let drawn = screen(120, 36, &app);
        let (x, y) = at_word(&drawn, "Reply");
        let effects = click_at(&mut app, x, y);
        assert!(
            effects.contains(&crate::app::Effect::ReplySource {
                kind: postio_body::replying::ReplyKind::Reply,
                message: postio_model::MessageId::new(9),
            }),
            "{effects:?}"
        );
    }

    #[test]
    fn an_answered_cancelled_or_past_invitation_shows_its_status_where_the_actions_were() {
        use postio_model::listing::InviteAnswer;
        for (answer, cancelled, word) in [
            (Some(InviteAnswer::Accepted), false, "Accepted"),
            (Some(InviteAnswer::Declined), false, "Declined"),
            (None, true, "Cancelled"),
        ] {
            let mut app = app((120, 36));
            seed_places(&mut app, places());
            let mut marker = invite();
            marker.answer = answer;
            marker.cancelled = cancelled;
            show_focus(
                &mut app,
                vec![FocusRow::conversation(marked(
                    unread(conversation(1, "Grace", "Review", "", local(23, 11, 2))),
                    marker,
                ))],
            );
            let drawn = screen(120, 36, &app);
            let line = drawn.lines().nth(4).unwrap();
            assert!(line.trim_end().ends_with(word), "{word}:\n{drawn}");
            assert!(!drawn.contains("Accept y"), "{drawn}");
        }
    }

    #[test]
    fn the_cursor_is_a_bar_on_every_line_of_its_row_and_nothing_else_changes() {
        let drawn = screen(120, 36, &inbox((120, 36)));
        let lines = lines(&drawn);
        assert!(lines[3].starts_with('▌'), "{drawn}");
        assert!(
            lines[4].starts_with('▌'),
            "both lines of a marked row:\n{drawn}"
        );
        assert!(
            lines.iter().filter(|line| line.starts_with('▌')).count() == 2,
            "{drawn}"
        );
    }

    #[test]
    fn the_first_line_gives_way_before_the_pills_and_the_sender_subject_and_time_stay() {
        let drawn = screen(50, 12, &inbox((50, 12)));
        let lines = lines(&drawn);
        let ada = lines
            .iter()
            .find(|line| line.contains("Ada Moreno"))
            .expect("the row is there");
        assert!(ada.contains("Re: Atlas"), "{drawn}");
        assert!(ada.trim_end().ends_with("09:51"), "{drawn}");
        assert!(
            !ada.contains("Hi, the final"),
            "the first line went first:\n{drawn}"
        );
    }

    #[test]
    fn hostile_text_in_a_row_reaches_the_screen_harmless() {
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let mut summary = unread(conversation(
            1,
            "Mallory\u{1b}[31m",
            "Pay now\u{1b}]0;owned\u{7}",
            "preview\u{9b}",
            local(23, 11, 0),
        ));
        summary.marker = Some(MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you\u{1b}[2J?".into()),
            answer: None,
            cancelled: false,
            notice: false,
        });
        show_focus(&mut app, vec![FocusRow::conversation(summary)]);
        let drawn = screen(120, 36, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{9b}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("Can you"), "{drawn}");
    }

    fn cells(
        app: &App,
        colour: crate::caps::Colour,
    ) -> (ratatui::buffer::Buffer, crate::theme::Theme) {
        use crate::caps::Background;
        let theme = crate::theme::Theme::new(colour, Background::Dark, &Default::default()).0;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::view::draw(frame, app, &theme, crate::test_support::now());
            })
            .unwrap();
        (terminal.backend().buffer().clone(), theme)
    }

    #[test]
    fn under_no_colour_every_state_keeps_a_mark_that_is_not_a_colour() {
        use crate::caps::Colour;
        use crate::test_support::local;
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let reminder = FocusRow::surfaced(&postio_model::listing::Surfaced::Reminder {
            reminder: postio_model::ids::ReminderId::new(1),
            thread: postio_model::ids::ThreadId::new(9),
            since: local(20, 9, 0),
            representative: conversation(9, "Ines", "Proposal", "", local(20, 9, 0)).representative,
            at: local(23, 8, 0),
            position: 0,
        })
        .expect("a reminder is a row");
        show_focus(
            &mut app,
            vec![
                reminder,
                digest(),
                FocusRow::conversation(unread(conversation(
                    1,
                    "Ada",
                    "Unread",
                    "",
                    local(23, 7, 0),
                ))),
            ],
        );
        let (drawn, _) = cells(&app, Colour::None);
        let text: Vec<String> = (0..drawn.area.height)
            .map(|y| {
                (0..drawn.area.width)
                    .map(|x| drawn[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect();
        assert!(text[3].starts_with("▌↺ Ines"), "{text:#?}");
        assert!(text[4].contains("No reply  since Sun 20 Sep"), "{text:#?}");
        assert!(text[5].starts_with(" ≡ Weekly"), "{text:#?}");
        assert!(
            text[6].starts_with("  ● Ada") || text[6].starts_with(" ● Ada"),
            "{text:#?}"
        );
    }
}
