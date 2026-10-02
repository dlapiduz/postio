//! The conversation document's own CSS, asked of the renderer that draws it.
//!
//! These were `gtk_reader_styles` cases asked of WebKit through a probe
//! view (`webkit_probe`): each is a claim about what the cascade and layout
//! *mean*, which no markup test can see. The reader draws with this crate
//! now, so this crate answers them -- in pixels and text geometry, headless.

use std::collections::HashMap;

use crate::support::{LIGHT, render, request_for};
use postio_render::{Raster, RenderedDocument};
use postio_ui::reader::document::Sheet;
use postio_ui::reader::thread::{Entry, conversation_document};

fn entry<'a>(scope: &'a str, body: &'a str, styles: &'a str) -> Entry<'a> {
    Entry {
        scope,
        sender: "Ada Norwood",
        address: "ada@example.com",
        when: "09:14",
        preview: "the first line",
        expanded: true,
        latest: false,
        draft: false,
        mine: false,
        blocked: 0,
        body,
        styles,
        recipients: "",
        cc: "",
    }
}

fn draw(entries: &[Entry<'_>]) -> (RenderedDocument, Raster) {
    let html = conversation_document(entries, postio_body::RemoteImages::Blocked, Sheet::Theme);
    let doc = render(&request_for(html, LIGHT));
    let raster = postio_render::rasterize(&doc);
    (doc, raster)
}

fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * raster.width + x) * 4) as usize;
    [raster.rgba[at], raster.rgba[at + 1], raster.rgba[at + 2]]
}

/// Every colour in `rect`, with how many pixels wear it.
fn colours(raster: &Raster, rect: postio_render::Rect) -> HashMap<[u8; 3], usize> {
    let mut seen = HashMap::new();
    let clamp = |v: f64, max: u32| (v.max(0.0) as u32).min(max);
    for y in clamp(rect.y0, raster.height)..clamp(rect.y1, raster.height) {
        for x in clamp(rect.x0, raster.width)..clamp(rect.x1, raster.width) {
            *seen.entry(pixel(raster, x, y)).or_default() += 1;
        }
    }
    seen
}

/// The box the first match of `text` is drawn in.
fn rect_of(doc: &RenderedDocument, text: &str) -> postio_render::Rect {
    let range = doc
        .text
        .find(text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{text:?} is not in the document"));
    doc.text.rects(range)[0]
}

/// Can one message's `<style>` block restyle another message? (#1326)
///
/// The question ADR 0032 turns on: a conversation is one document, and the
/// containment rests on `postio_body::styles` rewriting every selector under
/// its own message's container. That is a claim about a parser whose tests
/// can only read the text it emits; whether the text means what it should
/// is a question for the engine.
#[test]
fn one_senders_stylesheet_cannot_restyle_another_message() {
    let red = [200, 0, 0];
    let hostile = "<style>p { color: rgb(200, 0, 0) } body { color: rgb(200, 0, 0) } \
                   .postio-blocked { display: none }</style><p>first first first</p>";
    let mine = postio_body::sanitize::sanitize_body_in(
        hostile,
        postio_body::RemoteImages::Blocked,
        Some("1"),
    );
    let theirs = postio_body::sanitize::sanitize_body_in(
        "<p>second second second</p>",
        postio_body::RemoteImages::Blocked,
        Some("2"),
    );
    let (doc, raster) = draw(&[
        entry("1", &mine.html, &mine.styles),
        entry("2", &theirs.html, &theirs.styles),
    ]);
    let red_in = |text: &str| {
        colours(&raster, rect_of(&doc, text))
            .iter()
            .filter(|(c, _)| c.iter().zip(red).all(|(a, b)| a.abs_diff(b) < 12))
            .map(|(_, n)| n)
            .sum::<usize>()
    };
    // The control, first: without it everything below passes when the
    // stylesheet is simply dropped, and FR-019 says a message renders as
    // its sender built it.
    assert!(
        red_in("first first") > 0,
        "the sender's own rule did not reach their own message"
    );
    assert_eq!(
        red_in("second second"),
        0,
        "one sender's `<style>` restyled another sender's message"
    );
}

/// The user's own message wears a mark a person can see (#1241): folded,
/// in the head's square; open, in its fill. Both differ from a
/// correspondent's.
#[test]
fn the_users_own_message_is_marked_in_the_document() {
    let entries = [(true, false), (false, false), (true, true), (false, true)];
    let entries: Vec<Entry<'_>> = ["1", "2", "3", "4"]
        .iter()
        .zip(entries)
        .map(|(scope, (mine, expanded))| Entry {
            mine,
            expanded,
            ..entry(scope, "<p>a body</p>", "")
        })
        .collect();
    let (doc, raster) = draw(&entries);
    let ground = pixel(&raster, raster.width - 1, 0);
    // The mark sits before each head's "From": the strip from the page's
    // left edge to the label, as tall as the label's line.
    let marks: Vec<Vec<[u8; 3]>> = doc
        .text
        .find("From")
        .into_iter()
        .map(|range| {
            let label = doc.text.rects(range)[0];
            let strip = postio_render::Rect::new(0.0, label.y0, label.x0, label.y1);
            let mut painted: Vec<[u8; 3]> = colours(&raster, strip)
                .into_iter()
                .filter(|(c, n)| *c != ground && *n > 3)
                .map(|(c, _)| c)
                .collect();
            painted.sort_unstable();
            painted
        })
        .collect();
    assert_eq!(marks.len(), 4, "four heads, four marks: {marks:?}");
    for (index, mark) in marks.iter().enumerate() {
        assert!(
            !mark.is_empty(),
            "message {} draws no mark at all",
            index + 1
        );
    }
    assert_ne!(
        marks[0], marks[1],
        "a folded message of the user's own draws the same mark as a correspondent's"
    );
    assert_ne!(
        marks[2], marks[3],
        "an open message of the user's own draws the same mark as a correspondent's"
    );
}

/// `From`, `To` and `Cc` line up, and so do their values (#1437).
#[test]
fn from_to_and_cc_share_a_column() {
    let (doc, _) = draw(&[Entry {
        recipients: "Quinn Abara <quinn.abara@example.net>",
        cc: "Grace Hopper <grace@example.com>",
        ..entry("1", "<p>a body</p>", "")
    }]);
    let rows = [
        ("From", "Ada Norwood"),
        ("To", "Quinn Abara"),
        ("Cc", "Grace Hopper"),
    ];
    let lefts: Vec<(f64, f64)> = rows
        .iter()
        .map(|(label, value)| (rect_of(&doc, label).x0, rect_of(&doc, value).x0))
        .collect();
    let (label, value) = lefts[0];
    for ((name, _), (l, v)) in rows.iter().zip(&lefts) {
        assert!(
            (l - label).abs() < 1.0,
            "{name}'s label starts at {l} and From's at {label}: one column ({lefts:?})"
        );
        assert!(
            (v - value).abs() < 1.0,
            "{name}'s value starts at {v} and From's at {value} ({lefts:?})"
        );
    }
    assert!(
        value > label,
        "the values sit beside their labels ({lefts:?})"
    );
}

/// Each message's accent edge is its own segment (#1688): three open
/// messages laid end to end must not read as one line down the page.
#[test]
fn each_messages_accent_breaks_before_the_next() {
    let (_, raster) = draw(&[
        entry("1", "<p>first</p>", ""),
        entry("2", "<p>second</p>", ""),
        entry("3", "<p>third</p>", ""),
    ]);
    let ground = pixel(&raster, raster.width - 1, 0);
    let painted = |x: u32, y: u32| pixel(&raster, x, y) != ground;
    // The accent is the most painted column at the left edge.
    let column = (0..60.min(raster.width))
        .max_by_key(|&x| (0..raster.height).filter(|&y| painted(x, y)).count())
        .expect("a raster");
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for y in 0..raster.height {
        if !painted(column, y) {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == y => *end = y,
            _ => runs.push((y, y)),
        }
    }
    let segments: Vec<(u32, u32)> = runs.into_iter().filter(|(a, b)| b - a > 10).collect();
    assert_eq!(
        segments.len(),
        3,
        "column {column} should carry three accent segments, one per message: {segments:?}"
    );
    for pair in segments.windows(2) {
        assert!(
            pair[1].0 - pair[0].1 >= 4,
            "one accent ends at {} and the next begins at {}: they read as one line",
            pair[0].1,
            pair[1].0
        );
    }
}

/// Can one message's inline styling paint over another message? (#1346)
///
/// Inheritance cannot carry a declaration into a sibling, but painting and
/// layout can: a `transform` paints outside its box without moving layout,
/// and a negative margin pulls what follows up under it. Asked as a person
/// experiences it -- what is drawn where the next message's words are --
/// through the path mail takes (sanitized, then contained), against the raw
/// markup as the control that proves each payload is hostile at all.
#[test]
fn one_senders_styling_cannot_paint_over_another_message() {
    const SECOND: &str = r#"<p>second probe</p>"#;
    for hostile in [
        r#"<div style="height:120px;transform:translateY(120px);background:#f0f">first</div>"#,
        r#"<div style="height:120px;margin-bottom:-120px;background:#f0f">first</div>"#,
    ] {
        let magenta_at_probe = |as_mail: bool| {
            let (first, second) = if as_mail {
                let first = postio_body::sanitize::sanitize_body_in(
                    hostile,
                    postio_body::RemoteImages::Blocked,
                    Some("1"),
                );
                let second = postio_body::sanitize::sanitize_body_in(
                    SECOND,
                    postio_body::RemoteImages::Blocked,
                    Some("2"),
                );
                (
                    postio_ui::reader::document::contain_body(&first.html),
                    postio_ui::reader::document::contain_body(&second.html),
                )
            } else {
                (
                    format!("<div>{hostile}</div>"),
                    format!("<div>{SECOND}</div>"),
                )
            };
            let html = postio_ui::reader::document::wrap_document(
                &format!("{first}{second}"),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            );
            let doc = render(&request_for(html, LIGHT));
            let raster = postio_render::rasterize(&doc);
            let magenta_in = |text: &str| {
                colours(&raster, rect_of(&doc, text))
                    .iter()
                    .filter(|(c, _)| c[0] > 230 && c[1] < 30 && c[2] > 230)
                    .map(|(_, n)| *n)
                    .sum::<usize>()
            };
            // Whatever holds the line must not do it by erasing the sender's
            // block: as mail, it is still drawn, inside its own message.
            assert!(
                !as_mail || magenta_in("first") > 0,
                "{hostile}: the sender's own block is not drawn at all"
            );
            magenta_in("second probe")
        };
        assert!(
            magenta_at_probe(false) > 0,
            "{hostile}: without the sanitizer and containers it did not reach \
             the next message, so it is not hostile enough to prove anything"
        );
        assert_eq!(
            magenta_at_probe(true),
            0,
            "{hostile}: one sender's content is drawn over another sender's mail"
        );
    }
}
