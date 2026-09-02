//! The reader's typefaces reach the web process, and the document does not
//! carry them (#768, ADR 0023).
//!
//! Every document used to inline ~1.21 MB of base64 `@font-face` data, on
//! every render, including an empty pane and every absent plate. The faces
//! are served over `postio-font:` now, so the document names them and the
//! engine fetches only the ones the page actually draws with.
//!
//! Two things are worth a real display and neither is visible from the
//! document string alone:
//!
//! * **The faces actually arrive.** A rule naming a URL nothing answers is a
//!   silent fall back to system sans — no error, no broken-image icon,
//!   nothing to notice. Asserting the engine *requested* a face is as close
//!   as a headless test gets to "the message is drawn in Postio's type".
//! * **What a second message costs.** ADR 0023 leaves the caching question
//!   open on purpose ("measure it; do not assert it"), because it decides
//!   how large the win is rather than whether there is one. This measures
//!   it and prints the number; run with `--nocapture` to read it.
//!
//! One test function, for the reason `gtk_reader.rs` gives. Skips without a
//! display. Nothing here touches the network: `postio-font:` is answered
//! in-process from compiled-in bytes.

use crate::pump;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::message::MessageBody;
use postio_ui::reader::document::{FACES, FONT_SCHEME};
use webkit6::prelude::*;

pub fn the_faces_are_fetched_over_the_scheme_and_not_carried_by_the_document() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 500);

    // No inline parts in this fixture: the subject is the fonts, and a
    // source that resolves nothing keeps `postio-cid:` out of the counts.
    let source: Rc<dyn BlobSource> = Rc::new(|_: &str| None);
    let reader = Reader::with_allowlist(
        source,
        RemoteImageAllowList::default(),
        scratch_path("allowlist"),
    );

    // Every subresource the engine actually asks for. The document is a
    // string a test can read; what the *engine* does with it is not, and
    // "the faces reached the web process" is a claim about the engine.
    let requested: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let requested_for_view = Rc::clone(&requested);
    reader
        .view()
        .connect_resource_load_started(move |_, resource, _| {
            if let Some(uri) = resource.uri() {
                requested_for_view.borrow_mut().push(uri.to_string());
            }
        });

    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    // ── a message renders, and its type arrives with it ──────────────────
    let finished = track_load_finished(&reader);
    reader.render(&body("<p>The tide gate interlock is fixed.</p>"), None);
    wait_for(&finished, Duration::from_secs(5));
    // A subresource fetch is not part of the document load, so it can still
    // be in flight when `load-changed` says finished.
    pump_for(Duration::from_millis(500));

    let first: Vec<String> = requested.borrow().clone();
    let font_requests = |seen: &[String]| -> Vec<String> {
        seen.iter()
            .filter(|uri| uri.starts_with(&format!("{FONT_SCHEME}:")))
            .cloned()
            .collect()
    };
    let first_faces = font_requests(&first);
    assert!(
        !first_faces.is_empty(),
        "the engine never asked for a single face, so the message is being \
         drawn in whatever sans the web process happens to have: {first:?}"
    );
    for uri in &first_faces {
        let name = uri
            .trim_start_matches(&format!("{FONT_SCHEME}:"))
            .trim_start_matches('/');
        assert!(
            FACES.iter().any(|face| face.name == name),
            "the engine asked for {uri}, which is not one of the vendored \
             faces — the rules and the table have drifted apart"
        );
    }

    // ── and the document itself stayed small ─────────────────────────────
    // The whole of #768: what a message change costs is the message, not the
    // typeface catalogue.
    let document = postio_ui::reader::document::document_for(
        "<p>The tide gate interlock is fixed.</p>",
        postio_body::RemoteImages::Blocked,
    );
    assert!(
        !document.contains("data:font/"),
        "the faces are travelling with the document again"
    );
    assert!(
        document.len() < 64 * 1024,
        "the document is {} bytes for a one-line message",
        document.len()
    );

    // ── what a second message costs ──────────────────────────────────────
    // Measured, not asserted (ADR 0023): whether WebKit caches a
    // custom-scheme subresource across document loads decides how large the
    // win is, not whether there is one. Either answer is fine here — the
    // number is the point, and it is written down in the ADR from this run.
    requested.borrow_mut().clear();
    let finished = track_load_finished(&reader);
    reader.render(&body("<p>A second message, different words.</p>"), None);
    wait_for(&finished, Duration::from_secs(5));
    pump_for(Duration::from_millis(500));

    let second_faces = font_requests(&requested.borrow());
    eprintln!(
        "#768 measurement: first render fetched {} face(s) ({:?}); \
         second render fetched {} ({:?})",
        first_faces.len(),
        first_faces,
        second_faces.len(),
        second_faces,
    );

    // The engine drew this message too, whether it refetched the faces or
    // served them from its cache. What must never happen is the faces going
    // missing between messages — that is the silent fallback again.
    assert!(
        second_faces.len() <= first_faces.len(),
        "a second message asked for more faces than the first: {second_faces:?}"
    );

    window.close();
}

fn body(html: &str) -> MessageBody {
    MessageBody {
        text: None,
        html: Some(html.to_owned()),
    }
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("postio-gtk-fonts-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.ini"))
}

fn track_load_finished(reader: &Reader) -> Rc<RefCell<bool>> {
    let done = Rc::new(RefCell::new(false));
    let flag = Rc::clone(&done);
    reader.view().connect_load_changed(move |_, event| {
        if event == webkit6::LoadEvent::Finished {
            *flag.borrow_mut() = true;
        }
    });
    done
}

fn wait_for(flag: &Rc<RefCell<bool>>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !*flag.borrow() && Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(*flag.borrow(), "the WebView never finished loading");
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
}
