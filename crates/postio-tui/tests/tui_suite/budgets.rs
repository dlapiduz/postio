use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use postio_model::{FocusScope, ListScope};
use postio_tui::app::{App, Effect, Input, update};
use postio_tui::test_support::{
    app, buffer, key, places_with_features, press, row, seed_places, serve_with,
};

const MESSAGES: u32 = 100_000;

const SIZES: [(u16, u16); 2] = [(120, 36), (50, 12)];

fn opened(size: (u16, u16)) -> (App, Vec<Effect>) {
    let mut app = app(size);
    seed_places(&mut app, places_with_features());
    postio_tui::test_support::open_scope(&mut app, ListScope::Focus(FocusScope::Inbox), MESSAGES);
    let effects = update(&mut app, Input::Resize(size.0, size.1));
    (app, effects)
}

fn reads(effects: &[Effect]) -> (usize, u32, u32) {
    let mut pages = 0;
    let mut rows = 0;
    let mut reach = 0;
    for effect in effects {
        if let Effect::Focus {
            request: postio_focus::Request::Page { wanted, .. },
            ..
        } = effect
        {
            pages += 1;
            rows += wanted.limit;
            reach = reach.max(wanted.offset + wanted.limit);
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

fn content_reads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::ReadBody(_) | Effect::ReadSource(_)))
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
    // A keystroke costs at most one page and the one after it, which `ListWindow` reads with the
    // missing one, so a fast scroll does not stall at the next boundary.
    for size in SIZES {
        let (mut app, opening) = opened(size);
        serve_with(&mut app, opening, row);
        let mut crossings: Vec<(u32, Vec<u32>)> = Vec::new();
        for _ in 0..130 {
            let effects = update(&mut app, press('j'));
            let (pages, _, _) = reads(&effects);
            assert!(pages <= 2, "{size:?}: a keystroke read {pages} pages");
            if pages > 0 {
                let offsets = effects
                    .iter()
                    .filter_map(|effect| match effect {
                        Effect::Focus {
                            request: postio_focus::Request::Page { wanted, .. },
                            ..
                        } => Some(wanted.offset),
                        _ => None,
                    })
                    .collect();
                crossings.push((app.cursor(), offsets));
            }
            serve_with(&mut app, effects, row);
        }
        println!("{size:?}: held j for 130 rows, reads at {crossings:?}");
        assert_eq!(app.cursor(), 130);
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
        clash: None,
        moved_from: None,
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
    // The budget is for a release build: here the typical keystroke is held to it and the slow
    // tail to three times it.
    let budget = postio_test_support::scaled(std::time::Duration::from_millis(16));
    for size in SIZES {
        let (mut app, opening) = opened(size);
        serve_with(&mut app, opening, busy);
        let walk = (0..400).map(|step| if step % 90 < 60 { 'j' } else { 'k' });
        let walking = timed(&mut app, size, walk);
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
