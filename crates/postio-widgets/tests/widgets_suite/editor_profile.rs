use crate::support_compose::settle_until as settle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_widgets::composer::editor;
use postio_widgets::reader::BlobSource;
use webkit6::prelude::*;

struct Blobs(HashMap<String, (Vec<u8>, String)>);

impl BlobSource for Blobs {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self.0.get(content_id).cloned()
    }
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
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    let mut blobs = HashMap::new();
    // Encoded here, not hand-copied: a constant with a bad IDAT got a size from its header and
    // failed only when it painted (#2157).
    let pixels = glib::Bytes::from_owned(vec![255u8; 4]);
    let png = gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &pixels, 4)
        .save_to_png_bytes()
        .to_vec();
    blobs.insert("inline-one".to_owned(), (png, "image/png".to_owned()));

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

    assert_eq!(
        eval(
            &view,
            "document.queryCommandValue('defaultParagraphSeparator')"
        ),
        "p",
        "the profile script did not run before the first gesture could"
    );

    // Two layers hold this, the markup-JS setting and the shell CSP; neither alone is the defence.
    assert_ne!(
        eval(&view, "document.title"),
        "pwned",
        "a script tag in edited content executed"
    );

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

    let what = "the cid image to decode from the blob source";
    settle(what, || {
        let state = eval(
            &view,
            &crate::support_compose::image_state_js("document.getElementById('inline')"),
        );
        crate::support_compose::cid_image_decoded(&state, what)
    });
    assert_eq!(
        eval(
            &view,
            "String(document.getElementById('inline').naturalWidth)"
        ),
        "1",
        "the cid image decoded at the wrong size"
    );

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
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        eval(&view, "document.body.textContent"),
        "a link",
        "a link click navigated the editing surface away"
    );

    // ── and through all of it, the wire stayed silent ─────────────────────
    // POSTIO-FIXED-DEADLINE: absence cannot be polled for, so this window is
    // spent in full every run. Scaling it would buy no confidence and would
    // multiply the one cost the case actually has.
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut contacted = false;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if hit_rx.try_recv().is_ok() {
            contacted = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    stopping.store(true, Ordering::Relaxed);
    watcher.join().expect("the listener thread ends");
    assert!(
        !contacted,
        "something in the editing profile opened a network connection"
    );
}
