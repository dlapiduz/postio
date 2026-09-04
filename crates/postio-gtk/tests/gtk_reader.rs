//! The reading pane on a real display: `postio-lu6`, `postio-1bz` and
//! `postio-xxz` end to end, against the corpus fixtures they exist for.
//!
//! One test function, for the reason `gtk_shell.rs` gives — GTK is
//! single-threaded and initialised once. Skips without a display. The
//! network-isolation case is the one part of this file that *does* touch a
//! socket: a listener on `127.0.0.1` this process owns, there only to prove
//! nothing else ever connects to it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::address::EmailAddress;
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use webkit6::prelude::*;

#[test]
fn the_reader_renders_and_hardens_the_corpus() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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

    let allowlist_path = scratch_path("allowlist");
    let reader = Reader::with_allowlist(
        source,
        RemoteImageAllowList::default(),
        allowlist_path.clone(),
    );

    // What `postio_gtk::parts::PartsPanel::set_held_back` is wired from —
    // every render's blocked-reference counts, in order, split into ordinary
    // pictures and likely trackers (#174).
    let rendered_counts: Rc<RefCell<Vec<postio_gtk::reader::HeldBack>>> =
        Rc::new(RefCell::new(Vec::new()));
    let counts_for_reader = Rc::clone(&rendered_counts);
    reader.connect_rendered(move |held| counts_for_reader.borrow_mut().push(held));

    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("ada.norwood@example.com"));
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

    // ── the tracking-pixel fixture: blocked by default, banner says so ────
    let tracking = test_corpus::load("html-tracking-pixel-remote-images");
    let parsed = postio_model::mime::parse(tracking.bytes());

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("orders@shop.example.org"));
    wait_for(&finished, Duration::from_secs(5));

    assert!(
        reader.banner_visible(),
        "a message with remote images, none of them allowed, should show the banner"
    );
    assert!(
        reader
            .banner_always_allow_label()
            .contains("orders@shop.example.org"),
        "the banner should name the sender it would allow: {}",
        reader.banner_always_allow_label()
    );
    // The fixture is built for exactly this split (#174): a 320x240 product
    // shot, a 120x28 logo, and an open-rate beacon declaring `width="1"
    // height="1"` and `width:1px; height:1px` in its style. All three are
    // held back identically -- the split only changes what the parts panel
    // calls them.
    //
    // Note every one of them is served from a host with `tracker` in its
    // name, and two of them are pictures. That is the fixture making the
    // point the heuristic rests on: the host says nothing.
    assert_eq!(
        rendered_counts.borrow().last().copied(),
        Some(postio_gtk::reader::HeldBack {
            remote_images: 2,
            trackers: 1,
        }),
        "the fixture's three remote <img> tags should all be counted, and \
         the 1x1 beacon told apart from the two real pictures"
    );

    // ── a newsletter with nothing remote gets no banner ────────────────────
    let newsletter = test_corpus::load("html-newsletter");
    let parsed = postio_model::mime::parse(newsletter.bytes());

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    wait_for(&finished, Duration::from_secs(5));

    assert!(
        !reader.banner_visible(),
        "no remote reference was stripped, so there is nothing for the banner to report"
    );

    // ── #971: the unsubscribe banner names a list and reports activation ──
    assert!(
        !reader.unsubscribe_banner_visible(),
        "nothing has named a list yet"
    );
    reader.set_unsubscribe(Some("newsletter.example.com"));
    assert!(reader.unsubscribe_banner_visible());
    assert!(
        reader
            .unsubscribe_banner_label()
            .contains("newsletter.example.com"),
        "the banner should name the list a click would leave: {}",
        reader.unsubscribe_banner_label()
    );
    let activated: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let activated_for_handler = Rc::clone(&activated);
    reader.connect_unsubscribe_activated(move |list| {
        activated_for_handler.borrow_mut().push(list.to_owned());
    });
    reader.click_unsubscribe();
    pump();
    assert_eq!(
        activated.borrow().as_slice(),
        &["newsletter.example.com".to_owned()],
        "clicking unsubscribe should report the list currently named"
    );
    // A fresh render clears it, same convention as the decode notice: the
    // caveat belongs to one message and must not outlive it.
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    assert!(
        !reader.unsubscribe_banner_visible(),
        "render clears the previous message's banner until the caller sets a new one"
    );

    // ── #1009: a newsletter opens in reader view, and `C-o` leaves it ──────
    // Rendered above, so the state is whatever `render` decided for it.
    assert!(
        reader.is_reader_view(),
        "a campaign should open reduced; that is what reader view is for"
    );
    assert!(
        reader.reader_notice_visible(),
        "and it has to say so -- a surface that silently rewrites somebody's \
         mail is worse than one that does not rewrite it"
    );

    let finished = track_load_finished(&reader);
    reader.click_view_original();
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        !reader.is_reader_view(),
        "`View original` should have gone back to the sender's own markup"
    );
    assert!(
        !reader.reader_notice_visible(),
        "and the notice goes with it: an offer to show what is already on \
         screen is a control that does nothing"
    );

    // Per message, never sticky. Somebody who wanted to see one newsletter's
    // layout has said nothing at all about the next one.
    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        reader.is_reader_view(),
        "the next message decides for itself"
    );

    // ── and ordinary correspondence is never dragged into it ──────────────
    let ordinary = test_corpus::load("multipart-alternative");
    let ordinary = postio_model::mime::parse(ordinary.bytes());
    let finished = track_load_finished(&reader);
    reader.render(&ordinary.body, Some("ada.norwood@example.com"));
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        !reader.is_reader_view(),
        "a person's actual mail must look like the person wrote it"
    );
    assert!(!reader.reader_notice_visible());

    // ── #319: the header puts sender, subject and date on screen ──────────
    let header = reader.header();
    let ada = EmailAddress::new(Some("Ada Lovelace"), "ada@example.com");
    let bob = EmailAddress::new(Some("Bob"), "bob@example.com");
    let carol = EmailAddress::new(None::<&str>, "carol@example.org");
    let date = Utc.with_ymd_and_hms(2026, 8, 12, 14, 32, 0).unwrap();
    header.set_message(
        std::slice::from_ref(&ada),
        std::slice::from_ref(&bob),
        std::slice::from_ref(&carol),
        Some("Dinner Friday?"),
        date,
    );
    assert!(
        header.subject_label().contains("Dinner Friday?"),
        "the subject must be on screen: {}",
        header.subject_label()
    );
    assert!(
        header.sender_label().contains("Ada Lovelace")
            && header.sender_label().contains("ada@example.com"),
        "the sender's display name and address must both be on screen: {}",
        header.sender_label()
    );
    assert!(
        !header.date_label().is_empty(),
        "an absolute date and time must be on screen"
    );
    assert!(
        header.to_visible() && header.to_label().contains("bob@example.com"),
        "the one recipient must be reachable in one line: {}",
        header.to_label()
    );
    assert!(
        header.cc_toggle_visible(),
        "a Cc disclosure must be offered when the message has one"
    );
    assert!(
        !header.cc_revealed(),
        "Cc must not cost vertical space until asked for"
    );

    // #487: the conversation pane already draws sender/subject/date on the
    // entry above this header, and must not repeat them -- but it still
    // needs recipients on screen, so hiding "identity" cannot mean hiding
    // the whole header.
    header.set_identity_visible(false);
    assert!(
        !header.identity_visible(),
        "hiding identity has to be observable, not just asserted"
    );
    assert!(
        header.to_visible() && header.to_label().contains("bob@example.com"),
        "recipients must stay reachable with sender/subject/date hidden: {}",
        header.to_label()
    );
    assert!(header.cc_toggle_visible(), "so must the Cc disclosure");
    header.set_identity_visible(true);
    assert!(
        header.identity_visible(),
        "identity is restored for the single-message reading pane"
    );

    // A message with no subject and a sender with no display name still
    // renders a complete header, not a blank one.
    header.set_message(std::slice::from_ref(&carol), &[], &[], None, date);
    assert!(
        !header.subject_label().trim().is_empty(),
        "a missing subject must say so rather than showing nothing"
    );
    assert_eq!(
        header.sender_label(),
        "carol@example.org",
        "a sender with no display name shows the bare address, not a blank line"
    );
    assert!(
        !header.to_visible(),
        "no recipients means no To line taking up space for nothing"
    );
    assert!(
        !header.cc_toggle_visible(),
        "no Cc means no disclosure to offer"
    );

    // A header-only message -- headers synced, body not -- gets the same
    // header a message with a body does: `set_message` never reads the body.
    header.set_message(
        std::slice::from_ref(&ada),
        std::slice::from_ref(&bob),
        &[],
        Some("No body yet"),
        date,
    );
    reader.show_absent(postio_gtk::reader::Absent::Partial);
    assert!(
        header.subject_label().contains("No body yet"),
        "the header must survive showing an absent body"
    );

    // Closing the pane clears the header along with everything else.
    reader.clear();
    assert!(
        header.subject_label().is_empty(),
        "a cleared pane must not keep showing the last message's header"
    );

    // ── "always allow" persists and lifts the block on the next render ────
    let tracking_body = postio_model::mime::parse(tracking.bytes()).body;
    let finished = track_load_finished(&reader);
    reader.render(&tracking_body, Some("orders@shop.example.org"));
    wait_for(&finished, Duration::from_secs(5));
    assert!(reader.banner_visible());

    let finished = track_load_finished(&reader);
    reader.click_always_allow();
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        !reader.banner_visible(),
        "the banner should drop once its own sender is allow-listed"
    );
    assert_eq!(
        rendered_counts.borrow().last().copied(),
        Some(postio_gtk::reader::HeldBack::default()),
        "nothing is held back any more once the sender is allowed -- of \
         either kind -- and the parts panel's badge has to hear about that \
         re-render too"
    );

    let persisted = RemoteImageAllowList::load_from(&allowlist_path);
    assert!(
        persisted.is_allowed("orders@shop.example.org"),
        "the allow-list file should carry the exception, not just memory"
    );

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
    reader.render(&parsed.body, Some("quinn.abara@example.net"));
    wait_for(&finished, Duration::from_secs(5));
    // The body has no images at all, so folding a quote must not trip the
    // remote-image banner.
    assert!(!reader.banner_visible());

    // ── network isolation: nothing ever reaches a real socket ─────────────
    //
    // The claim is "nothing leaves this machine that the user did not ask
    // for", and half a proof of it is worthless: a listener nobody could ever
    // have reached would sit silent whether the reader were hardened or wide
    // open. So this runs twice — blocked, then consented — and the second run
    // is what makes the first one mean something.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener should bind");
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let accepting = std::thread::spawn(move || {
        while !stopping.load(Ordering::Relaxed) {
            if let Ok((stream, _addr)) = listener.accept() {
                // Answered, so WebKit's fetch completes rather than hanging:
                // a request that arrives is the thing being measured, and it
                // has to arrive the same way in both phases.
                let _ = (&stream).write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 0\r\n\r\n",
                );
                let _ = tx.send(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    let beacon = MessageBody {
        text: None,
        html: Some(format!(
            r#"<html><body><img src="http://127.0.0.1:{port}/beacon.gif"></body></html>"#
        )),
    };
    let sender = "tracker@example.org";
    let finished = track_load_finished(&reader);
    reader.render(&beacon, Some(sender));
    wait_for(&finished, Duration::from_secs(5));
    // Give the listener the full window, in case WebKit's own image fetch is
    // merely slow rather than blocked.
    pump_for(Duration::from_millis(900));

    assert!(
        rx.try_recv().is_err(),
        "the reader must never have connected to its own blocked image's host"
    );
    assert!(
        reader.banner_visible(),
        "and it has to say so, or the user cannot consent to what they cannot see"
    );

    // ── and the same beacon does arrive once the user asks for it ─────────
    //
    // Not a feature test: this is what proves the silence above was the
    // reader's doing. If this fetch never lands either, the listener was
    // unreachable and the assertion before it proved nothing at all.
    let finished = track_load_finished(&reader);
    reader.click_always_allow();
    wait_for(&finished, Duration::from_secs(5));
    let arrived = wait_for_connection(&rx, Duration::from_secs(3));

    stop.store(true, Ordering::Relaxed);
    let _ = accepting.join();

    assert!(
        arrived,
        "the beacon never arrived even after consent, so the blocked case \
         proved nothing — the listener was never reachable"
    );

    window.destroy();
}

/// Wait for the listener to report a connection, pumping GTK meanwhile.
fn wait_for_connection(rx: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if rx.try_recv().is_ok() {
            return true;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("postio-gtk-reader-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.ini"))
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
