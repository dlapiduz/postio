//! A document drawn again keeps the reader where they were.
//!
//! Every document change is a full WebKit load, and `load_html` starts a
//! document at the top. So a conversation redrawn because a late body
//! arrived -- the ordinary case while a mailbox backfills -- threw whoever
//! was half-way down it back to the first message, and so did showing a
//! message's images or its original. The page cannot be told to keep its
//! place; the reader has to remember it and put it back.
//!
//! Measured in the engine, with `window.scrollY`: what a person sees is where
//! the page is, and nothing on this side of the process boundary knows that.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::conversation::ConversationView;
use postio_gtk::list::Row;
use postio_gtk::reader::Reader;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::MessageBody;
use postio_model::ids::{MessageId, ThreadId};
use webkit6::prelude::*;

fn display() -> bool {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    true
}

/// Pump the main loop for `how_long`, so timers and frames can happen.
fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + how_long;
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A number the engine computes in `view`, or NaN if it never answered.
fn number(view: &webkit6::WebView, script: &str) -> f64 {
    let answer = Rc::new(std::cell::Cell::new(f64::NAN));
    let slot = Rc::clone(&answer);
    view.evaluate_javascript(
        &format!("String({script})"),
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            if let Ok(value) = outcome {
                slot.set(value.to_str().parse().unwrap_or(f64::NAN));
            }
        },
    );
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while answer.get().is_nan() && std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(5));
    }
    answer.get()
}

/// Wait until `view` has finished loading and has somewhere to scroll.
fn wait_laid_out(view: &webkit6::WebView) {
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(10));
        if !view.is_loading()
            && number(view, "document.documentElement.scrollHeight - innerHeight") > 2000.0
        {
            return;
        }
    }
    panic!("the document never laid out tall enough to scroll");
}

fn long_body(name: &str) -> MessageBody {
    MessageBody {
        text: Some(format!("{name}: a paragraph of a message that goes on. ").repeat(300)),
        html: None,
    }
}

fn message(id: i64) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some("Tide gate interlock".to_owned()),
        preview: Some(format!("Snippet {id}")),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen: true,
        flagged: false,
        answered: false,
        send_state: None,
        send_at: None,
        has_attachments: false,
        thread_count: 3,
        participants: Vec::new(),
    }
}

pub fn a_late_body_does_not_throw_the_reader_back_to_the_top() {
    if !display() {
        return;
    }
    let pane = ConversationView::new();
    pane.set_one_document(true);
    pane.set_reader_factory(|_message| Some(Reader::new(Rc::new(|_content_id: &str| None))));
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&pane));
    window.present();

    // Two of three bodies are here; the third is late, which is what a
    // mailbox still backfilling looks like. The pane draws what it has once
    // its deadline passes.
    pane.open((1..=3).map(message).collect());
    pane.set_thread_body(MessageId::new(1), long_body("first"));
    pane.set_thread_body(MessageId::new(2), long_body("second"));
    let reader = pane.document_reader().expect("a document reader");
    let view = reader.view().clone();
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while !reader.test_document().contains("second:") && std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(20));
    }
    wait_laid_out(&view);

    // Half-way down, reading.
    number(&view, "(window.scrollTo(0, 1500), window.scrollY)");
    settle_for(std::time::Duration::from_millis(300));
    let reading_at = number(&view, "window.scrollY");
    assert!(
        (reading_at - 1500.0).abs() < 2.0,
        "the page did not scroll where it was asked: {reading_at}"
    );

    // The third body lands and the conversation is drawn again.
    let loads = reader.loads();
    pane.set_thread_body(MessageId::new(3), long_body("third"));
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while reader.loads() == loads && std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(10));
    }
    assert!(
        reader.loads() > loads,
        "the late body never redrew the thread"
    );
    wait_laid_out(&view);
    settle_for(std::time::Duration::from_millis(100));

    let after = number(&view, "window.scrollY");
    assert!(
        (after - reading_at).abs() < 2.0,
        "a late body moved the reader from {reading_at}px to {after}px"
    );

    window.close();
}

pub fn showing_a_messages_images_keeps_its_place() {
    if !display() {
        return;
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        postio_gtk::reader::allowlist::RemoteImageAllowList::default(),
        dir.path().join("allow.json"),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();

    let paragraph = "<p>A paragraph of a message that goes on and on. </p>".repeat(300);
    reader.render(
        &MessageBody {
            text: None,
            html: Some(format!(
                "<img src=\"https://images.invalid/banner.png\">{paragraph}"
            )),
        },
        Some("ada@example.com"),
    );
    let view = reader.view().clone();
    wait_laid_out(&view);
    assert!(
        reader.banner_visible(),
        "the remote image was not held back"
    );

    number(&view, "(window.scrollTo(0, 1500), window.scrollY)");
    settle_for(std::time::Duration::from_millis(300));
    let reading_at = number(&view, "window.scrollY");

    let loads = reader.loads();
    reader.click_show_once();
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while reader.loads() == loads && std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(10));
    }
    wait_laid_out(&view);
    settle_for(std::time::Duration::from_millis(100));

    let after = number(&view, "window.scrollY");
    assert!(
        (after - reading_at).abs() < 2.0,
        "showing the images moved the reader from {reading_at}px to {after}px"
    );

    window.close();
}
