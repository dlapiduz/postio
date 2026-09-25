//! The reader's notices never move the message under them.
//!
//! A message can carry several notices -- remote images held back, reader
//! view, a decode caveat, the list it came from -- and each was a bar of its
//! own stacked above the body. Moving from a message with two of them to one
//! with none moved the body's first line up by two bars, and back down on
//! the next: the pane jumping on most cursor moves through a folder of
//! newsletters. And the waiting plate kept whatever notices the previous
//! message had, over a message they said nothing about.
//!
//! Asserted on geometry and on what is on screen, which is what a person
//! sees; nothing here asks what the reader was told.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::{Absent, Reader, RemoteImageAllowList};
use postio_gtk::{fonts, style};
use postio_model::address::EmailAddress;
use postio_model::message::MessageBody;
use postio_model::test_corpus;

fn reader_in_a_window() -> Option<(gtk::Window, Reader, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        dir.path().join("allow.json"),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    Some((window, reader, dir))
}

/// Let the window lay itself out: a size is only allocated on a frame.
fn lay_out() {
    let until = std::time::Instant::now()
        + postio_test_support::scaled(std::time::Duration::from_millis(250));
    while std::time::Instant::now() < until {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Where the body's top edge is, in the reader's own coordinates.
fn body_top(reader: &Reader) -> f32 {
    reader
        .view()
        .compute_bounds(&reader.widget())
        .expect("the body is laid out inside the reader")
        .y()
}

fn header(reader: &Reader) {
    reader.set_message_header(
        &[EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        &[EmailAddress::new(Some("Grace Hopper"), "grace@example.com")],
        &[],
        Some("Figures"),
        Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
    );
}

fn plain(text: &str) -> MessageBody {
    MessageBody {
        text: Some(text.to_owned()),
        html: None,
    }
}

pub fn the_body_starts_at_the_same_place_whatever_the_notices() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    let mut tops = Vec::new();

    // Nothing to say about it.
    header(&reader);
    reader.render(&plain("a plain message"), Some("ada@example.com"));
    lay_out();
    tops.push(("no notice", body_top(&reader)));

    // The list it came from.
    header(&reader);
    reader.render(&plain("a list message"), Some("ada@example.com"));
    reader.set_unsubscribe(Some("list.example.com"));
    lay_out();
    tops.push(("one notice", body_top(&reader)));

    // Images held back, a decode caveat, and the list: three at once.
    header(&reader);
    reader.render(
        &MessageBody {
            text: None,
            html: Some(
                "<p>Figures attached.</p><img src=\"https://images.invalid/chart.png\">".to_owned(),
            ),
        },
        Some("ada@example.com"),
    );
    reader.set_encoding_problems(true);
    reader.set_unsubscribe(Some("list.example.com"));
    lay_out();
    assert!(
        reader.banner_visible() || reader.shows_encoding_problems(),
        "the third message raised no notice at all, so this proves nothing"
    );
    tops.push(("three notices", body_top(&reader)));

    // And a message still waiting for its body.
    header(&reader);
    reader.show_absent(Absent::Partial);
    lay_out();
    tops.push(("the waiting plate", body_top(&reader)));

    let first = tops[0].1;
    assert!(
        tops.iter().all(|(_, top)| (*top - first).abs() < 0.5),
        "the body's first line moved between messages: {tops:?}"
    );

    window.close();
}

pub fn a_waiting_plate_carries_no_notice_from_the_message_before() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };

    // A newsletter: reader view, a decode caveat and the list, all raised.
    let newsletter = test_corpus::load("html-newsletter");
    let parsed = postio_model::mime::parse(newsletter.bytes());
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    reader.set_encoding_problems(true);
    reader.set_unsubscribe(Some("newsletter.example.com"));
    lay_out();
    assert!(
        reader.reader_notice_visible()
            || reader.shows_encoding_problems()
            || reader.unsubscribe_banner_visible(),
        "the newsletter raised no notice, so the plate below proves nothing"
    );

    // The next message has no body yet. None of that was about it.
    reader.show_absent(Absent::Partial);
    lay_out();
    let left = [
        ("reader view", reader.reader_notice_visible()),
        ("decode caveat", reader.shows_encoding_problems()),
        ("unsubscribe", reader.unsubscribe_banner_visible()),
        ("remote images", reader.banner_visible()),
    ];
    let still: Vec<&str> = left
        .iter()
        .filter(|(_, shown)| *shown)
        .map(|(name, _)| *name)
        .collect();
    assert!(
        still.is_empty(),
        "the waiting plate kept the previous message's notices: {still:?}"
    );

    window.close();
}
