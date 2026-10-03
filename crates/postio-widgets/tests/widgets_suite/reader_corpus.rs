//! The reader on a real display, end to end, against the corpus fixtures it
//! exists for: `cid:` images, the remote-image banner and its allow list, the
//! notices, quote folding, the reader view, and network isolation -- the one
//! part that does touch a socket: a listener on `127.0.0.1` this process
//! owns, there only to prove nothing else ever connects to it.
//!
//! Run as cases of the suite on its main thread: WebKit gives each thread
//! that asks a `RunLoop` of its own, and a dropped web view queues its
//! teardown onto the thread's, which nothing drains on a spawned one (#272).
//!
//! Skips without a display.

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
use gtk::glib;
use gtk::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use postio_ui::reader::document;
use postio_widgets::reader::{BlobSource, Reader, RemoteImageAllowList};

pub fn the_reader_renders_and_hardens_the_corpus() {
    // The whole point of the harness at the foot of this file. A libtest
    // `#[test]` would be running on a thread of its own here, and this test
    // deadlocks there (#272).
    assert_eq!(
        std::thread::current().name(),
        Some("main"),
        "this case must run on the main thread -- see `main` below"
    );

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

    // What the app hands every reader: a fetcher for the remote images a
    // document names once the user allowed them. This one only ever dials
    // this machine's loopback, so the file touches no real network.
    reader.set_remote_fetch(loopback_fetch);

    // What `the parts panel::set_held_back` is wired from —
    // every render's blocked-reference counts, in order, split into ordinary
    // pictures and likely trackers (#174).
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
    //
    // Four, not two, since #1326. The fixture's `<style>` block carries two
    // more remote references -- a hero background and a `css-beacon` -- and
    // it always did: they were simply invisible while `<style>` was deleted
    // unread, which is presumably why whoever built the fixture put them
    // there. Now that Postio parses the stylesheet it can see them, and a
    // background that did not load is exactly as held back, to the reader, as
    // an `<img>` that did not.
    //
    // The CSS beacon is counted with the images rather than as a tracker on
    // purpose. `is_likely_tracker` reads a declared 1x1 size off an `<img>`,
    // and a CSS background declares no size at all -- so Postio can say it
    // was blocked but cannot honestly say it was a beacon. Guessing from the
    // host would be the one thing the comment above says the fixture exists
    // to refute.
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
    // On screen at once: the newsletter opened as its sender built it (spec
    // 006 FR-031), so no reader-view notice outranks the list in the
    // reader's one notice slot.
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
    // A fresh render clears it, same convention as the decode notice: the
    // caveat belongs to one message and must not outlive it.
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    assert!(
        !reader.unsubscribe_banner_visible(),
        "render clears the previous message's banner until the caller sets a new one"
    );
    // And the caller does, for this message, as the application would.
    reader.set_unsubscribe(Some("newsletter.example.com"));

    // ── spec 006 FR-031: a newsletter opens as sent; reader view is asked for ─
    // Rendered above, so the state is whatever `render` decided for it.
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

    // ── #1029: and it lands on the sender's own paper, not Postio's ───────
    // The half of the canvas (turn 7, screen 20) that `View original` did
    // not do yet: *where* the original is drawn. A sender who laid out for a
    // white page gets their white-background logo, their mid-grey body text
    // and their links on a dark ground otherwise, which is the failure the
    // feature exists to avoid rather than one it may cause.
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
    // The chrome is the other half, and it is what makes the sheet a sheet:
    // `body` keeps painting the theme's ground, so nothing here may move the
    // light values onto the root.
    assert!(
        !sheet.contains(":root"),
        "the light palette belongs inside the sender's box, not on the root"
    );

    // And it paints. A selector that lost on specificity, or a custom
    // property assumed to inherit where it does not, would leave the text
    // above intact and paint nothing at all -- so the renderer is asked, in
    // both directions, in the dark theme where the two grounds differ.
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

    // Per message, never sticky. Somebody who wanted to see one newsletter's
    // layout has said nothing at all about the next one.
    let finished = track_load_finished(&reader);
    reader.render(&parsed.body, Some("weekly@news.example.org"));
    wait_for(&finished, Duration::from_secs(5));
    assert!(
        !reader.is_reader_view(),
        "the next message opens as its sender built it again"
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
    // #1029: correspondence is `Rendering::Original` too, and is exactly the
    // case that must not change. A reply on a white page inside a dark
    // window would be worse than what the theme already does.
    assert!(
        !reader
            .test_document()
            .contains(document::SENDERS_SHEET_CLASS),
        "an ordinary reply must keep following the theme"
    );

    // ── #1030: transactional mail gets its facts lifted above the copy ────
    // The end of the chain: `reader_view::lift` finds the rows, `body_html`
    // draws them, and this is the document the web view was actually handed.
    // Asserting on the extractor alone could not fail if nothing rendered it.
    let shipping = test_corpus::load("transactional-shipping-notice");
    let shipping = postio_model::mime::parse(shipping.bytes());
    let finished = track_load_finished(&reader);
    reader.render(&shipping.body, Some("orders@shop.example.test"));
    wait_for(&finished, Duration::from_secs(5));
    // Opens as sent, like everything (spec 006 FR-031); the facts are
    // reader view's, one command away.
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
    reader.show_absent(postio_widgets::reader::Absent::Partial);
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
                // Answered, so the fetch completes rather than hanging:
                // a request that arrives is the thing being measured, and it
                // has to arrive the same way in both phases.
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
    // Give the listener the full window, in case a fetch is merely slow
    // rather than never asked for.
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

    // ── the same proof for a beacon carried in a *style* ──────────────────
    //
    // #1325 admitted the sender's inline styling, and with it a second route
    // to the network that no `<img>` appears in: `background-image: url(…)`.
    // Spec FR-022 is explicit that the rule covers every way a message can
    // name a resource, "including from within its styling", so the isolation
    // proof has to cover it too.
    //
    // Two mechanisms should refuse this — `sanitize::contain_declarations`
    // drops a remote `url()` while images are blocked, and the renderer
    // resolves no URL the app did not hand it, so a declaration the
    // sanitizer missed still fetches nothing. Defence in depth is only
    // defence if something checks both layers are there.
    //
    // Run against a listener of its own, so a stray connection from the phase
    // above cannot be read as this one passing.
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
    // A sender of its own: the one above has just been granted consent, and
    // reusing it would test the allowed path while claiming to test the
    // blocked one.
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

    // And it arrives on consent, for the reason the image phase runs twice:
    // otherwise an unreachable listener would pass the assertion above.
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

/// The instrument, against the real reader (#1328).
///
/// `postio_ui::test_support` is only worth having if its numbers move when
/// the reader moves, and stay put when it does not. Asserted here rather than
/// in `postio-ui`'s own tests because that crate cannot build a `Reader`, and
/// an instrument proven only against itself is the shape of #327: written,
/// tested, wired to nothing.
pub fn the_counters_see_what_the_reader_actually_does() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

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

    // GTK finalizes on the main loop, not at the closing brace.
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

/// Building a reader loads nothing (#1603).
///
/// The constructor used to end in `clear()`, which loads an empty document --
/// and a view's first load is what starts its web process. The window's
/// reader is built inside `reading::install`, before the first frame, so that
/// load put a process start into the startup burst for a pane nobody had
/// opened a message in. The pane is painted its ground colour without a
/// document (`the_pane_is_painted_before_it_has_a_document`); a load waits
/// until there is something to draw, or until `warm` is asked for.
pub fn a_new_reader_loads_nothing_until_it_is_asked_to() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
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

/// Fetch `urls` the way the app's fetcher would, but only from `127.0.0.1`:
/// anything else is dropped unasked, so no case here reaches a real host.
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

/// A flag the reader's view raises when its next snapshot reaches the
/// screen.
fn track_load_finished(reader: &Reader) -> Rc<RefCell<bool>> {
    let done = Rc::new(RefCell::new(false));
    let flag = Rc::clone(&done);
    reader.view().connect_rendered(move |_| {
        *flag.borrow_mut() = true;
    });
    done
}

/// The ground the renderer paints behind `document`'s first text, in the
/// dark theme, as `#rrggbb`.
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
