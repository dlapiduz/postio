//! What a keystroke and an open list cost the terminal, counted (FR-197,
//! FR-195, SC-017).
//!
//! The host's half -- a usable inbox in under 500 ms -- is
//! `postio-host`'s `startup_budget`. This is the terminal's: how many rows
//! an open reads for a mailbox of a hundred thousand, how many a held `j`
//! reads at a page's edge, how many bodies an open reads, and how long a
//! keystroke takes to be handled and drawn. The counts are of what `update`
//! asks the loop for, so they hold with no host and no terminal.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use postio_model::{FocusScope, ListScope};
use postio_tui::app::{App, Effect, Input, update};
use postio_tui::test_support::{
    app, buffer, key, places_with_features, press, row, seed_places, serve_with,
};
use postio_ui::paging::Fetch;

/// A mailbox no frontend may load.
const MESSAGES: u32 = 100_000;

/// The two sizes every surface must work at: a wide terminal, and the
/// smallest (FR-191).
const SIZES: [(u16, u16); 2] = [(120, 36), (50, 12)];

/// The Focus inbox of `MESSAGES` rows, opened, with nothing read yet.
fn opened(size: (u16, u16)) -> (App, Vec<Effect>) {
    let mut app = app(size);
    seed_places(&mut app, places_with_features());
    let effects = update(
        &mut app,
        Input::Opened {
            scope: ListScope::Focus(FocusScope::Inbox),
            total: MESSAGES,
        },
    );
    (app, effects)
}

/// How many pages `effects` read, and how many rows those pages hold, and
/// the last row any of them reaches.
fn reads(effects: &[Effect]) -> (usize, u32, u32) {
    let mut pages = 0;
    let mut rows = 0;
    let mut reach = 0;
    for effect in effects {
        if let Effect::Fetch {
            fetch: Fetch::Scope(request),
            ..
        } = effect
        {
            pages += 1;
            rows += request.limit;
            reach = reach.max(request.offset + request.limit);
        }
    }
    (pages, rows, reach)
}

fn bodies(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::ReadBody(_)))
        .count()
}

/// Everything that reads a message's content rather than the list.
fn content_reads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ReadBody(_)
                    | Effect::ReadConversation(_)
                    | Effect::ReadParts(_)
                    | Effect::ReadSource(_)
            )
        })
        .count()
}

#[test]
fn opening_the_inbox_reads_only_the_pages_in_view() {
    for size in SIZES {
        let (_, effects) = opened(size);
        let (pages, rows, reach) = reads(&effects);
        println!("{size:?}: {pages} pages, {rows} rows, reaching row {reach}");
        assert!(pages >= 1, "{size:?}: the first rows are asked for");
        assert!(
            pages <= 2 && rows <= 2 * 50 && reach <= 2 * 50,
            "{size:?}: a page for the view and one read ahead, no more: \
             {pages} pages, {rows} rows, up to row {reach}"
        );
        assert!(
            u64::from(rows) * 100 < u64::from(MESSAGES),
            "{size:?}: under one row in a hundred of the mailbox"
        );
    }
}

#[test]
fn j_held_across_a_page_boundary_reads_one_page_and_its_read_ahead() {
    // Spec 005's budget: a keystroke costs at most one page and the page
    // after it, which `ListWindow` reads with the one that is missing, so
    // a fast scroll does not stall at the next boundary.
    for size in SIZES {
        let (mut app, opening) = opened(size);
        serve_with(&mut app, opening, row);
        // The opening read rows 0 to 99; walking past them crosses the edge.
        let mut crossings: Vec<(u32, Vec<u32>)> = Vec::new();
        for _ in 0..130 {
            let effects = update(&mut app, press('j'));
            let (pages, _, _) = reads(&effects);
            assert!(pages <= 2, "{size:?}: a keystroke read {pages} pages");
            if pages > 0 {
                let offsets = effects
                    .iter()
                    .filter_map(|effect| match effect {
                        Effect::Fetch {
                            fetch: Fetch::Scope(request),
                            ..
                        } => Some(request.offset),
                        _ => None,
                    })
                    .collect();
                crossings.push((app.cursor(), offsets));
            }
            serve_with(&mut app, effects, row);
        }
        println!("{size:?}: held j for 130 rows, reads at {crossings:?}");
        assert_eq!(app.cursor(), 130);
        // The key that brings row 99's neighbour into view: the cursor's own
        // row, or a few before it on a short screen that shows rows below it.
        assert_eq!(crossings.len(), 1, "{size:?}: {crossings:?}");
        let (cursor, offsets) = &crossings[0];
        assert!(
            (90..=100).contains(cursor),
            "{size:?}: at the edge of what was read, not before: {crossings:?}"
        );
        assert_eq!(
            offsets,
            &vec![100, 150],
            "{size:?}: the page it needs and the one after, nothing else"
        );
    }
}

#[test]
fn a_cursor_move_reads_no_body_and_opening_reads_exactly_one() {
    for size in SIZES {
        let (mut app, opening) = opened(size);
        serve_with(&mut app, opening, row);
        for _ in 0..30 {
            let effects = update(&mut app, press('j'));
            assert_eq!(
                content_reads(&effects),
                0,
                "{size:?}: a cursor move reads no content"
            );
            serve_with(&mut app, effects, row);
        }
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(bodies(&effects), 1, "{size:?}: {effects:?}");
        assert_eq!(
            content_reads(&effects),
            1,
            "{size:?}: and nothing else: {effects:?}"
        );
        // The next message opens in the frame: one body more, not the first
        // again; leaving it reads nothing.
        let effects = update(&mut app, press('j'));
        assert_eq!(bodies(&effects), 1, "{size:?}: {effects:?}");
        assert_eq!(content_reads(&effects), 1, "{size:?}: {effects:?}");
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            content_reads(&effects),
            0,
            "{size:?}: closing reads nothing"
        );
    }
}

/// The heaviest row the list draws: unread, two lines for its marker, two
/// pills, an attachment and a count.
fn busy(position: u32) -> postio_tui::row::Row {
    use postio_model::listing::{MarkerKind, MarkerSummary};
    use postio_ui::label_colour::Rgb;
    let mut busy = row(position);
    busy.unread = true;
    busy.attachment = true;
    busy.count = 3;
    busy.preview = postio_ui::terminal::SafeText::new(
        "Hi, the final numbers are in and the totals match what we discussed on Tuesday",
    );
    busy.marker = Some(MarkerSummary {
        kind: MarkerKind::Question,
        when: None,
        excerpt: Some("Can you approve these by Friday so finance can close the quarter?".into()),
        answer: None,
        cancelled: false,
    });
    busy.labels = ["Atlas", "Harbor"]
        .iter()
        .map(|name| postio_tui::row::Pill {
            name: postio_ui::terminal::SafeText::new(name),
            colour: Rgb::new(0x35, 0x84, 0xe4),
        })
        .collect();
    busy
}

/// Press `keys` in turn, each timed from the key to the drawn buffer; the
/// timings, sorted.
fn timed(
    app: &mut App,
    size: (u16, u16),
    keys: impl Iterator<Item = char>,
) -> Vec<std::time::Duration> {
    let mut spent = Vec::new();
    for c in keys {
        let started = Instant::now();
        let effects = update(app, press(c));
        let _ = buffer(size.0, size.1, app);
        spent.push(started.elapsed());
        serve_with(app, effects, busy);
    }
    spent.sort();
    spent
}

#[test]
fn a_keystroke_is_handled_and_drawn_inside_its_sixteen_milliseconds() {
    // FR-197. The budget is for a release build; this runs in a debug build
    // beside other tests, so the typical keystroke is held to the budget and
    // the slow tail to three times it. Release measures about a fifth of
    // debug: under 1.3 ms at the 99th percentile at 120x36.
    let budget = postio_test_support::scaled(std::time::Duration::from_millis(16));
    for size in SIZES {
        let (mut app, opening) = opened(size);
        serve_with(&mut app, opening, busy);
        let walk = (0..400).map(|step| if step % 90 < 60 { 'j' } else { 'k' });
        let walking = timed(&mut app, size, walk);
        // Thirty rows marked, the bulk bar drawn, then extended on.
        let selecting = timed(
            &mut app,
            size,
            "x".chars().chain("Jx".chars().cycle().take(60)),
        );
        for (what, spent) in [("walking", &walking), ("selecting", &selecting)] {
            let median = spent[spent.len() / 2];
            let p99 = spent[spent.len() * 99 / 100];
            let worst = spent[spent.len() - 1];
            println!("{what} at {size:?}: median {median:?}, p99 {p99:?}, worst {worst:?}");
            assert!(
                median < budget,
                "{what} at {size:?}: a keystroke took {median:?} at the median, over {budget:?}"
            );
            assert!(
                p99 < 3 * budget,
                "{what} at {size:?}: p99 {p99:?} is over three times {budget:?}"
            );
        }
    }
}
