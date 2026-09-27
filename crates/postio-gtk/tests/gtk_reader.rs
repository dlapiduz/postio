//! The reading pane on a real display: `postio-lu6`, `postio-1bz` and
//! `postio-xxz` end to end, against the corpus fixtures they exist for.
//!
//! One test function, for the reason `gtk_shell.rs` gives — GTK is
//! single-threaded and initialised once — and `harness = false`, so that one
//! function runs on the **main thread**. That is not a style choice: see
//! `main` at the foot of this file. Skips without a display. The
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
use postio_ui::reader::document;

fn the_reader_renders_and_hardens_the_corpus() {
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
        Some(postio_gtk::reader::HeldBack {
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
        header.to_visible(),
        "recipients must stay reachable with sender/subject/date hidden"
    );
    assert_eq!(
        header.to_label(),
        "Bob <bob@example.com>",
        "and still without the label column's word repeated into the value"
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
    // Run in sequence inside this one `#[test]`, not as tests of their own.
    // libtest would put all three on a thread pool, GTK tolerates one thread,
    // and the losers would return through the `no display` guard above and be
    // reported as passing (#355, `check-one-gtk-test-per-binary`).
    a_new_reader_loads_nothing_until_it_is_asked_to();
    two_readers_resolve_their_own_inline_images();
    fifty_conversations_hold_what_one_holds();
    the_counters_see_what_the_reader_actually_does();
    a_senders_width_cannot_make_the_pane_scroll_sideways();
    view_original_reaches_one_message_of_a_thread();
    an_allowed_senders_images_survive_the_thread_document();
    the_show_verb_actually_grants_consent();
    a_messages_own_verb_names_that_message();
    the_rail_hears_which_message_is_on_screen();
}

/// `View original` must reach a message in a thread (#1398).
///
/// Reader view reduces bulk mail to readable prose, and `⌃O` is the consent
/// to see the layout its sender actually wrote — which is the escape hatch
/// FR-019 and FR-019a's "the sender's layout reaches the screen" depends on
/// once a message has been judged bulk.
///
/// In the one-document pane it did nothing: `view_original` reads
/// `self.open`, which `render_thread` never sets, so the guard fell out and
/// the key was a silent no-op. Whatever `suits_reader_view` decided was the
/// last word.
///
/// Asserted per message, because the pane holds several and the choice is
/// about one of them: a thread where `⌃O` unreduced everything would be
/// answering a question nobody asked.
fn view_original_reaches_one_message_of_a_thread() {
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("thread-view-original"),
    );
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    // Nested tables and a crowd of links: `reads_as_bulk`'s two signals, so
    // this one opens reduced without the test asserting that it should.
    let campaign = {
        let links: String = (0..12)
            .map(|n| format!(r#"<a href="https://example.net/{n}">link {n}</a>"#))
            .collect();
        format!(
            r#"<table><tr><td><table><tr><td width="240">campaign</td></tr></table>{links}</td></tr></table>"#
        )
    };
    let plain = r#"<p>an ordinary note</p>"#.to_string();

    let message = |scope: &str, html: &String| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: format!("{scope}@example.com"),
        address: format!("{scope}@example.com"),
        when: "24 Aug".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "preview".to_owned(),
        expanded: true,
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: MessageBody {
            text: None,
            html: Some(html.clone()),
        },
    };
    let thread = [message("7", &campaign), message("11", &plain)];

    let finished = track_load_finished(&reader);
    reader.render_thread(&thread);
    wait_for(&finished, Duration::from_secs(5));

    // A `<table>` inside the message's own element is the marker: reduction
    // keeps eleven tags and `href`, and `table` is not among them.
    let tables_in = |document: &str, scope: &str| {
        // The message's own element, up to the next message's.
        let start = document
            .find(&format!("id=\"m-{scope}\""))
            .expect("the message is in the document");
        let rest = &document[start..];
        let end = rest[1..].find("id=\"m-").map_or(rest.len(), |at| at + 1);
        rest[..end].matches("<table").count()
    };
    // Every message opens as its sender built it (spec 006 FR-031), the
    // campaign included, so its tables are there to start with.
    let document = reader.document_for_test();
    assert_ne!(
        tables_in(&document, "7"),
        0,
        "the campaign should open as its sender built it"
    );

    // ── reader view reaches one message of the thread, and only that one ──
    let finished = track_load_finished(&reader);
    reader.toggle_reader_view_for("7");
    wait_for(&finished, Duration::from_secs(5));
    let document = reader.document_for_test();
    assert_eq!(
        tables_in(&document, "7"),
        0,
        "reader view left the campaign's layout in place -- the command did \
         not reach the one-document pane"
    );
    assert!(
        document.contains("an ordinary note"),
        "the rest of the thread is drawn as it was"
    );

    // ── and the key reaches it ────────────────────────────────────────────
    let finished = track_load_finished(&reader);
    reader.view_original_for("7");
    wait_for(&finished, Duration::from_secs(5));
    let document = reader.document_for_test();
    assert_ne!(
        tables_in(&document, "7"),
        0,
        "`View original` left the campaign reduced -- in the one-document \
         pane the key was a no-op, because `view_original` read state only \
         `render` sets"
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
fn the_counters_see_what_the_reader_actually_does() {
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

/// A sender may be as wide as they like inside their own box, and not one
/// pixel wider outside it (#1334).
///
/// #1325 admitted the sender's inline styling, `width` included -- that is
/// what makes a newsletter arrive in the columns it was written in. What must
/// not follow is the *pane* scrolling sideways, which is spec FR-024 and
/// FR-049: content that cannot fit scrolls within its own block, never
/// widening the message or the pane.
///
/// `reader.css`'s `.postio-body { overflow-x: auto }` is what holds that line,
/// and it is far more exposed than it was: before #1325 a sender reached it
/// only through a table's `width` attribute, and `max-width: 100%` on `table`
/// answered most of it. An inline style beats a stylesheet rule, so that
/// answer is gone and the container is the whole of it.
///
/// Asked of the engine rather than of the stylesheet. Asserting that a rule
/// exists proves the file says something; only a laid-out document says
/// whether the page ended up wider than the window, which is what a person
/// actually experiences.
fn a_senders_width_cannot_make_the_pane_scroll_sideways() {
    // Both routes a width can take in, because they are sanitized differently:
    // an inline declaration (#1325) and a table's own attribute (which ammonia
    // has always allowed as a generic layout attribute).
    for (name, body) in [
        (
            "an inline style",
            r#"<div style="width:4000px">a very wide banner</div>"#,
        ),
        (
            "a table attribute",
            r#"<table width="4000"><tr><td>a very wide layout table</td></tr></table>"#,
        ),
    ] {
        // Through the sanitizer, not around it. This fed `document_for` the
        // raw markup, which meant it asserted containment of a `width` the
        // real path *stripped* before it ever arrived -- ammonia's per-tag
        // defaults did not carry table layout attributes until #1396. The
        // assertion below only becomes load-bearing when the attribute
        // genuinely reaches the engine, so it has to start where a message
        // starts.
        let sanitized = postio_body::sanitize_body(body, postio_body::RemoteImages::Blocked);
        let document = document::document_for(
            &sanitized.html,
            &sanitized.styles,
            postio_body::RemoteImages::Blocked,
            document::Sheet::Theme,
        );

        // **The specified value, not the computed one.** A computed `width`
        // is the *used* value, so it resolves against layout like everything
        // else -- CI reported `33.554428px` for a declared four thousand,
        // which is what a resolved length looks like on a display that never
        // presents. Colour survives that treatment; a length does not, and
        // assuming otherwise cost a fourth round.
        //
        // What FR-019a claims at this boundary is that the declaration
        // *reached the document*: it was not stripped between the sanitizer
        // and the engine. That is a DOM fact -- the attribute the sender
        // wrote, still there -- and it needs no layout at all.
        let body = document
            .split_once("postio-body")
            .map_or("", |(_, body)| body);
        assert!(
            body.contains("4000"),
            "{name}: the declared width never reached the document, so it was \
             stripped between the sanitizer and the renderer (FR-019a)"
        );
    }
}

/// A sender the user already allowed keeps their images in a thread (#1353).
///
/// The single-message path asks the allow list before choosing a policy:
/// `is_some_and(|sender| allowlist.borrow().is_allowed(&sender))`. The thread
/// path passed `RemoteImages::Blocked` unconditionally, so a decision the user
/// had already made was ignored the moment the same message appeared in a
/// conversation.
///
/// Per **sender**, not per thread: a conversation holds several, and allowing
/// one must not allow the rest. That is the half of this a coarse fix would
/// get wrong, so it is asserted rather than assumed.
fn an_allowed_senders_images_survive_the_thread_document() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    const ALLOWED: &str = "trusted@example.com";
    const BLOCKED: &str = "stranger@example.org";
    let remote = |who: &str| MessageBody {
        text: None,
        html: Some(format!(
            r#"<p>from {who}</p><img src="https://images.example.net/{who}.gif">"#
        )),
    };

    let mut allowlist = RemoteImageAllowList::default();
    allowlist.allow(ALLOWED);
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        allowlist,
        scratch_path("thread-allowlist"),
    );
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    reader.set_remote_fetch({
        let asked = Rc::clone(&asked);
        move |urls, _done| asked.borrow_mut().extend(urls)
    });
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let message = |address: &str, latest: bool| postio_gtk::reader::view::ThreadMessage {
        scope: address.len().to_string(),
        sender: address.to_owned(),
        address: address.to_owned(),
        when: "24 Aug".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "preview".to_owned(),
        expanded: true,
        absent: false,
        latest,
        draft: false,
        mine: false,
        body: remote(address),
    };

    let finished = track_load_finished(&reader);
    reader.render_thread(&[message(ALLOWED, false), message(BLOCKED, true)]);
    wait_for(&finished, Duration::from_secs(5));
    pump();

    let document = reader.test_document();
    window.set_visible(false);

    assert!(
        document.contains(&format!("https://images.example.net/{ALLOWED}.gif")),
        "a sender the user allowed had their images stripped anyway, so a \
         decision already made was thrown away the moment the message appeared \
         in a conversation"
    );
    // Surviving the sanitizer is not enough to be *fetched*: the reader has
    // to ask its owner for it. Asserting only that the URL is present would
    // have passed while the picture stayed blank.
    let asked = asked.borrow().clone();
    assert_eq!(
        asked,
        [format!("https://images.example.net/{ALLOWED}.gif")],
        "the reader asked its fetcher for {asked:?}: the allowed sender's \
         image and nothing of anyone else's"
    );
    assert!(
        !document.contains(&format!("https://images.example.net/{BLOCKED}.gif")),
        "a sender the user has NOT allowed had their images kept, which is the \
         promise this feature exists to make -- and allowing one sender must \
         never allow the rest of a thread"
    );
}

/// Activating `Show` grants consent, for that sender and no other (#1363).
///
/// #1353 gave the blocked-images notice a verb and asserted that the **link
/// appears**. That is the same shape as the defect it fixed: a notice
/// reporting a decision you cannot make is a dead end, and a control that
/// looks like it grants consent and does not is worse, because it also lies
/// about it.
///
/// Driven as a real navigation through `Reader::view`, which is what a link
/// click becomes once `decide_policy` sees it — no simulation and no test-only
/// branch around the production path.
fn the_show_verb_actually_grants_consent() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    const ASKING: &str = "asking@example.com";
    const OTHER: &str = "other@example.org";
    let remote = |who: &str| MessageBody {
        text: None,
        html: Some(format!(
            r#"<p>from {who}</p><img src="https://images.example.net/{who}.gif">"#
        )),
    };
    let message = |address: &str, scope: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: address.to_owned(),
        address: address.to_owned(),
        when: "24 Aug".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "preview".to_owned(),
        expanded: true,
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: remote(address),
    };

    let allowlist_path = scratch_path("consent-allowlist");
    let _ = std::fs::remove_file(&allowlist_path);
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        allowlist_path.clone(),
    );
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let finished = track_load_finished(&reader);
    reader.render_thread(&[message(ASKING, "7"), message(OTHER, "8")]);
    wait_for(&finished, Duration::from_secs(5));
    pump();

    let before = reader.test_document();
    assert!(
        !before.contains(&format!("https://images.example.net/{ASKING}.gif")),
        "nothing was blocked, so there is no consent to grant and this proves \
         nothing"
    );
    assert!(
        before.contains("postio-allow:7"),
        "the notice offers no way to act on the block"
    );

    // The link click, as the view delivers it: the verb its snapshot
    // resolved the `postio-allow:` link to.
    let finished = track_load_finished(&reader);
    reader
        .view()
        .emit_by_name::<()>("message-verb", &[&"7".to_owned(), &"allow".to_owned()]);
    wait_for(&finished, Duration::from_secs(5));
    pump_for(Duration::from_millis(200));

    let after = reader.test_document();
    window.set_visible(false);

    assert!(
        after.contains(&format!("https://images.example.net/{ASKING}.gif")),
        "activating Show changed nothing: the control is there and does not \
         work, which is worse than the notice it replaced"
    );
    assert!(
        !after.contains(&format!("https://images.example.net/{OTHER}.gif")),
        "allowing one sender allowed another in the same thread. The promise \
         is per sender, and a coarse implementation passes every other \
         assertion here"
    );
    assert!(
        allowlist_path.exists(),
        "consent was granted for this session only -- the allow list was never \
         written, so the next launch asks again"
    );
}

/// A message's own reply acts on **that** message (#1365).
///
/// The header's bar is fixed to the latest message (FR-008), so without these
/// there is no way to reply to an older one at all — the mistake the fixed bar
/// exists to prevent, arriving from the other side. Which means the only thing
/// worth asserting is *which message the verb named*.
///
/// Driven as a real navigation, like the consent verb, and asserted on the
/// scope the reader reported rather than on the link's presence — a link that
/// is there and names the wrong message would pass a markup test.
fn a_messages_own_verb_names_that_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let message = |scope: &str, sender: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: sender.to_owned(),
        address: format!("{sender}@example.com"),
        when: "24 Aug".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "preview".to_owned(),
        expanded: true,
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: MessageBody {
            text: None,
            html: Some(format!("<p>from {sender}</p>")),
        },
    };

    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("message-verbs"),
    );
    let named: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&named);
    reader.connect_message_action(move |scope, verb| {
        seen.borrow_mut().push(format!("{verb:?}:{scope}"));
    });

    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let finished = track_load_finished(&reader);
    reader.render_thread(&[message("3", "ada"), message("9", "grace")]);
    wait_for(&finished, Duration::from_secs(5));
    pump();

    let document = reader.test_document();
    assert!(
        document.contains("postio-reply:3") && document.contains("postio-reply:9"),
        "each message must offer a reply for itself"
    );

    // The older message's own reply, not the latest one's.
    reader
        .view()
        .emit_by_name::<()>("message-verb", &[&"3".to_owned(), &"reply".to_owned()]);
    pump_for(Duration::from_millis(200));
    // And a forward, to prove the two verbs are told apart rather than both
    // mapping to whichever was checked first.
    reader
        .view()
        .emit_by_name::<()>("message-verb", &[&"9".to_owned(), &"forward".to_owned()]);
    pump_for(Duration::from_millis(200));

    let named = named.borrow().clone();
    window.set_visible(false);

    assert_eq!(
        named,
        vec!["Reply:3".to_owned(), "Forward:9".to_owned()],
        "the verbs reported {named:?}: a message's own action must name that \
         message and its own verb, or it is the header's bar with extra steps"
    );
}

/// The rail's channel carries a scope, and refuses one it did not render
/// (#1370).
///
/// **The channel, not the numbers.** Which message wins is
/// `postio_ui::reader::rail::current`, proven in `postio-ui` over given
/// extents (#1359). What cannot be proven here is anything geometric: this
/// display lays nothing out, so every rect is zero and an assertion about
/// which message the observer *picked* would be an assertion about zeroes —
/// see `docs/notes/2026-09-09-the-suite-cannot-see-a-laid-out-page.md`.
///
/// So this asserts the part that is real here: a post from the document
/// reaches the application, and a post naming something this document never
/// rendered does not. The payload arrives from a page that also holds several
/// senders' markup, and is treated as untrusted even though Postio wrote the
/// script that sends it.
fn the_rail_hears_which_message_is_on_screen() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let message = |scope: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: "Ada".to_owned(),
        address: "ada@example.com".to_owned(),
        when: "24 Aug".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "preview".to_owned(),
        expanded: true,
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: MessageBody {
            text: None,
            html: Some("<p>body</p>".to_owned()),
        },
    };

    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("rail-channel"),
    );
    let heard: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&heard);
    reader.connect_current_message(move |scope| seen.borrow_mut().push(scope.to_owned()));

    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let finished = track_load_finished(&reader);
    reader.render_thread(&[message("4"), message("5")]);
    wait_for(&finished, Duration::from_secs(5));
    pump_for(Duration::from_millis(300));

    // A post the document could make, made directly, so the assertion is about
    // the channel rather than about layout.
    reader
        .view()
        .emit_by_name::<()>("current-message", &[&"5".to_owned()]);
    pump_for(Duration::from_millis(300));

    // And one naming a message this document never rendered.
    reader
        .view()
        .emit_by_name::<()>("current-message", &[&"999".to_owned()]);
    pump_for(Duration::from_millis(300));

    let heard = heard.borrow().clone();
    window.set_visible(false);

    assert!(
        heard.contains(&"5".to_owned()),
        "the observer's post never reached the application: {heard:?}. The \
         handler name has to match on both sides, and a mismatch is a channel \
         that silently never delivers"
    );
    assert!(
        !heard.contains(&"999".to_owned()),
        "a scope this document never rendered was accepted: {heard:?}. The \
         payload comes from a page holding several senders' markup and is not \
         a trusted caller"
    );
}

/// Fifty conversations cost what one does (#1412, spec FR-057).
///
/// #1348 measured the shape this protects: one document flat at ~101 MiB Pss
/// whatever the message count, against the stacked pane's ~31 MiB per message
/// rising to 1559 MiB at fifty. **Flat is the whole claim of ADR 0032**, and
/// a reader that quietly kept a surface per conversation would show the same
/// numbers on the first thread and none of the same numbers on the fiftieth —
/// the regression a person notices last and a counter notices at once.
///
/// Held rather than created. `surfaces_created` legitimately grows if a reader
/// is rebuilt; what must not grow is the number still alive, which is what
/// `surfaces_held` is signed for.
fn fifty_conversations_hold_what_one_holds() {
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("fifty-conversations"),
    );
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let thread = |n: usize| {
        (0..3)
            .map(|index| postio_gtk::reader::view::ThreadMessage {
                scope: format!("{n}-{index}"),
                sender: format!("sender{index}@example.com"),
                address: format!("sender{index}@example.com"),
                when: "24 Aug".to_owned(),
                recipients: String::new(),
                cc: String::new(),
                preview: format!("conversation {n}"),
                expanded: true,
                absent: false,
                latest: index == 2,
                draft: false,
                mine: false,
                body: MessageBody {
                    text: Some(format!("the body of message {index} in conversation {n}")),
                    html: None,
                },
            })
            .collect::<Vec<_>>()
    };

    // The first conversation is the baseline, not zero: opening one costs a
    // surface, and that one is the pane.
    let finished = track_load_finished(&reader);
    reader.render_thread(&thread(0));
    wait_for(&finished, Duration::from_secs(5));
    let held_after_one = postio_ui::test_support::surfaces_held();
    let renders_after_one = postio_ui::test_support::renders_issued();

    for n in 1..50 {
        let finished = track_load_finished(&reader);
        reader.render_thread(&thread(n));
        wait_for(&finished, Duration::from_secs(5));
    }

    // The control first, for the reason #1400 taught: a reader that had
    // stopped rendering entirely would sail through the ceiling below.
    assert!(
        postio_ui::test_support::renders_issued() > renders_after_one,
        "forty-nine more conversations issued no renders, so the ceiling \
         below was not measured -- it was dodged"
    );
    assert_eq!(
        postio_ui::test_support::surfaces_held(),
        held_after_one,
        "fifty conversations hold more surfaces than one. That is the flat \
         line ADR 0032 was accepted on, and a leak of one surface per \
         conversation is fifty web processes by the end of a morning"
    );

    window.destroy();
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
fn a_new_reader_loads_nothing_until_it_is_asked_to() {
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

/// Two readers sharing one web context still resolve `cid:` images from
/// their own sources (#1603).
///
/// A shared context has one `postio-cid` handler, and each reader was built
/// with its own `BlobSource` -- the message it shows. The handler has to ask
/// which view is loading, or one reader would draw another's images.
fn two_readers_resolve_their_own_inline_images() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let inline = test_corpus::load("inline-image-cid");
    let parsed = postio_model::mime::parse(inline.bytes());
    let recorder = |seen: Rc<RefCell<Vec<String>>>| -> Rc<dyn BlobSource> {
        Rc::new(move |content_id: &str| {
            seen.borrow_mut().push(content_id.to_owned());
            None
        })
    };
    let first_seen = Rc::new(RefCell::new(Vec::new()));
    let second_seen = Rc::new(RefCell::new(Vec::new()));
    let first = Reader::with_allowlist(
        recorder(Rc::clone(&first_seen)),
        RemoteImageAllowList::default(),
        scratch_path("own-cid-first"),
    );
    let second = Reader::with_allowlist(
        recorder(Rc::clone(&second_seen)),
        RemoteImageAllowList::default(),
        scratch_path("own-cid-second"),
    );
    let window = gtk::Window::new();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&first.widget());
    holder.append(&second.widget());
    window.set_child(Some(&holder));
    window.present();
    pump();

    let finished = track_load_finished(&second);
    second.render(&parsed.body, None);
    wait_for(&finished, Duration::from_secs(5));
    pump();
    assert!(
        !second_seen.borrow().is_empty(),
        "the reader that drew the message never resolved its images"
    );
    assert!(
        first_seen.borrow().is_empty(),
        "a reader that drew nothing was asked for another reader's images: {:?}",
        first_seen.borrow()
    );
    window.set_visible(false);
}

/// A blob source with nothing in it, for a render that needs no `cid:` parts.
struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

/// Fetch `urls` the way the app's fetcher would, but only from `127.0.0.1`:
/// anything else is dropped unasked, so no case here reaches a real host.
fn loopback_fetch(urls: Vec<String>, done: postio_gtk::reader::view::RemoteArrived) {
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
    let dir = std::env::temp_dir().join(format!("postio-gtk-reader-test-{}", std::process::id()));
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
    let drawn = postio_render::render(&request, postio_gtk::body_view::font_set());
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

/// The one case, named for a libtest-compatible runner.
const CASES: &[(&str, fn())] = &[(
    "the_reader_renders_and_hardens_the_corpus",
    the_reader_renders_and_hardens_the_corpus as fn(),
)];

/// `harness = false`, so the case above runs on the main thread.
///
/// # What a libtest thread cost
///
/// libtest runs every `#[test]` on a thread it spawns — `--test-threads=1`
/// included, which only stops it spawning *more* than one. WebKit gives each
/// thread that asks a `WTF::RunLoop` of its own, and dropping a `WebView`
/// does not tear its process down where the drop happens: it queues the
/// destruction of the `WebProcessProxy` — and so of its `WebsiteDataStore`,
/// which must send `removeSession` to the network process — onto that
/// RunLoop.
///
/// Nothing drains a spawned thread's RunLoop. When the thread ends, glibc
/// runs `WTF::RunLoop::threadWillExit` from `__nptl_deallocate_tsd` and the
/// queued work is *destroyed* rather than run, on a thread already exiting,
/// where `IPC::Connection`'s lock is never granted. The process then parks
/// forever at 0% CPU with every thread asleep. Caught live with `gdb -p`:
///
/// ```text
/// WTF::RunLoop::threadWillExit
///  -> WebKit::WebProcessProxy::~WebProcessProxy
///  -> WebKit::WebsiteDataStore::~WebsiteDataStore
///  -> WebKit::NetworkProcessProxy::removeSession
///  -> IPC::Connection::sendMessageWithAsyncReply
///  -> WTF::LockAlgorithm<...>::lockSlow            <- never returns
/// ```
///
/// The main thread's RunLoop is never torn down before `exit()`, so the same
/// work completes there. That is the whole fix, and it is why the case
/// asserts its own thread name rather than trusting this file to keep its
/// `[[test]]` entry: delete `harness = false` from `Cargo.toml` and the
/// binary goes back to hanging for 241s on every CI run it meets.
///
/// # Why not `gtk_suite`
///
/// `check-one-gtk-test-per-binary.py` points there, and for an ordinary GTK
/// case it is right. This one owns a `TcpListener` and proves that nothing
/// ever connects to it; a shared binary makes that claim about every case
/// beside it too. It stays alone, and takes the harness with it.
///
/// The `--list` output is a contract with whatever runs this binary: a runner
/// takes `--ignored` as a subset of plain `--list`, so answering both with
/// the same names tells a process-per-test runner that everything is ignored
/// — it then runs nothing and reports success. `gtk_suite`'s `main` carries
/// the same shape and `list_contract.rs` is what notices when it drifts.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let only_ignored = arguments.iter().any(|a| a == "--ignored");

    if arguments.iter().any(|a| a == "--list") {
        // Nothing here is ignored, so `--ignored` names nothing.
        if !only_ignored {
            for (name, _) in CASES {
                println!("{name}: test");
            }
        }
        // `--format terse` is machine-readable: the names and nothing else.
        // The count is what `cargo test` and the tooling's test counting read.
        if !arguments.iter().any(|a| a == "terse") {
            println!();
            let listed = if only_ignored { 0 } else { CASES.len() };
            println!("{listed} tests, 0 benchmarks");
        }
        return;
    }

    if only_ignored {
        println!("\ntest result: ok. 0 passed; 0 failed");
        return;
    }

    // `--exact` means the argument is a whole test name, not a substring: a
    // process-per-test runner passes it for every case.
    let exact = arguments.iter().any(|a| a == "--exact");
    let filters: Vec<&str> = arguments
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let mut failed = 0usize;
    let mut ran = 0usize;
    for (name, case) in CASES {
        let matched = filters
            .iter()
            .any(|f| if exact { name == f } else { name.contains(f) });
        if !filters.is_empty() && !matched {
            continue;
        }
        ran += 1;
        println!("test {name} ...");
        if std::panic::catch_unwind(case).is_err() {
            println!("test {name} ... FAILED");
            failed += 1;
        } else {
            println!("test {name} ... ok");
        }
    }

    if failed == 0 {
        println!("\ntest result: ok. {ran} passed; 0 failed");
    } else {
        println!(
            "\ntest result: FAILED. {} passed; {failed} failed",
            ran - failed
        );
        std::process::exit(1);
    }
}
