//! What the reader and body-view cases share beyond `support`: the render
//! deadline for cases about content, the loop helpers, and the host window
//! setup a reader needs (the bundled faces and the shared sheet).

use gtk::glib;
use gtk::prelude::*;

/// The render deadline for cases that assert what a render *shows*.
///
/// Not the product's 400 ms. Past its deadline a `BodyView` shows the plain
/// text instead (FR-023), so a content case given the product deadline is
/// really asking "does this render finish in 400 ms on this machine right
/// now" -- a stopwatch, which this project does not ask of a shared runner.
/// The deadline itself has its own case, with its own 1 ms deadline.
pub fn reader_deadline() -> std::time::Duration {
    postio_test_support::scaled(std::time::Duration::from_secs(30))
}

/// Turn the GTK main loop until `done`, or fail saying what `what` was.
///
/// 120 seconds, scaled by `POSTIO_TEST_PATIENCE`: these wait on WebKit
/// loading a document, which is a different order of thing from "a widget
/// should have updated by now".
pub fn settle_until(what: &str, done: impl Fn() -> bool) {
    postio_test_support::settle_until_within(
        postio_test_support::scaled(std::time::Duration::from_secs(120)),
        what,
        || {
            while glib::MainContext::default().iteration(false) {}
            // A document held by a dead web process is never going to
            // arrive; fail now, naming the death.
            if let Some(reason) = postio_widgets::composer::web_process::take_death() {
                panic!("a WebKit web process died ({reason}) while waiting for {what}");
            }
        },
        done,
    );
}

/// Turn the main loop a fixed number of times, draining it each time.
/// A count is a guess: where a case knows what it waits for, it names it
/// with `settle_until`.
pub fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.iteration(false) {}
    }
}

/// What a window hosting a reader needs before it is built: the bundled
/// faces registered with the default font map and the shared sheet on the
/// display. Returns the display.
pub fn prepare(display: &gtk::gdk::Display) {
    register_faces();
    postio_widgets::style::install(display);
}

/// The bundled faces, unpacked once into a scratch directory and handed to
/// the default font map, as the apps do at startup.
fn register_faces() {
    use postio_ui::reader::document::FACES;
    let Some(font_map) = gtk::Label::new(None).pango_context().font_map() else {
        return;
    };
    let dir =
        std::env::temp_dir().join(format!("postio-widgets-test-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory for the faces");
    for face in FACES {
        let file = dir.join(face.name);
        std::fs::write(&file, face.bytes).expect("a face written to scratch");
        font_map
            .add_font_file(&file)
            .expect("pango should take the bundled face");
    }
}

/// Drain the main loop once.
pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}
