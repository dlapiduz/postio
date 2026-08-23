//! The reading pane on a real display: `postio-lu6` and `postio-1bz`,
//! against the corpus fixtures they exist for.
//!
//! One test function, for the reason `gtk_shell.rs` gives — GTK is
//! single-threaded and initialised once. Skips without a display. The
//! network-isolation case is the one part of this file that *does* touch a
//! socket: a listener on `127.0.0.1` this process owns, there only to prove
//! nothing else ever connects to it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::{BlobSource, Reader, RemoteImages};
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use webkit6::prelude::*;

#[test]
fn the_reader_renders_and_hardens_the_corpus() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 500);

    // ── inline cid: images resolve locally, including the dangling one ────
    let inline = test_corpus::load("inline-image-cid");
    let parsed = postio_model::mime::parse(inline.bytes());
    let mut blobs = HashMap::new();
    for part in &parsed.parts {
        if let Some(content_id) = &part.attachment.content_id {
            blobs.insert(
                content_id.clone(),
                (part.content.clone(), part.attachment.mime_type.clone()),
            );
        }
    }
    assert_eq!(blobs.len(), 2, "the fixture carries two real inline parts");

    let requested: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let requested_for_source = Rc::clone(&requested);
    let source: Rc<dyn BlobSource> = Rc::new(move |content_id: &str| {
        requested_for_source
            .borrow_mut()
            .push(content_id.to_owned());
        blobs.get(content_id).cloned()
    });

    let reader = Reader::new(source);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, RemoteImages::Blocked);
    wait_for(&finished, Duration::from_secs(5));

    let seen = requested.borrow().clone();
    assert!(
        seen.iter().any(|id| id == "reader-left.44b1@example.com"),
        "the left image's cid should have been resolved: {seen:?}"
    );
    assert!(
        seen.iter().any(|id| id == "reader-right.44b1@example.com"),
        "the right image's cid should have been resolved: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(|id| id == "missing-signature.44b1@example.com"),
        "the dangling cid: reference should still reach the scheme handler: {seen:?}"
    );

    // ── a newsletter with script, remote CSS and no consent loads clean ────
    let newsletter = test_corpus::load("html-newsletter");
    let parsed = postio_model::mime::parse(newsletter.bytes());
    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, RemoteImages::Blocked);
    wait_for(&finished, Duration::from_secs(5));

    let tracking = test_corpus::load("html-tracking-pixel-remote-images");
    let parsed = postio_model::mime::parse(tracking.bytes());
    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, RemoteImages::Blocked);
    wait_for(&finished, Duration::from_secs(5));

    // ── quoted-text folding: the corpus's flowed reply ─────────────────────
    let flowed = test_corpus::load("plain-text-flowed-reply");
    let parsed = postio_model::mime::parse(flowed.bytes());
    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains('>')),
        "sanity: the fixture actually has a quote marker"
    );
    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, RemoteImages::Blocked);
    wait_for(&finished, Duration::from_secs(5));

    // ── network isolation: nothing ever reaches a real socket ─────────────
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener should bind");
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            if let Ok((_stream, _addr)) = listener.accept() {
                let _ = tx.send(());
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    let beacon = MessageBody {
        text: None,
        html: Some(format!(
            r#"<html><body><img src="http://127.0.0.1:{port}/beacon.gif"></body></html>"#
        )),
    };
    let finished = track_load_finished(&reader);
    reader.render(&beacon, RemoteImages::Blocked);
    wait_for(&finished, Duration::from_secs(5));
    // Give the background listener thread the full window it waits, in case
    // WebKit's own image fetch is merely slow rather than blocked.
    pump_for(Duration::from_millis(900));

    assert!(
        rx.try_recv().is_err(),
        "the reader must never have connected to its own blocked image's host"
    );

    window.destroy();
}

/// A flag that flips once `reader`'s `WebView` finishes its current load —
/// success or failure both count, since a `load-failed` is still "done" for
/// the purpose of "stop pumping and check the result".
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

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
}
