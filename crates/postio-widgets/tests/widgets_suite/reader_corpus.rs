//! Run as cases on the suite's main thread: a dropped web view queues teardown onto its thread's
//! WebKit `RunLoop`, which nothing drains on a spawned thread (#272).

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
use gtk::glib;
use gtk::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use postio_ui::reader::document;
use postio_widgets::reader::{BlobSource, Reader, RemoteImageAllowList};

pub fn the_reader_renders_and_hardens_the_corpus() {
    assert_eq!(
        std::thread::current().name(),
        Some("main"),
        "this case must run on the main thread -- see `main` below"
    );

    let window = gtk::Window::new();
    window.set_default_size(600, 500);

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

    // Dials only this machine's loopback, so the file touches no real network.
    reader.set_remote_fetch(loopback_fetch);

    // What `the parts panel::set_held_back` is wired from: every render's blocked-reference counts,
    // split into pictures and likely trackers (#174).
    let rendered_counts: Rc<RefCell<Vec<postio_widgets::reader::HeldBack>>> =
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
    // Held back identically: a 320x240 shot, a 120x28 logo and a 1x1 beacon, all from a `tracker`
    // host, so the host says nothing.
    // Four since #1326: the `<style>` block carries two more remote references (a hero background
    // and a `css-beacon`), visible now the stylesheet is parsed.
    // The CSS beacon counts as an image: `is_likely_tracker` reads a declared 1x1 size off an
    // `<img>`, and a CSS background declares none.
    assert_eq!(
        rendered_counts.borrow().last().copied(),
        Some(postio_widgets::reader::HeldBack {
            remote_images: 4,
            trackers: 1,
        }),
        "the fixture's three remote <img> tags and its two CSS references \
         should all be counted, and the 1x1 beacon told apart from the two \
         real pictures"
    );

    let newsletter = test_corpus::load("html-newsletter");
    let parsed = postio_model::mime::parse(newsletter.bytes());

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    wait_for(&finished, Duration::from_secs(5));

    assert!(
        !reader.banner_visible(),
        "no remote reference was stripped, so there is nothing for the banner to report"
    );

    assert!(
        !reader.unsubscribe_banner_visible(),
        "nothing has named a list yet"
    );
    reader.set_unsubscribe(Some("newsletter.example.com"));
    // On screen at once: the newsletter opens as sent, so no reader-view notice outranks the list
    // in the one notice slot.
    assert!(
        reader.unsubscribe_banner_visible() && !reader.reader_notice_visible(),
        "nothing was rewritten, so the list is the notice that applies"
    );
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
    // A fresh render clears it: the caveat belongs to one message.
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    assert!(
        !reader.unsubscribe_banner_visible(),
        "render clears the previous message's banner until the caller sets a new one"
    );
    reader.set_unsubscribe(Some("newsletter.example.com"));

    assert!(
        !reader.is_reader_view(),
        "every message opens as its sender built it, bulk mail included"
    );
    let finished = track_load_finished(&reader);
    reader.toggle_reader_view();
    wait_for(&finished, Duration::from_secs(5));
    assert!(reader.is_reader_view(), "reader view is one command away");
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
    assert!(
        reader.unsubscribe_banner_visible(),
        "with reader view gone, the list is the notice that applies"
    );

    // It lands on the sender's own paper: a sender who laid out for white would otherwise get
    // white-background logos and grey text on a dark ground (#1029).
    let document = reader.test_document();
    assert!(
        document.contains(document::SENDERS_SHEET_CLASS),
        "leaving reader view should draw the original on the sender's sheet"
    );
    let sheet = document
        .split_once(&format!(
            ".{} .postio-body {{",
            document::SENDERS_SHEET_CLASS
        ))
        .expect("the sheet rule reached the document")
        .1
        .split_once('}')
        .expect("the sheet rule closes")
        .0;
    assert!(
        sheet.contains(&format!("--r-ground: {}", document::reader_ground(false))),
        "the sheet is the palette's light ground: {sheet}"
    );
    // `body` keeps painting the theme's ground, so the light values must not move onto the root.
    assert!(
        !sheet.contains(":root"),
        "the light palette belongs inside the sender's box, not on the root"
    );

    // And it paints: a selector that lost on specificity, or a custom property that does not
    // inherit, would leave the text intact and paint nothing. Ask the renderer in the dark theme,
    // where the grounds differ.
    let light = document::reader_ground(false);
    let sheet_document = document::document_for(
        "<p>hi</p>",
        "",
        postio_body::RemoteImages::Blocked,
        document::Sheet::Senders,
    );
    let sheet_ground = ground_behind_first_text(&sheet_document);
    assert_eq!(
        sheet_ground, light,
        "the sender's box should actually be painted the light ground"
    );
    let theme_document = document::document_for(
        "<p>hi</p>",
        "",
        postio_body::RemoteImages::Blocked,
        document::Sheet::Theme,
    );
    assert_ne!(
        ground_behind_first_text(&theme_document),
        light,
        "an ordinary document must leave the box unpainted, so it shows the \
         chrome's ground through -- that is what makes the sheet a change"
    );

    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        !reader.is_reader_view(),
        "the next message opens as its sender built it again"
    );

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
    // #1029: correspondence must not change; a reply on a white page in a dark window would be
    // worse than the theme.
    assert!(
        !reader
            .test_document()
            .contains(document::SENDERS_SHEET_CLASS),
        "an ordinary reply must keep following the theme"
    );

    // The end of the chain: asserting on the extractor alone could not fail if nothing rendered it
    // (#1030).
    let shipping = test_corpus::load("transactional-shipping-notice");
    let shipping = postio_model::mime::parse(shipping.bytes());
    let finished = track_load_finished(&reader);
    reader.render(&shipping.body, Some("orders@shop.example.test"));
    wait_for(&finished, Duration::from_secs(5));
    let finished = track_load_finished(&reader);
    reader.toggle_reader_view();
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        reader.is_reader_view(),
        "a shipping notice reduces on request"
    );
    let document = reader.test_document();
    let block = document
        .find(document::FACTS_CLASS)
        .expect("the facts block reached the reading pane");
    let copy = document
        .find("Follow the parcel")
        .expect("and so did the body copy");
    assert!(
        block < copy,
        "the canvas draws the facts above the body copy"
    );
    assert!(
        document.contains("EXTEST0042199317") && document.contains("1 Example Way"),
        "the tracking number and destination are on screen"
    );
    assert_eq!(
        document.matches("EXTEST0042199317").count(),
        1,
        "and drawn once, not once in the block and again in the paragraph"
    );

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
        header.to_visible(),
        "the one recipient must be reachable in one line"
    );
    assert_eq!(
        header.to_label(),
        "Bob <bob@example.com>",
        "the label column already draws \"To\" (#1437), so the value beside it \
         must be the bare recipients -- `contains` could not see a doubled prefix"
    );
    assert!(
        header.cc_toggle_visible(),
        "a Cc disclosure must be offered when the message has one"
    );
    assert!(
        !header.cc_revealed(),
        "Cc must not cost vertical space until asked for"
    );

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

    header.set_message(
        std::slice::from_ref(&ada),
        std::slice::from_ref(&bob),
        &[],
        Some("No body yet"),
        date,
    );
    reader.show_absent(postio_widgets::reader::Absent::Partial);
    assert!(
        header.subject_label().contains("No body yet"),
        "the header must survive showing an absent body"
    );

    reader.clear();
    assert!(
        header.subject_label().is_empty(),
        "a cleared pane must not keep showing the last message's header"
    );

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
        Some(postio_widgets::reader::HeldBack::default()),
        "nothing is held back any more once the sender is allowed -- of \
         either kind -- and the parts panel's badge has to hear about that \
         re-render too"
    );

    let persisted = RemoteImageAllowList::load_from(&allowlist_path);
    assert!(
        persisted.is_allowed("orders@shop.example.org"),
        "the allow-list file should carry the exception, not just memory"
    );

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
    assert!(!reader.banner_visible());

    // Run twice, blocked then consented: a listener nobody could reach would be silent either way,
    // so the second run is what makes the first mean something.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener should bind");
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let accepting = std::thread::spawn(move || {
        while !stopping.load(Ordering::Relaxed) {
            if let Ok((stream, _addr)) = listener.accept() {
                // Answered so the fetch completes; a request must arrive the same way in both
                // phases.
                let _ = (&stream).write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 0\r\n\r\n",
                );
                let _ = tx.send(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
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
    pump_for(Duration::from_millis(900));

    assert!(
        rx.try_recv().is_err(),
        "the reader must never have connected to its own blocked image's host"
    );
    assert!(
        reader.banner_visible(),
        "and it has to say so, or the user cannot consent to what they cannot see"
    );

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

    // The same proof for a beacon in a style (#1325): `background-image: url(...)` is a second
    // route to the network. Two layers refuse it, `sanitize::contain_declarations` and the
    // renderer, so check both.
    // A listener of its own, so a stray connection from the phase above is not read as this one
    // passing.
    let styled_listener = TcpListener::bind("127.0.0.1:0").expect("a local listener should bind");
    styled_listener.set_nonblocking(true).unwrap();
    let styled_port = styled_listener.local_addr().unwrap().port();
    let (styled_tx, styled_rx) = mpsc::channel();
    let styled_stop = Arc::new(AtomicBool::new(false));
    let styled_stopping = Arc::clone(&styled_stop);
    let styled_accepting = std::thread::spawn(move || {
        while !styled_stopping.load(Ordering::Relaxed) {
            if let Ok((stream, _addr)) = styled_listener.accept() {
                let _ = (&stream).write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 0\r\n\r\n",
                );
                let _ = styled_tx.send(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    let styled_beacon = MessageBody {
        text: None,
        html: Some(format!(
            r#"<html><body><div style="background-image:url(http://127.0.0.1:{styled_port}/styled.gif);width:20px;height:20px">.</div></body></html>"#
        )),
    };
    // A sender of its own: the one above has consent, and reusing it would test the allowed path.
    let styled_sender = "styled-tracker@example.org";
    let finished = track_load_finished(&reader);
    reader.render(&styled_beacon, Some(styled_sender));
    wait_for(&finished, Duration::from_secs(5));
    pump_for(Duration::from_millis(900));

    assert!(
        styled_rx.try_recv().is_err(),
        "a remote URL named by the sender's *style* reached the network. The \
         message never contained an <img>, which is exactly why FR-022 is \
         written about every way a resource can be named"
    );
    assert!(
        reader.banner_visible(),
        "a style-borne remote reference must be held back visibly, like any \
         other — a user cannot consent to what they cannot see"
    );

    let finished = track_load_finished(&reader);
    reader.click_always_allow();
    wait_for(&finished, Duration::from_secs(5));
    let styled_arrived = wait_for_connection(&styled_rx, Duration::from_secs(3));

    styled_stop.store(true, Ordering::Relaxed);
    let _ = styled_accepting.join();

    assert!(
        styled_arrived,
        "the styled beacon never arrived even after consent, so the blocked \
         case proved nothing — either the listener was unreachable, or the \
         sanitizer is dropping the declaration even when the user has allowed \
         the sender, which would make consent a lie in the other direction"
    );

    window.destroy();
}

/// `postio_ui::test_support` is only worth having if its numbers move when the reader moves;
/// `postio-ui` cannot build a `Reader`.
pub fn the_counters_see_what_the_reader_actually_does() {
    let surfaces_before = postio_ui::test_support::surfaces_created();
    let held_before = postio_ui::test_support::surfaces_held();
    let renders_before = postio_ui::test_support::renders_issued();

    {
        let window = gtk::Window::new();
        window.set_default_size(600, 500);
        let reader = Reader::with_allowlist(
            Rc::new(NoBlobs),
            RemoteImageAllowList::default(),
            scratch_path("counted-reader"),
        );
        window.set_child(Some(&reader.widget()));
        window.present();
        pump();

        assert_eq!(
            postio_ui::test_support::surfaces_created() - surfaces_before,
            1,
            "one Reader is one rendering surface -- once one web process, which \
             is the cost ADR 0032 measured at thirty for a thirty-message thread"
        );

        for fixture in ["multipart-alternative", "html-newsletter"] {
            let parsed = postio_model::mime::parse(test_corpus::load(fixture).bytes());
            let finished = track_load_finished(&reader);
            reader.render(&parsed.body, None);
            wait_for(&finished, Duration::from_secs(5));
            pump();
        }

        assert!(
            postio_ui::test_support::renders_issued() - renders_before >= 2,
            "two messages rendered and the counter did not see them, so every \
             cost-of-moving assertion built on it would pass without measuring"
        );
        assert_eq!(
            postio_ui::test_support::surfaces_created() - surfaces_before,
            1,
            "rendering a second message into the same reader must not build a \
             second surface -- that is the whole of FR-057"
        );

        window.set_child(None::<&gtk::Widget>);
        window.destroy();
    }

    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }

    assert_eq!(
        postio_ui::test_support::surfaces_held() - held_before,
        0,
        "the reader went out of scope and its surface was never reported \
         released, so `surfaces_held` cannot answer the question it exists for: \
         whether a conversation lets go of what it opened"
    );
}

/// The constructor used to end in `clear()`, whose empty load starts the web process inside the
/// startup burst for a pane nobody had opened (#1603).
pub fn a_new_reader_loads_nothing_until_it_is_asked_to() {
    let before = postio_ui::test_support::renders_issued();
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("loads-nothing"),
    );
    assert_eq!(reader.loads(), 0, "building a reader loaded a document");
    assert_eq!(
        postio_ui::test_support::renders_issued() - before,
        0,
        "building a reader handed a document to the engine"
    );
}

struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

/// Anything but `127.0.0.1` is dropped unasked, so no case reaches a real host.
fn loopback_fetch(urls: Vec<String>, done: postio_widgets::reader::view::RemoteArrived) {
    let (tx, rx) = mpsc::channel::<Vec<(String, Vec<u8>)>>();
    std::thread::spawn(move || {
        let mut arrived = Vec::new();
        for url in urls {
            let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
                continue;
            };
            let (port, path) = rest.split_once('/').unwrap_or((rest, ""));
            let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{port}")) else {
                continue;
            };
            let _ = write!(
                stream,
                "GET /{path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            );
            let mut response = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stream, &mut response);
            arrived.push((url, response));
        }
        let _ = tx.send(arrived);
    });
    let done = RefCell::new(Some(done));
    glib::timeout_add_local(Duration::from_millis(10), move || match rx.try_recv() {
        Ok(arrived) => {
            if let Some(done) = done.borrow_mut().take() {
                done(arrived);
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn wait_for_connection(rx: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + postio_test_support::scaled(timeout);
    while Instant::now() < deadline {
        if rx.try_recv().is_ok() {
            return true;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("postio-widgets-reader-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.ini"))
}

fn track_load_finished(reader: &Reader) -> Rc<RefCell<bool>> {
    let done = Rc::new(RefCell::new(false));
    let flag = Rc::clone(&done);
    reader.view().connect_rendered(move |_| {
        *flag.borrow_mut() = true;
    });
    done
}

fn ground_behind_first_text(document: &str) -> String {
    let resources = postio_render::Resources::new();
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let request = postio_render::RenderRequest {
        generation: 1,
        document: document.to_owned(),
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        resources: Arc::new(resources),
        viewport: postio_render::Viewport {
            width: 600.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: postio_render::Theme {
            dark: true,
            high_contrast: false,
        },
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    let drawn = postio_render::render(&request, postio_widgets::body_view::font_set());
    let ground = drawn
        .text
        .clusters
        .first()
        .expect("the document draws some text")
        .painted_ground;
    let channel = |v: f64| (v * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(ground.r),
        channel(ground.g),
        channel(ground.b)
    )
}

fn wait_for(flag: &Rc<RefCell<bool>>, timeout: Duration) {
    let deadline = Instant::now() + postio_test_support::scaled(timeout);
    while !*flag.borrow() && Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(*flag.borrow(), "the reader never drew the document");
}

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}

/// Turn the loop for `duration`, spending all of it.
///
/// POSTIO-FIXED-DEADLINE: nothing is waited *for* here -- callers pass a
/// window they intend to spend, to give a thing that must not happen every
/// chance to happen. Scaling it would multiply the case's cost and add no
/// confidence.
fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
