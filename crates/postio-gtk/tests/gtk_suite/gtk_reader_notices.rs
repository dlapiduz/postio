//! The conversation pane puts the body where the single reader does (#1671):
//! the one case of the reader-notices family that needs the classic
//! conversation pane; the rest live in postio-widgets' `reader_notices`.
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
use postio_gtk::conversation::ConversationView;
use postio_gtk::list::Row as ListRow;
use postio_gtk::reader::{Reader, RemoteImageAllowList};
use postio_gtk::{fonts, style};
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_model::message::MessageBody;

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

fn plain(text: &str) -> MessageBody {
    MessageBody {
        text: Some(text.to_owned()),
        html: None,
    }
}

/// Where the body's top edge is in a conversation pane, in the pane's own
/// coordinates -- the same question [`body_top`] asks of the reader.
fn conversation_body_top(pane: &ConversationView) -> f32 {
    pane.document_reader()
        .expect("the one-document pane draws through a reader")
        .view()
        .compute_bounds(pane)
        .expect("the body is laid out inside the pane")
        .y()
}

/// A thread of `count` messages, `id` apart from every other thread here.
fn thread(id: i64, count: i64) -> Vec<ListRow> {
    (0..count)
        .map(|at| ListRow {
            id: MessageId::new(id * 100 + at),
            thread: Some(ThreadId::new(id)),
            from: Some(EmailAddress::new(
                Some(if at % 2 == 0 {
                    "Ada Lovelace"
                } else {
                    "Grace Hopper"
                }),
                if at % 2 == 0 {
                    "ada@example.com"
                } else {
                    "grace@example.com"
                },
            )),
            subject: Some("Figures".to_owned()),
            preview: Some("a plain message".to_owned()),
            received_at: Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap()
                + chrono::Duration::minutes(at),
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: count as u32,
            participants: Vec::new(),
        })
        .collect()
}

/// #1671: the single reader and the conversation pane put the body at the
/// same height.
///
/// Query views (Flagged, search results) open message rows in the reader
/// and folders open thread rows in the conversation pane, so a person moving
/// between them met a body that jumped by 64px on every change of surface.
/// The maintainer's call (2026-09-25) is one header, the reader's expanded
/// one, on both -- so this measures the body, not the header: it is the
/// body's first line a person's eye is on.
///
/// Across everything that could make one header taller than the other: a
/// `Cc` or none, an account line or none, a thread of one or of many (the
/// verbs' scoping note, the participants), and a window narrow enough that
/// the conversation's counter stands in for its rail.
pub fn the_body_starts_at_the_same_place_in_the_reader_and_the_conversation() {
    let Some((reader_window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    let pane = ConversationView::new();
    pane.set_one_document(true);
    pane.set_reader_factory(|_message| Some(Reader::new(Rc::new(|_content_id: &str| None))));
    let pane_window = gtk::Window::new();
    pane_window.set_child(Some(&pane));
    pane_window.present();

    let ada = EmailAddress::new(Some("Ada Lovelace"), "ada@example.com");
    let grace = EmailAddress::new(Some("Grace Hopper"), "grace@example.com");
    let bob = EmailAddress::new(None::<&str>, "bob@example.com");
    let date = Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap();

    let mut tops = Vec::new();
    let mut next_thread = 1;
    for (width, height) in [(900, 700), (1400, 800)] {
        reader_window.set_default_size(width, height);
        pane_window.set_default_size(width, height);
        pane.set_window_width(width);
        for cc in [vec![], vec![bob.clone()]] {
            for account in [None, Some(("Work", 1))] {
                let case = format!(
                    "{width}px, {} Cc, {}",
                    if cc.is_empty() { "no" } else { "a" },
                    if account.is_some() {
                        "account line"
                    } else {
                        "one account"
                    }
                );

                reader.set_message_header(
                    std::slice::from_ref(&ada),
                    std::slice::from_ref(&grace),
                    &cc,
                    Some("Figures"),
                    date,
                );
                reader.set_account(account.map(|(name, _)| name), account.map_or(0, |(_, h)| h));
                reader.render(&plain("a plain message"), Some("ada@example.com"));
                lay_out();
                tops.push((format!("reader, {case}"), body_top(&reader)));

                for count in [1, 6] {
                    let rows = thread(next_thread, count);
                    next_thread += 1;
                    let before = pane.thread_renders();
                    pane.open(rows.clone());
                    for row in &rows {
                        pane.set_thread_envelope(
                            row.id,
                            std::slice::from_ref(&grace),
                            &cc,
                            account,
                        );
                        pane.set_thread_body(row.id, plain("a plain message"));
                    }
                    crate::settle_until("the conversation drew", || pane.thread_renders() > before);
                    lay_out();
                    // The same header, so the same lines: who the newest
                    // message went to, and the account under the reader's
                    // rule -- not an empty row kept for its height.
                    let header = pane.header();
                    assert!(
                        header.to_visible(),
                        "{case}: no To line in the thread's header"
                    );
                    assert_eq!(
                        header.cc_toggle_visible(),
                        !cc.is_empty(),
                        "{case}: the Cc disclosure disagrees with the newest message"
                    );
                    assert_eq!(
                        header.account_label().as_deref(),
                        account.map(|(name, _)| name),
                        "{case}: the account line disagrees with the reader's rule"
                    );
                    tops.push((
                        format!("conversation of {count}, {case}"),
                        conversation_body_top(&pane),
                    ));
                }
            }
        }
    }

    // Grouped by the account line, and only by it: whether a second
    // account is configured is a fact about the installation, not about
    // the message, so it holds for a whole session -- and the reader's own
    // header has always been one line taller with it. Everything else --
    // the surface, the Cc, the thread's length, the width -- must not
    // move the body at all.
    for named in [false, true] {
        let group: Vec<&(String, f32)> = tops
            .iter()
            .filter(|(case, _)| case.ends_with("account line") == named)
            .collect();
        let first = group[0].1;
        assert!(first > 0.0, "the body was never laid out: {group:#?}");
        assert!(
            group.iter().all(|(_, top)| (*top - first).abs() < 0.5),
            "the body starts at a different height in the reader and the \
             conversation pane: {group:#?}"
        );
    }

    pane_window.close();
    reader_window.close();
}
