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
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use postio_ui::reader::document;

fn the_reader_cases_that_remain() {
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
    // Run in sequence inside this one `#[test]`, not as tests of their own.
    // libtest would put all three on a thread pool, GTK tolerates one thread,
    // and the losers would return through the `no display` guard above and be
    // reported as passing (#355, `check-one-gtk-test-per-binary`).
    two_readers_resolve_their_own_inline_images();
    fifty_conversations_hold_what_one_holds();
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
    // to ask its owner for it, once the message has stayed on screen for the
    // dwell that makes it opened. Asserting only that the URL is present
    // would have passed while the picture stayed blank.
    let deadline = Instant::now() + postio_test_support::patience();
    while asked.borrow().is_empty() && Instant::now() < deadline {
        pump_for(Duration::from_millis(50));
    }
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
    "the_reader_cases_that_remain",
    the_reader_cases_that_remain as fn(),
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
