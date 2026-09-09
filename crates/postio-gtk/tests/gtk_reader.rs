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
use webkit6::prelude::*;

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

    // And it computes. A selector that lost on specificity, or a custom
    // property assumed to inherit where it does not, would leave the text
    // above intact and paint nothing at all — so the engine is asked, in
    // both directions.
    let painted = computed(&document, ".postio-body", "background-color");
    assert_eq!(
        painted,
        rgb(document::reader_ground(false)),
        "the sender's box should actually be painted the light ground"
    );
    let theme_document = document::document_for(
        "<p>hi</p>",
        postio_body::RemoteImages::Blocked,
        document::Sheet::Theme,
    );
    assert_eq!(
        computed(&theme_document, ".postio-body", "background-color"),
        "rgba(0, 0, 0, 0)",
        "an ordinary document must leave the box unpainted, so it shows the \
         chrome's ground through -- that is what makes the sheet a change"
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
    assert!(
        reader.is_reader_view(),
        "a shipping notice is bulk mail and opens reduced"
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

    // ── the same proof for a beacon carried in a *style* ──────────────────
    //
    // #1325 admitted the sender's inline styling, and with it a second route
    // to the network that no `<img>` appears in: `background-image: url(…)`.
    // Spec FR-022 is explicit that the rule covers every way a message can
    // name a resource, "including from within its styling", so the isolation
    // proof has to cover it too.
    //
    // Two mechanisms should refuse this — `sanitize::contain_declarations`
    // drops a remote `url()` while images are blocked, and the CSP's
    // `img-src` refuses the fetch if the sanitizer ever missed one. Defence
    // in depth is only defence if something checks both layers are there.
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
            std::thread::sleep(Duration::from_millis(10));
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
    rendering_the_next_message_keeps_the_web_process();
    each_reader_costs_a_web_process_of_its_own();
    sender_script_is_refused_even_with_javascript_enabled();
    the_counters_see_what_the_reader_actually_does();
    a_senders_width_cannot_make_the_pane_scroll_sideways();
    one_senders_styling_cannot_reach_another_message();
    a_whole_thread_costs_one_web_process();
}

/// **The proof #1323 exists for.** With JavaScript enabled at the engine
/// level, script that arrives *in a message* still does not run.
///
/// The reader turns JavaScript off wholesale today, and that is the strongest
/// possible answer. It is also the one thing standing between the conversation
/// pane and a rail that marks what you are actually reading: once a whole
/// conversation is one document, message positions live in coordinates only
/// the engine has, and with script off nothing can ask it.
/// `document::scroll_markers` solved the other direction without script —
/// anchors at `top: Nvh` moved by fragment navigation — and there is no
/// fragment trick for document → application.
///
/// So the question is whether WebKit's two switches really separate *whose*
/// script runs, rather than merely how much. `enable_javascript_markup(false)`
/// claims to ignore script arriving in the document while leaving the
/// application's own injections working. If that claim holds, the guarantee a
/// user cares about — a message cannot run code — survives turning JavaScript
/// on for Postio's own observer. If it does not, the rail marks by navigation
/// instead and the spec is amended; the sanitizer is never the thing that
/// gives way.
///
/// `document.title` is the channel, because it is observable from the
/// application **without** script: `WebView::title()` reads it directly. A
/// document that changed its own title would prove its script ran even in a
/// view where nothing could be evaluated to ask.
fn sender_script_is_refused_even_with_javascript_enabled() {
    // No `Content-Security-Policy` in this document, deliberately. The
    // reader's real documents carry `script-src 'none'` and that is a second,
    // independent refusal -- which is exactly why it cannot be in here. With
    // both present, a passing assertion would not say *which* one refused the
    // script, and the whole question is whether the engine setting does.
    //
    // Three ways a message can carry code: a script element, an
    // event-handler attribute on an element guaranteed to fire it, and a
    // handler on the body. Each writes a distinct title, so a failure names
    // which one got through rather than only that one did.
    let hostile = "<!DOCTYPE html><html><head><title>quiet</title></head>\
        <body onload=\"document.title='body onload ran'\">\
        <script>document.title = 'script element ran';</script>\
        <img src=\"postio-cid:nothing-resolves-this\" \
             onerror=\"document.title='onerror ran'\">\
        </body></html>";

    // The control, and the reason this spike is worth anything. Run the same
    // document with markup script *allowed*: if the title does not change
    // there either, then something else in this harness is refusing it and
    // the real assertion below would be passing for a reason that has nothing
    // to do with the switch under test.
    let (permitted, _) = load_and_read_title(true, hostile);
    assert_ne!(
        permitted, "quiet",
        "the control did not run the message's script even with markup \
         enabled, so this spike cannot tell the switch apart from whatever \
         else refused it -- fix the fixture before trusting the result"
    );

    let (refused, injected) = load_and_read_title(false, hostile);

    assert_eq!(
        refused, "quiet",
        "script that arrived in the message ran ({permitted:?} got through \
         with markup enabled, and enable_javascript_markup(false) did not \
         stop it). The switch does not separate the sender's script from the \
         application's, so the rail must mark by navigation and spec.md's \
         FR-034/FR-035 need amending -- do not weaken postio-body's sanitizer \
         to get around this"
    );
    assert_eq!(
        injected, "the application still speaks",
        "the application's own script did not run, so there is nothing to be \
         gained by enabling JavaScript at all"
    );
}

/// Load `document` with JavaScript on and markup script `markup`, and report
/// the title afterwards plus what an injected script sees.
///
/// `document.title` is the channel because it is observable from the
/// application **without** script: `WebView::title()` reads it directly, so a
/// document that changed its own title is caught even in a view where nothing
/// could be evaluated to ask.
fn load_and_read_title(markup: bool, document: &str) -> (String, String) {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(true);
    settings.set_enable_javascript_markup(markup);

    let (title, injected, weak) = {
        // Its own ephemeral session and context, dropped with the view, for
        // `computed`'s reason: the default `WebContext` is process-global and
        // its WebProcess outlives every scope here.
        let network_session = webkit6::NetworkSession::new_ephemeral();
        let context = webkit6::WebContext::new();
        let view = webkit6::WebView::builder()
            .settings(&settings)
            .web_context(&context)
            .network_session(&network_session)
            .build();
        let window = gtk::Window::new();
        window.set_child(Some(&view));
        window.present();

        let loaded = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&loaded);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        view.load_html(document, None);
        wait_for(&loaded, Duration::from_secs(5));

        // The broken image has to actually fail before `onerror` has had its
        // chance; a title read too early would pass for the wrong reason.
        pump_for(Duration::from_millis(300));

        let title = view.title().map(|t| t.to_string()).unwrap_or_default();

        // The other half of the claim, and the reason enabling this is worth
        // anything: Postio's own script still runs. `script-src 'none'`
        // governs what the *page* may load and execute; it has never governed
        // the host application's injections, which is why `computed` above
        // has been evaluating JavaScript against reader documents that carry
        // that very directive all along.
        let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        view.evaluate_javascript(
            "document.title = 'the application still speaks'; document.title",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                *slot.borrow_mut() = Some(
                    outcome
                        .map(|value| value.to_str().to_string())
                        .unwrap_or_default(),
                );
            },
        );
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
        while answer.borrow().is_none() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        let injected = answer.borrow_mut().take().unwrap_or_default();

        let weak = view.downgrade();
        window.set_child(None::<&gtk::Widget>);
        window.destroy();
        (title, injected, weak)
    };
    // #794: a view still alive at `exit()` kills the binary after every test
    // has been reported as passing. Same discipline as `computed`.
    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        weak.upgrade().is_none(),
        "the spike's WebView outlived its window, so its WebProcess is still \
         attached at exit -- #794 all over again"
    );
    (title, injected)
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
            "one Reader is one rendering surface -- and one web process, which \
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
        let document = document::document_for(
            body,
            postio_body::RemoteImages::Blocked,
            document::Sheet::Theme,
        );

        // `scrollWidth` is what the document *would* need; `clientWidth` is
        // what the window gives it. The pane scrolls sideways exactly when the
        // first exceeds the second.
        let overflowing = measure(
            &document,
            "document.documentElement.scrollWidth > document.documentElement.clientWidth",
        );
        assert_eq!(
            overflowing, "false",
            "{name}: a {} px declaration made the whole pane scroll sideways, \
             so `.postio-body`'s overflow-x is no longer containing what #1325 \
             admitted",
            4000
        );

        // And it is contained rather than clipped: the content is still there
        // to scroll to inside its own box. A container that simply hid the
        // overflow would pass the assertion above and lose the message.
        let reachable = measure(
            &document,
            "(() => { const b = document.querySelector('.postio-body'); \
              return b.scrollWidth > b.clientWidth ? 'scrollable' : 'fits'; })()",
        );
        assert_eq!(
            reachable, "scrollable",
            "{name}: four thousand pixels of content in a six hundred pixel \
             window is not reachable by scrolling its own box, so it is being \
             clipped away instead of contained -- which passes the assertion \
             above while losing the message"
        );
    }
}

/// Can one message's inline styling move another message? (#1346)
///
/// Two pieces of work rest on opposite answers. #1327 admitted the sender's
/// inline styling, reasoning that an inline declaration has no selector and
/// so reaches its own subtree and nothing else. #1316's note says the
/// opposite in terms — that stripping `style` is *"the only"* reason several
/// senders can share a page — and the one-document conversation is built on
/// that premise.
///
/// The inheritance argument is sound as far as it goes and does not cover the
/// ways an element affects a **sibling** without inheriting into it: `float`
/// escapes normal flow, a negative `margin` pulls content outside its own
/// box, and `transform` paints outside the box without changing layout.
/// #1327's refusal table names neither.
///
/// So it is asked of the engine, in the arrangement that makes contamination
/// possible at all: two messages in one document. Geometry is compared
/// against the identical document with a benign first message, because an
/// absolute pixel value would only say the layout is what it is.
fn one_senders_styling_cannot_reach_another_message() {
    // Everything #1327 does not refuse, in one message.
    // A block that occupies its own 120px, then paints itself 120px lower --
    // exactly over whatever follows it. `transform` is the operative part: it
    // paints above normal flow, so the intruder lands *on top of* the next
    // message rather than beside or beneath it. #1327 refuses neither
    // transform nor a negative margin.
    //
    // On one line deliberately. In a raw string a trailing `\` is a literal
    // backslash, not a line continuation, and it lands inside the attribute
    // where CSS reads it as an escape and swallows the declaration after it.
    // Three earlier versions of this test "passed" against a payload broken
    // exactly that way.
    const HOSTILE: &str = r#"<div id="intruder" style="height:120px;transform:translateY(120px);background:#f0f">first</div>"#;
    const SECOND: &str = r#"<p id="probe">second</p>"#;

    // **What is actually being asked.** Not whether two boxes overlap: a
    // float contributes nothing to its parent's height, so box arithmetic
    // reports "clear" while the float paints straight over the message below.
    // The question a person experiences is *what is drawn at this point*, so
    // that is what is asked -- hit-test inside the second message and see
    // whose element answers.
    let hit_at_probe = |wrap: bool| -> String {
        let first = if wrap {
            document::contain_body(HOSTILE)
        } else {
            format!("<div>{HOSTILE}</div>")
        };
        let second = if wrap {
            document::contain_body(SECOND)
        } else {
            format!("<div>{SECOND}</div>")
        };
        let document = document::wrap_document(
            &format!("{first}{second}{}", document::scroll_markers()),
            postio_body::RemoteImages::Blocked,
            document::Sheet::Theme,
        );
        measure(
            &document,
            "(() => { const p = document.getElementById('probe') \
              .getBoundingClientRect(); \
              const hit = document.elementFromPoint(p.left + p.width / 2, \
                                                    p.top + p.height / 2); \
              return hit ? (hit.id || hit.tagName) : 'nothing'; })()",
        )
    };

    // The control first, and it is what makes the rest mean anything: without
    // the container, the hostile message *does* reach the one below it.
    // Measured: the intruder occupies 136-256 and the probe 136-157, so they
    // overlap exactly, and the transformed element paints above normal flow. If
    // this ever reports the probe, the styling is being refused by something
    // else and the assertion below is a coincidence rather than a containment.
    assert_eq!(
        hit_at_probe(false),
        "intruder",
        "the hostile styling did not reach the message below even without a \
         container, so it is not hostile enough to prove anything"
    );

    // And with each message in its own `.postio-body`, it does not.
    assert_eq!(
        hit_at_probe(true),
        "probe",
        "one sender's content is drawn over another sender's mail. #1327's \
         REFUSED table needs float, transform and negative margin BEFORE the \
         one-document conversation lands"
    );
}

/// Evaluate `expression` against `document` and return it as a string.
///
/// [`computed`]'s twin, for questions that are not a computed style. Same
/// lifecycle discipline, and the same reason for it (#794).
fn measure(document: &str, expression: &str) -> String {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(true);

    let (value, weak) = {
        let network_session = webkit6::NetworkSession::new_ephemeral();
        let context = webkit6::WebContext::new();
        let view = webkit6::WebView::builder()
            .settings(&settings)
            .web_context(&context)
            .network_session(&network_session)
            .build();
        let window = gtk::Window::new();
        window.set_default_size(600, 500);
        window.set_child(Some(&view));
        window.present();

        let loaded = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&loaded);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        view.load_html(document, None);
        wait_for(&loaded, Duration::from_secs(5));
        // Layout has to have happened before a width means anything.
        pump_for(Duration::from_millis(200));

        let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        view.evaluate_javascript(
            expression,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                *slot.borrow_mut() = Some(
                    outcome
                        .map(|value| value.to_str().to_string())
                        .unwrap_or_default(),
                );
            },
        );
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
        while answer.borrow().is_none() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        let value = answer.borrow_mut().take().unwrap_or_default();

        let weak = view.downgrade();
        window.set_child(None::<&gtk::Widget>);
        window.destroy();
        (value, weak)
    };
    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        weak.upgrade().is_none(),
        "the measuring WebView outlived its window -- #794 all over again"
    );
    value
}

/// Wait for the listener to report a connection, pumping GTK meanwhile.
/// The web processes **this test** owns, by pid.
///
/// Scoped to our own descendants, and that is not a detail. The first version
/// counted every `WebKitWebProces` on the machine, and a developer running
/// this with Postio open counts *its* web processes -- which are alive before
/// and after whatever the test does, so the assertion passed without ever
/// observing the reader. WebKit puts the process under a `bwrap` sandbox, so
/// the walk is up the parent chain rather than a direct child check.
///
/// `-x` against the **truncated** name: Linux cuts `comm` to fifteen
/// characters, so it is `WebKitWebProces` and `pgrep -x WebKitWebProcess`
/// matches nothing, warning about it on stderr where it is easy to miss. `-f`
/// is worse -- it matches the `bwrap` wrapper too, counting each process
/// twice, and this binary's own command line with them.
fn web_processes() -> Vec<i32> {
    let mine = std::process::id() as i32;
    let out = std::process::Command::new("pgrep")
        .args(["-x", "WebKitWebProces"])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .filter(|pid| descends_from(*pid, mine))
        .collect()
}

/// Whether `pid` has `ancestor` somewhere above it.
fn descends_from(pid: i32, ancestor: i32) -> bool {
    let mut current = pid;
    for _ in 0..16 {
        if current == ancestor {
            return true;
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{current}/status")) else {
            return false;
        };
        let Some(parent) = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .and_then(|value| value.trim().parse::<i32>().ok())
        else {
            return false;
        };
        if parent <= 1 {
            return false;
        }
        current = parent;
    }
    false
}

/// Rendering a second message must reuse the first one's web process.
///
/// The report is a **black flicker** moving between messages, alongside a
/// WebKit web process spawning and dying about once a second while doing it.
/// The two are one thing: a process that has just started has not finished
/// relocating its libraries -- the first profile of this burst was
/// `do_lookup_x`, `_dl_relocate_object_no_relro` and little else -- and a
/// process with nothing drawn yet composites black.
///
/// Nothing in Postio asks for a second process. `Reader` builds its `WebView`
/// once and reuses it, `show_occupant` only toggles visibility, and every
/// message loads through `load_html` against the same `postio-reader:///`
/// base, so there is no cross-site navigation to swap on. This asks whether
/// WebKit replaces the process regardless.
fn rendering_the_next_message_keeps_the_web_process() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("process-reuse"),
    );
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let first = postio_model::mime::parse(test_corpus::load("multipart-alternative").bytes());
    let finished = track_load_finished(&reader);
    reader.render(&first.body, None);
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let before = web_processes();
    assert!(
        !before.is_empty(),
        "no web process after rendering a message, so this test cannot see \
         what it is about"
    );

    let second = postio_model::mime::parse(test_corpus::load("html-newsletter").bytes());
    let finished = track_load_finished(&reader);
    reader.render(&second.body, None);
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let after = web_processes();

    let kept: Vec<i32> = before
        .iter()
        .filter(|p| after.contains(p))
        .copied()
        .collect();
    eprintln!("before {before:?}, after {after:?}, kept {kept:?}");
    window.set_visible(false);

    assert!(
        !kept.is_empty(),
        "every web process alive after the first message was gone after the \
         second, so rendering a message replaces the process rather than \
         reusing it -- which is the black flicker, since a process that has \
         just started has nothing drawn. before={before:?} after={after:?}"
    );
}

/// A second reader costs a second web process, and that is the flicker.
///
/// The conversation pane builds a `Reader` per expanded message -- lazily, so
/// a thirty-message thread does not cost thirty views up front, and `collapse`
/// keeps the widget so reopening is cheap. Both of those are right. What is
/// left is that the *first* expansion of each message still builds one, and
/// `Reader::with_allowlist` gives every reader a `WebContext` of its own.
/// WebKit runs a web process per context, so moving through a conversation
/// spawns one per message -- which is what "a process a second while opening
/// messages" was, and why each arrives showing black: it has not finished
/// relocating its libraries.
///
/// Recorded rather than fixed. Sharing one context needs the `postio-reader:`
/// scheme handler to route by URI instead of closing over one message's
/// blobs, which is a change to how parts are addressed, not a tuning knob.
fn each_reader_costs_a_web_process_of_its_own() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let window = gtk::Window::new();
    let first = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("one-context"),
    );
    window.set_child(Some(&first.widget()));
    window.present();
    let parsed = postio_model::mime::parse(test_corpus::load("multipart-alternative").bytes());
    let finished = track_load_finished(&first);
    first.render(&parsed.body, None);
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let after_one = web_processes();

    // A second reader, as the conversation pane makes for the next message.
    let second = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("two-contexts"),
    );
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // Unparent before re-parenting, or `append` refuses the child --
    //   Gtk-CRITICAL: gtk_box_append: assertion 'gtk_widget_get_parent
    //   (child) == NULL' failed
    // -- and leaves `first` where it was, for the `set_child` below to
    // unparent instead, mid-load, with its WebView live.
    window.set_child(None::<&gtk::Widget>);
    holder.append(&first.widget());
    holder.append(&second.widget());
    window.set_child(Some(&holder));
    let finished = track_load_finished(&second);
    second.render(&parsed.body, None);
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let after_two = web_processes();

    eprintln!("one reader {after_one:?}, two readers {after_two:?}");
    window.set_visible(false);

    assert!(
        after_two.len() > after_one.len(),
        "a second reader did not cost a second web process ({after_one:?} -> \
         {after_two:?}). If that is now true, the contexts are shared and the \
         comment above is stale -- delete it rather than the assertion"
    );
}

/// A blob source with nothing in it, for a render that needs no `cid:` parts.
struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

fn wait_for_connection(rx: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + postio_test_support::scaled(timeout);
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
/// What a rendering engine actually computes for `selector`'s `property`,
/// given `document`.
///
/// The string assertions elsewhere prove the right CSS reached the document.
/// They cannot prove the CSS *applies* — a selector that loses on specificity,
/// or a custom property that does not inherit where it was assumed to, writes
/// exactly the same text into exactly the same place and paints nothing. So
/// this asks the engine.
///
/// A scratch `WebView` with JavaScript **on**, and deliberately not the
/// reader's: the reading pane runs with script off by construction (ADR 0003)
/// and must keep doing so, which is precisely why a test cannot interrogate
/// it and needs an instrument of its own. Nothing sender-authored is ever
/// loaded here — the argument is a document Postio composed.
fn computed(document: &str, selector: &str, property: &str) -> String {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(true);

    // Weakly held past the end, because a `WebView` still alive at `exit()`
    // is #794: WebKit reports `WebProcess didn't exit as expected after the
    // UI process connection was closed` and the binary dies on the way out,
    // *after* reporting every test as passed. An instrument that leaks one
    // per call would reintroduce exactly that, so this releases it and says
    // so — the same mechanism `gtk_reader_teardown` asserts, at the one place
    // in this binary that builds views of its own.
    let (value, weak) = {
        // Its own ephemeral session and context, dropped with the view: the
        // default `WebContext` is process-global and its WebProcess outlives
        // every scope this helper has.
        let network_session = webkit6::NetworkSession::new_ephemeral();
        let context = webkit6::WebContext::new();
        let view = webkit6::WebView::builder()
            .settings(&settings)
            .web_context(&context)
            .network_session(&network_session)
            .build();
        let window = gtk::Window::new();
        window.set_child(Some(&view));
        window.present();

        let loaded = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&loaded);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        view.load_html(document, None);
        wait_for(&loaded, Duration::from_secs(5));

        let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        view.evaluate_javascript(
            &format!(
                "getComputedStyle(document.querySelector('{selector}'))\
                 .getPropertyValue('{property}')"
            ),
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                *slot.borrow_mut() = Some(
                    outcome
                        .map(|value| value.to_str().to_string())
                        .unwrap_or_default(),
                );
            },
        );
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
        while answer.borrow().is_none() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        let value = answer.borrow_mut().take().unwrap_or_default();

        let weak = view.downgrade();
        window.set_child(None::<&gtk::Widget>);
        window.destroy();
        (value, weak)
    };
    // GTK finalizes on the main loop, not at the closing brace.
    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        weak.upgrade().is_none(),
        "the measuring WebView outlived its window, so its WebProcess is \
         still attached at exit -- #794 all over again"
    );
    value
}

/// A `#rrggbb` from the generated palette, as a rendering engine reports it.
fn rgb(hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("a #rrggbb colour");
    format!("rgb({}, {}, {})", channel(0), channel(2), channel(4))
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
    let deadline = Instant::now() + postio_test_support::scaled(timeout);
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
        std::thread::sleep(Duration::from_millis(10));
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

/// ADR 0032's claim, measured: a thread of any length is one web process.
///
/// The stacked pane builds a `Reader` per expanded message, and
/// `each_reader_costs_a_web_process_of_its_own` above is why that is not
/// free — WebKitGTK runs a process per *view*, so a thirty-message thread
/// ends with thirty of them. One document in one view should cost one,
/// whatever the thread's length, and this is what says whether it does.
///
/// Counted rather than timed, and counted against a *long* thread and a
/// short one in the same reader: the number that matters is that it does not
/// grow.
fn a_whole_thread_costs_one_web_process() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    // Before this reader exists: the cases above it in this binary leave
    // their own readers alive, and what this measures is what *this* one
    // costs, not what the process happens to be holding.
    let before = web_processes();

    let window = gtk::Window::new();
    let reader = Reader::with_allowlist(
        Rc::new(NoBlobs),
        RemoteImageAllowList::default(),
        scratch_path("one-document-thread"),
    );
    window.set_child(Some(&reader.widget()));
    window.present();

    let parsed = postio_model::mime::parse(test_corpus::load("multipart-alternative").bytes());
    let thread = |count: usize| -> Vec<postio_gtk::reader::view::ThreadMessage> {
        (0..count)
            .map(|index| postio_gtk::reader::view::ThreadMessage {
                scope: index.to_string(),
                sender: format!("Sender {index}"),
                address: format!("sender{index}@example.com"),
                when: "09:14".into(),
                preview: "the first line".into(),
                expanded: index + 1 == count,
                latest: index + 1 == count,
                body: parsed.body.clone(),
            })
            .collect()
    };

    let finished = track_load_finished(&reader);
    reader.render_thread(&thread(2));
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let short = web_processes();

    let finished = track_load_finished(&reader);
    reader.render_thread(&thread(30));
    wait_for(&finished, Duration::from_secs(5));
    pump();
    let long = web_processes();

    eprintln!("2 messages {short:?}, 30 messages {long:?}");
    window.set_visible(false);

    assert_eq!(
        short.len(),
        before.len() + 1,
        "the thread never rendered, or it cost more than one view's process \
         ({before:?} -> {short:?}), and either way the count below proves \
         nothing"
    );
    assert_eq!(
        long.len(),
        short.len(),
        "a thirty-message thread cost more web processes than a two-message \
         one ({short:?} -> {long:?}), which is the whole claim of ADR 0032"
    );

    // And every message is actually in the document -- a view that rendered
    // one message would also pass the count above.
    let document = reader.test_document();
    assert_eq!(
        document.matches("<details").count(),
        30,
        "the document does not hold the whole thread"
    );
}
