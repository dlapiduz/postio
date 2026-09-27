//! A document drawn again keeps the reader where they were.
//!
//! A conversation redrawn because a late body arrived -- the ordinary case
//! while a mailbox backfills -- used to throw whoever was half-way down it
//! back to the first message, and so did showing a message's images or its
//! original. The view keeps the character at its top where it was.
//!
//! Measured on the snapshot on screen: the character a person is reading,
//! and where on screen it sits.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::body_view::BodyView;
use postio_gtk::conversation::ConversationView;
use postio_gtk::list::Row;
use postio_gtk::reader::Reader;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::MessageBody;
use postio_model::ids::{MessageId, ThreadId};

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
    let deadline = std::time::Instant::now() + postio_test_support::scaled(how_long);
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Wait until `view` shows a snapshot whose text satisfies `ready` and
/// that is tall enough to scroll.
fn wait_laid_out(view: &BodyView, ready: impl Fn(&str) -> bool) {
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while std::time::Instant::now() < deadline {
        settle_for(std::time::Duration::from_millis(10));
        if let Some(document) = view.document()
            && ready(&document.text.text)
            && document.size.height - f64::from(view.height()) > 2000.0
        {
            return;
        }
    }
    panic!("the document never laid out tall enough to scroll");
}

fn scroll_to(view: &BodyView, y: f64) -> f64 {
    let adjustment = view.vadjustment().expect("the view scrolls");
    adjustment.set_value(y);
    settle_for(std::time::Duration::from_millis(50));
    adjustment.value()
}

/// The character at the top of the view: what a person is reading.
fn reading(view: &BodyView) -> usize {
    let top = view.vadjustment().expect("the view scrolls").value();
    view.document().expect("a snapshot").text.char_at_top(top)
}

/// Where the character at `offset` sits on screen, from the view's top.
fn on_screen(view: &BodyView, offset: usize) -> f64 {
    let document = view.document().expect("a snapshot");
    let top = view.vadjustment().expect("the view scrolls").value();
    document
        .text
        .clusters
        .iter()
        .filter(|c| c.range.end > offset)
        .min_by_key(|c| c.range.start)
        .map(|c| c.rect.y0 - top)
        .expect("the character is in the snapshot")
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
    wait_laid_out(&view, |text| text.contains("second:"));

    // Half-way down, reading.
    let scrolled = scroll_to(&view, 1500.0);
    assert!(
        (scrolled - 1500.0).abs() < 2.0,
        "the view did not scroll where it was asked: {scrolled}"
    );
    let offset = reading(&view);
    let reading_at = on_screen(&view, offset);

    // The third body lands and the conversation is drawn again.
    let loads = reader.loads();
    pane.set_thread_body(MessageId::new(3), long_body("third"));
    wait_laid_out(&view, |text| text.contains("third:"));
    assert!(
        reader.loads() > loads,
        "the late body never redrew the thread"
    );

    let after = on_screen(&view, offset);
    assert!(
        (after - reading_at).abs() < 2.0,
        "a late body moved the words being read from {reading_at}px to {after}px"
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
    wait_laid_out(&view, |_| true);
    assert!(
        reader.banner_visible(),
        "the remote image was not held back"
    );

    scroll_to(&view, 1500.0);
    // What the person is reading: the character at the top of the view, and
    // where on screen it sits. Showing the images changes what is *above*
    // it, so the scroll offset may move; the words must not.
    let offset = reading(&view);
    let reading_at = on_screen(&view, offset);

    let loads = reader.loads();
    let generation = view.document().expect("a snapshot").generation;
    reader.click_show_once();
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while (reader.loads() == loads || view.document().is_none_or(|d| d.generation == generation))
        && std::time::Instant::now() < deadline
    {
        settle_for(std::time::Duration::from_millis(10));
    }
    assert!(reader.loads() > loads, "showing the images never redrew");

    let after = on_screen(&view, offset);
    assert!(
        (after - reading_at).abs() < 2.0,
        "showing the images moved the words being read from {reading_at}px to {after}px"
    );

    window.close();
}
