use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::MessageBody;
use postio_widgets::body_view::BodyView;
use postio_widgets::reader::Reader;

fn prepare() {
    crate::support_reader::prepare(&gdk::Display::default().unwrap());
}

fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + postio_test_support::scaled(how_long);
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

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
    // The view draws only the window it shows; wait for the redraw rather than pausing.
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    loop {
        settle_for(std::time::Duration::from_millis(10));
        let top = adjustment.value();
        let covered = view.document().is_some_and(|document| {
            let offset = document.text.char_at_top(top);
            document.text.clusters.iter().any(|c| c.range.end > offset)
        });
        if covered || std::time::Instant::now() >= deadline {
            break;
        }
    }
    adjustment.value()
}

fn reading(view: &BodyView) -> usize {
    let top = view.vadjustment().expect("the view scrolls").value();
    view.document().expect("a snapshot").text.char_at_top(top)
}

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

pub fn showing_a_messages_images_keeps_its_place() {
    prepare();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        postio_ui::allowlist::RemoteImageAllowList::default(),
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
    // Showing the images changes what is above the reader's place, so the scroll offset may move;
    // the words must not.
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
