//! The message list: day headings and Focus's rows.
//!
//! Only the rows in view are drawn, from the window's resident pages; a row
//! whose page is still on its way is blank, and asking for it is `App`'s
//! business, not the drawing's (Principle V: never a whole mailbox).
//!
//! A row is one line, or two when it carries a marker (terminal.md,
//! "Rows"): the cursor's `▌`, a mark, the sender, the subject with its
//! label pills and first line, and the trailing column. Rows have no rule
//! between them. Every state has a mark that is not a colour: `▌` the
//! cursor, `✓` selected, `●` unread, `≡` a digest, `↺` a reminder, `⎘` an
//! attachment.

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

/// What stands over a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heading {
    /// The day its mail arrived on.
    Day(NaiveDate),
    /// One heading for the whole list while a filter is on.
    Text(String),
}

/// One visible row of the list.
pub struct Visible<'a> {
    /// The row, or `None` while its page is on its way.
    pub row: Option<&'a Row>,
    /// Whether the cursor is here.
    pub cursor: bool,
    /// Whether the row is selected.
    pub selected: bool,
    /// The heading that starts here, on a line of its own above the row.
    pub heading: Option<Heading>,
}

/// Where the sender starts: the cursor, the mark and a space.
const SENDER_X: u16 = 3;
/// From this width the sender's column is [`SENDER_WIDE`].
const WIDE: u16 = 100;
const SENDER_WIDE: u16 = 20;
const SENDER_NARROW: u16 = 16;
/// The sender's column beside the reading pane.
const SENDER_BESIDE: u16 = 12;
/// The room the right edge keeps.
const MARGIN: usize = 0;

/// Where the subject starts in a list `width` cells wide, `beside` the
/// reading pane or not: after the cursor, the mark and the sender's column.
pub fn subject_x(width: u16, beside: bool) -> u16 {
    SENDER_X + sender_width(width, beside) + 1
}

/// How many lines `row` takes: one, or two with a marker. A row not here yet
/// is one.
pub fn lines_of(row: Option<&Row>) -> u16 {
    1 + u16::from(row.is_some_and(Row::two_lines))
}

/// The sender's column in a list `width` cells wide.
fn sender_width(width: u16, beside: bool) -> u16 {
    if beside {
        SENDER_BESIDE
    } else if width >= WIDE {
        SENDER_WIDE
    } else {
        SENDER_NARROW
    }
}

/// Draw `rows` into `area`, beside the reading pane or not. `first` is the list position of the first row,
/// for what a click on each means; `hint` is the key this terminal sends for
/// a command, for the answers a marked row offers.
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
            // Each drawn answer is its key, for this row.
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

/// The mark in front of the sender: what the row is, or whether it is
/// unread.
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

/// The trailing column: a draft's state, `⎘`, the count and the time.
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

/// A row's first line: the sender, then the subject, its pills and its
/// first line, then what trails.
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

/// What a marked row's second line offers to answer it, each as the words
/// and the key the keymap gives: nothing once the marker has a status.
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

/// The room a row's answers take on the right, with their gaps; what its
/// status takes when it has one.
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

/// A marked row's second line, under the subject: the chip, the date and the
/// quoted sentence, and on the right what answers it.
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
    // Under the subject, or under the sender once the sender's column has
    // narrowed.
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
    let accent = theme.style(Role::Accent);
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

/// A selection's mark: the selection's colour as the mark's own, rather than
/// a block behind it; without colour, the role's reversal.
fn selection_mark(theme: &Theme) -> Style {
    let selection = theme.style(Role::Selection);
    match selection.bg {
        Some(colour) => Style::default().fg(colour).add_modifier(Modifier::BOLD),
        None => selection,
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

    /// A small inbox: an unread invitation with a pill, a digest, a busy
    /// conversation with two pills and an attachment, a plain read one, and
    /// one from yesterday.
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
        // The invitation: a line, and under it the marker's line.
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
        // The digest is one line: its title, subject, line, count and time.
        assert!(lines[5].contains("≡ Weekly · digest"), "{drawn}");
        assert!(lines[5].contains("Newsletters · 14 messages"), "{drawn}");
        assert!(lines[5].contains("Summary of 14 messages"), "{drawn}");
        assert!(lines[5].trim_end().ends_with("14  10:00"), "{drawn}");
        // A plain row is one line; its pills and its count and clip come
        // before the time.
        assert!(lines[6].contains("● Ada Moreno"), "unread:\n{drawn}");
        assert!(lines[6].contains("●Atlas ●Budget"), "{drawn}");
        assert!(lines[6].contains("⎘ 3  09:51"), "{drawn}");
        assert!(lines[7].contains("Tomás Reyes"), "no second line:\n{drawn}");
        assert!(!lines[7].contains('●'), "read, so no unread mark:\n{drawn}");
        // The next day is under its own heading.
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

    fn sent(effects: &[crate::app::Effect]) -> Vec<postio_core::Command> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                crate::app::Effect::Send(command) => Some(command.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn y_and_shift_y_answer_the_invitation_the_cursor_is_on_with_its_message() {
        use crate::test_support::press;
        let mut app = inbox((120, 36));
        let accept = sent(&crate::app::update(&mut app, press('y')));
        assert_eq!(
            accept,
            vec![postio_core::Command::AcceptInvite {
                message: Some(postio_model::MessageId::new(1))
            }]
        );
        let decline = sent(&crate::app::update(&mut app, press('Y')));
        assert_eq!(
            decline,
            vec![postio_core::Command::DeclineInvite {
                message: Some(postio_model::MessageId::new(1))
            }]
        );
    }

    #[test]
    fn the_dash_takes_the_marker_off_the_row() {
        use crate::test_support::press;
        let mut app = inbox((120, 36));
        let effects = sent(&crate::app::update(&mut app, press('-')));
        assert!(
            matches!(
                effects.as_slice(),
                [postio_core::Command::DismissMarker {
                    dismissed: true,
                    ..
                }]
            ),
            "{effects:?}"
        );
    }

    /// The cell where `word` starts on the screen's first line holding it.
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
    fn a_click_on_a_drawn_action_does_what_its_key_does_for_that_row() {
        let mut app = inbox((120, 36));
        // The cursor is elsewhere: the invitation's action is still about
        // the invitation.
        crate::app::update(&mut app, crate::test_support::press('j'));
        crate::app::update(&mut app, crate::test_support::press('j'));
        let drawn = screen(120, 36, &app);
        let (x, y) = at_word(&drawn, "Decline");
        let effects = click_at(&mut app, x, y);
        assert_eq!(
            sent(&effects),
            vec![postio_core::Command::DeclineInvite {
                message: Some(postio_model::MessageId::new(1))
            }]
        );
        let (x, y) = at_word(&drawn, "Accept");
        assert_eq!(
            sent(&click_at(&mut app, x, y)),
            vec![postio_core::Command::AcceptInvite {
                message: Some(postio_model::MessageId::new(1))
            }]
        );
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
    fn a_selected_row_has_a_check_and_the_surface_across_it_and_the_cursor_keeps_its_bar() {
        use crate::caps::Colour;
        use crate::test_support::press;
        let mut app = inbox((120, 36));
        // Select the invitation (the cursor's row), then step to the next.
        crate::app::update(&mut app, press('x'));
        crate::app::update(&mut app, press('j'));
        for colour in [Colour::TrueColor, Colour::None] {
            let (drawn, theme) = cells(&app, colour);
            // The invitation is at lines 3 and 4; the cursor is on the digest.
            for y in [3, 4] {
                assert_eq!(drawn[(1, 3)].symbol(), "✓", "{colour:?}");
                let surface = theme.style(crate::theme::Role::Surface);
                for x in 0..drawn.area.width {
                    match surface.bg {
                        Some(bg) => assert_eq!(drawn[(x, y)].bg, bg, "{colour:?} ({x}, {y})"),
                        None => assert!(
                            drawn[(x, y)]
                                .modifier
                                .contains(ratatui::style::Modifier::REVERSED),
                            "{colour:?} ({x}, {y}) is not reversed"
                        ),
                    }
                }
                assert_ne!(drawn[(0, y)].symbol(), "▌", "the cursor is elsewhere");
            }
            assert_eq!(
                drawn[(0, 5)].symbol(),
                "▌",
                "{colour:?}: the digest has the cursor"
            );
            let reversed = drawn[(10, 5)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED);
            assert!(
                !reversed,
                "{colour:?}: the cursor alone does not change the row's ground"
            );
        }
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
