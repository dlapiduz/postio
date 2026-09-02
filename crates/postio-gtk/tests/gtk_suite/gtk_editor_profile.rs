//! The editing profile, proven closed: #336, ADR 0003 hardening
//! requirements 1 and 3 as assertions.
//!
//! The composer's `WebView` is the one place JavaScript runs, and this file
//! is what keeps that from quietly meaning more than it says. It builds the
//! real profile from [`postio_gtk::editor`] and proves, against a live
//! WebKit:
//!
//! * the host's script runs (the profile script set the paragraph
//!   separator; `evaluate_javascript` answers);
//! * **markup-borne script does not** — a `<script>` seeded into the edited
//!   content leaves no trace, which is the mechanical form of "message
//!   content never executes";
//! * **nothing reaches the network** — a loopback listener this process owns
//!   sees no connection while content laden with remote references loads
//!   and is edited;
//! * `postio-cid:` images resolve from the local blob source, the one
//!   fetch-shaped thing the profile permits;
//! * a click on a link edits rather than navigates.
//!
//! One test function: GTK is single-threaded and initialised once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::editor;
use postio_gtk::reader::BlobSource;
use webkit6::prelude::*;

/// A blob source holding exactly what the test hands it.
struct Blobs(HashMap<String, (Vec<u8>, String)>);

impl BlobSource for Blobs {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self.0.get(content_id).cloned()
    }
}

fn settle(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "timed out waiting for {what}");
}

fn eval(view: &webkit6::WebView, script: &str) -> String {
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let slot = result.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            let value = outcome.expect("the script runs").to_str().to_string();
            *slot.borrow_mut() = Some(value);
        },
    );
    settle("the script to answer", || result.borrow().is_some());
    result.borrow_mut().take().unwrap()
}

pub fn the_editing_profile_runs_our_script_and_nothing_else() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    // ── the tripwire: a socket nothing may touch ──────────────────────────
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let port = listener.local_addr().unwrap().port();
    let (hit_tx, hit_rx) = mpsc::channel::<()>();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    listener
        .set_nonblocking(true)
        .expect("a nonblocking listener");
    let watcher = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            if listener.accept().is_ok() {
                let _ = hit_tx.send(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    let mut blobs = HashMap::new();
    // A real 1x1 PNG, so a resolved cid image has dimensions to assert on.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    blobs.insert(
        "inline-one".to_owned(),
        (png.to_vec(), "image/png".to_owned()),
    );

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let view = editor::editing_view(Rc::new(Blobs(blobs)));
    window.set_child(Some(&view));
    window.present();

    let loaded = Rc::new(RefCell::new(false));
    view.connect_load_changed({
        let loaded = loaded.clone();
        move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *loaded.borrow_mut() = true;
            }
        }
    });

    // Seed content that tries everything the profile forbids at once:
    // markup script (with a network side effect that doubles as its
    // tripwire), a remote image aimed at the listener, and one legitimate
    // cid image.
    editor::seed(
        &view,
        &format!(
            "<p>hello</p>\
             <script>document.title='pwned';fetch('http://127.0.0.1:{port}/script');</script>\
             <img src=\"http://127.0.0.1:{port}/pixel.gif\">\
             <img src=\"postio-cid:inline-one\" id=\"inline\">"
        ),
    );
    settle("the editing shell to load", || *loaded.borrow());

    // ── the host's script runs, and the profile script already did ────────
    assert_eq!(
        eval(
            &view,
            "document.queryCommandValue('defaultParagraphSeparator')"
        ),
        "p",
        "the profile script did not run before the first gesture could"
    );

    // ── markup script is inert markup ─────────────────────────────────────
    // Two layers hold this — the markup-JS setting and the shell CSP — and
    // the red-proof for the assertion requires breaking both, which is the
    // point: neither layer alone is the defence.
    assert_ne!(
        eval(&view, "document.title"),
        "pwned",
        "a script tag in edited content executed"
    );

    // With enable_javascript_markup off the seeded <script> must not have
    // executed. Its network fetch is caught by the listener below; here the
    // DOM-side proof: editing still works and no script side effect exists.
    let dialect = eval(
        &view,
        "(() => { \
           const sel = window.getSelection(); \
           const range = document.createRange(); \
           range.selectNodeContents(document.body.firstChild); \
           range.collapse(false); \
           sel.removeAllRanges(); sel.addRange(range); \
           document.execCommand('insertText', false, ' edited'); \
           return document.body.firstChild.textContent; \
         })()",
    );
    assert_eq!(dialect, "hello edited", "the surface is not editable");

    // ── the cid image resolved locally ────────────────────────────────────
    settle("the cid image to decode from the blob source", || {
        eval(
            &view,
            "String(document.getElementById('inline').naturalWidth)",
        ) == "1"
    });

    // ── a link click edits rather than navigates ──────────────────────────
    editor::seed(
        &view,
        &format!("<p><a href=\"http://127.0.0.1:{port}/away\">a link</a></p>"),
    );
    // load_html is a navigation the policy must keep allowing:
    settle("the reseeded shell to load", || {
        eval(&view, "document.body.textContent") == "a link"
    });
    eval(
        &view,
        "(() => { \
           const link = document.querySelector('a'); \
           link.dispatchEvent(new MouseEvent('click', {bubbles: true})); \
           return 'clicked'; \
         })()",
    );
    for _ in 0..20 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        eval(&view, "document.body.textContent"),
        "a link",
        "a link click navigated the editing surface away"
    );

    // ── and through all of it, the wire stayed silent ─────────────────────
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut contacted = false;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if hit_rx.try_recv().is_ok() {
            contacted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    stopping.store(true, Ordering::Relaxed);
    watcher.join().expect("the listener thread ends");
    assert!(
        !contacted,
        "something in the editing profile opened a network connection"
    );
}
