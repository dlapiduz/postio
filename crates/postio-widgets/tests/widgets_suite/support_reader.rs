use gtk::glib;
use gtk::prelude::*;

/// Past its deadline a `BodyView` shows the plain text, so a content case given the product's 400
/// ms would be a stopwatch; the deadline has its own case.
pub fn reader_deadline() -> std::time::Duration {
    postio_test_support::scaled(std::time::Duration::from_secs(30))
}

/// 120 seconds before scaling: these wait on WebKit loading a document.
pub fn settle_until(what: &str, done: impl Fn() -> bool) {
    postio_test_support::settle_until_within(
        postio_test_support::scaled(std::time::Duration::from_secs(120)),
        what,
        || {
            while glib::MainContext::default().iteration(false) {}
            // A document held by a dead web process never arrives; fail now, naming the death.
            if let Some(reason) = postio_widgets::composer::web_process::take_death() {
                panic!("a WebKit web process died ({reason}) while waiting for {what}");
            }
        },
        done,
    );
}

/// A count is a guess: name what a case waits for with `settle_until`.
pub fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.iteration(false) {}
    }
}

pub fn prepare(display: &gtk::gdk::Display) {
    register_faces();
    postio_widgets::style::install(display);
}

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

pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}
