//! One sender's stylesheet against another sender's message (#1326).

use gtk::prelude::*;
use webkit6::prelude::*;

/// Can one message's `<style>` block restyle another message? (#1326)
///
/// The question ADR 0032 turns on. Its stated reason for putting a whole
/// conversation in one document was that the sanitizer had deleted every
/// sender's CSS, so messages could not contaminate each other. #1325 admitted
/// inline styling and #1326 admits `<style>` blocks, so that reason is gone
/// and the containment now rests on `postio_body::styles` rewriting every
/// selector under its own message's container.
///
/// That is a claim about a *parser*, and the parser's own tests can only
/// check the text it emits. Whether the text means what it is supposed to
/// mean is a question for the engine, so this asks the engine.
///
/// Cascade, not layout: unlike `one_senders_styling_cannot_reach_another_message`
/// this needs no boxes, so it runs everywhere including CI (#1307).
pub fn one_senders_stylesheet_cannot_restyle_another_message() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    const RED: &str = "rgb(1, 2, 3)";
    // Two rules a sender would plausibly write, and one they would not:
    // `p` reaches the other message's paragraphs, `body` is how a great deal
    // of real mail sets its type, and `.postio-blocked` is the notice saying
    // this very message's images were held back.
    let hostile = format!(
        "<style>p {{ color: {RED} }} body {{ color: {RED} }} \
         .postio-blocked {{ display: none }}</style><p>first</p>"
    );

    let sanitized = postio_body::sanitize::sanitize_body_in(
        &hostile,
        postio_body::RemoteImages::Blocked,
        Some("1"),
    );
    let innocent = postio_body::sanitize::sanitize_body_in(
        "<p>second</p>",
        postio_body::RemoteImages::Blocked,
        Some("2"),
    );

    fn entry<'a>(
        scope: &'a str,
        body: &'a str,
        styles: &'a str,
    ) -> postio_ui::reader::thread::Entry<'a> {
        postio_ui::reader::thread::Entry {
            scope,
            sender: "Ada Lovelace",
            address: "ada@example.com",
            when: "09:14",
            preview: "the first line of it",
            expanded: true,
            latest: false,
            blocked: 0,
            body,
            styles,
            recipients: "",
            cc: "",
        }
    }
    // Selected through the containers Postio itself writes: ammonia drops a
    // sender's `id`, so a probe element cannot carry one of its own.
    let mine = postio_body::sanitize::message_selector(Some("1"));
    let theirs = postio_body::sanitize::message_selector(Some("2"));

    let document = postio_ui::reader::thread::conversation_document(
        &[
            entry("1", &sanitized.html, &sanitized.styles),
            entry("2", &innocent.html, &innocent.styles),
        ],
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );

    // **The control, first.** Without it every assertion below passes when
    // the stylesheet is simply dropped -- which is what the code did before
    // #1326 and is not what it is supposed to do now. FR-019 says a message
    // renders as its sender built it.
    assert_eq!(
        crate::webkit_probe::computed(&document, &format!("{} p", mine), "color"),
        RED,
        "the sender's own rule did not reach their own message, so the \
         assertions below prove nothing: a dropped stylesheet contaminates \
         no one either"
    );

    assert_ne!(
        crate::webkit_probe::computed(&document, &format!("{} p", theirs), "color"),
        RED,
        "one sender's `<style>` restyled another sender's message. ADR 0032 \
         puts them in one document; `postio_body::styles` is the only thing \
         keeping them apart"
    );
}

/// A page key turns the page of a conversation (#1431).
///
/// `Reader::page_down`, `page_up` and `scroll_to_message` all guarded on
/// `open`, which only `render` sets -- `render_thread` has never touched it.
/// So every one of them was a no-op in the one-document pane, which is the
/// pane conversations now open in: `space` and `Page_Down` did nothing, and
/// `J`/`K` moved focus without scrolling to it.
///
/// #1402 added the keys and its tests passed, because they assert that
/// `ConversationView::page` returned `true` -- which it did. It found a
/// document reader and called into it. Nothing asked whether the page turned,
/// and that is the assertion here.
pub fn a_page_key_actually_turns_the_page_of_a_thread() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = postio_gtk::reader::Reader::new(std::rc::Rc::new(|_id: &str| None));
    let message = |scope: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: "Ada Norwood".to_owned(),
        address: "ada@example.com".to_owned(),
        when: "09:14".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "the first line".to_owned(),
        expanded: true,
        latest: false,
        body: postio_model::message::MessageBody {
            text: Some("a body long enough to have somewhere to scroll".to_owned()),
            html: None,
        },
    };

    // The control: with nothing rendered, a page key must still do nothing.
    reader.page_down();
    assert_eq!(
        reader.page_for_test(),
        0,
        "a page key moved a pane with nothing in it"
    );

    reader.render_thread(&[message("1"), message("2"), message("3")]);
    reader.page_down();
    assert_ne!(
        reader.page_for_test(),
        0,
        "`space` and `Page_Down` do nothing in the one-document pane. The \
         guard reads `open`, which `render_thread` never sets (#1431)"
    );

    let after_down = reader.page_for_test();
    reader.page_up();
    assert!(
        reader.page_for_test() < after_down,
        "the page went down and would not come back up"
    );
}

/// The document actually moves, not just the bookkeeping (#1433).
///
/// `gtk_reader_scroll` owns the marker arithmetic -- right marker for the
/// right key, clamped, reset. It used to own the other half too, by watching
/// `view().uri()`: while scrolling was a fragment `load_uri`, the URI *was*
/// the scroll. Scrolling by script means the URI never moves, so that half
/// needs asking of the engine directly, and `window.scrollY` is the thing a
/// person would call "did it scroll".
pub fn a_page_key_moves_the_document_itself() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = postio_gtk::reader::Reader::new(std::rc::Rc::new(|_id: &str| None));
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    window.set_child(Some(&reader.widget()));
    window.present();
    crate::pump();

    // Long enough to have somewhere to go. The marker tests deliberately use
    // a short body -- they are about arithmetic -- and a short body here
    // would scroll nowhere and pass for the wrong reason.
    let long = "A paragraph of a message that goes on. ".repeat(400);
    reader.render(
        &postio_model::message::MessageBody {
            text: Some(long),
            html: None,
        },
        Some("ada@example.com"),
    );
    crate::settle_until("the body to render", || {
        !reader.document_for_test().is_empty()
    });
    crate::pump();

    let offset = || -> f64 {
        let answer = std::rc::Rc::new(std::cell::Cell::new(f64::NAN));
        let slot = std::rc::Rc::clone(&answer);
        reader.view().evaluate_javascript(
            "String(window.scrollY)",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                if let Ok(value) = outcome {
                    slot.set(value.to_str().parse().unwrap_or(f64::NAN));
                }
            },
        );
        for _ in 0..200 {
            crate::pump();
            if !answer.get().is_nan() {
                break;
            }
        }
        answer.get()
    };

    let before = offset();
    if before.is_nan() {
        eprintln!("skipping: this display reports no scroll position -- see #1307");
        window.destroy();
        return;
    }
    reader.page_down();
    crate::pump();
    let after = offset();

    assert!(
        after > before,
        "a page key moved the marker and not the document: scrollY stayed at \
         {before} (#1433)"
    );

    window.destroy();
}

/// Moving between messages does not land on "the URL cannot be shown".
///
/// `scroll_to_message` and the page keys navigate to
/// `postio-reader:///#m-<scope>`, and `DOCUMENT_BASE_URI`'s own comment says
/// "nothing is ever registered to handle this scheme". That is safe only as a
/// **same-document** fragment jump against a document already loaded at that
/// exact URI; anything else is a real load of an unhandled scheme, which
/// WebKit answers with its error page.
///
/// Reported from a real store: hopping to the next message and back to the
/// previous one shows that page.
pub fn moving_between_messages_never_loads_an_error_page() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = postio_gtk::reader::Reader::new(std::rc::Rc::new(|_id: &str| None));
    let window = gtk::Window::new();
    window.set_child(Some(&reader.widget()));
    window.present();

    let failed: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let seen = std::rc::Rc::clone(&failed);
        reader
            .view()
            .connect_load_failed(move |_, _event, uri, error| {
                seen.borrow_mut().push(format!("{uri}: {error}"));
                false
            });
    }

    let message = |scope: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: "Ada Norwood".to_owned(),
        address: "ada@example.com".to_owned(),
        when: "09:14".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "the first line".to_owned(),
        expanded: true,
        latest: false,
        body: postio_model::message::MessageBody {
            text: Some("a body with enough text to scroll past".to_owned()),
            html: None,
        },
    };
    reader.render_thread(&[message("1"), message("2"), message("3")]);
    crate::pump();

    // **Then warm, and that ordering is the whole bug.** A spare reader is
    // warmed with `load_html("", None)`, which leaves the view's URI empty.
    // While scrolling was `load_uri("postio-reader:///#m-N")`, a fragment
    // jump was a same-document scroll *only* while the URI still matched the
    // base exactly -- so after this it became a real navigation to a scheme
    // that by design has no handler, and WebKit answered with "The URL can't
    // be shown".
    //
    // Taken from a real store's log, which is the only place it showed:
    //
    //     load started -> Some("postio-reader:///")
    //     load started -> Some("")
    //     LOAD FAILED [Started] postio-reader:///#m-82161
    //
    // My first attempt at this test warmed *before* rendering and passed,
    // which is why the order is spelled out rather than left to read like an
    // accident.
    reader.warm();
    crate::pump();

    // Whatever the view is pointing at, scrolling must leave it there.
    let before = reader.view().uri().map(|uri| uri.to_string());

    // Forward, back, forward -- the gesture in the report.
    for scope in ["2", "3", "2", "1"] {
        reader.scroll_to_message(scope);
        crate::pump();
    }

    // **The assertion is that scrolling did not navigate**, not that no load
    // failed. The failure itself would not reproduce here -- a headless view
    // that is never mapped does not reach the same load path, and a test
    // asserting on `load-failed` passed against the broken code, which makes
    // it worthless as a guard.
    //
    // What *is* observable, and what actually distinguishes the two
    // mechanisms: `load_uri` moves the view's URI to
    // `postio-reader:///#m-2`, and a scripted `scrollIntoView` leaves it
    // exactly where the document was loaded. Scrolling is not navigation,
    // and the moment it becomes navigation it is at the mercy of whatever
    // the current URI happens to be (#1433).
    assert_eq!(
        reader.view().uri().map(|uri| uri.to_string()),
        before,
        "scrolling navigated the view instead of moving the document. A \
         fragment `load_uri` is a same-document scroll only while the URI \
         still matches the base exactly -- after `warm()` it is a real load \
         of a scheme with no handler, which is \"The URL can't be shown\""
    );
    assert!(
        failed.borrow().is_empty(),
        "a load failed while moving between messages: {:?}",
        failed.borrow()
    );

    window.destroy();
}

/// The single-message reader carries its verbs in the header, like the
/// conversation pane does (#1435).
///
/// Two surfaces disagreed. The conversation pane puts the action bar in the
/// header (`header.actions().set_visible(true)`, footer stood down); the
/// single-message reader appended it last, "under the attachment chips,
/// matching the canvas' footer treatment" (#498). Same build, same message,
/// and where the bar sat depended on which surface you happened to get --
/// which for a one-message row is the reader, so the older placement was the
/// one most mail showed.
///
/// Asserted as an ancestry question rather than a pixel one: the bar must be
/// *inside* the header, which is what makes it draw with the subject instead
/// of below the body.
pub fn the_readers_verbs_live_in_its_header() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = postio_gtk::reader::Reader::new(std::rc::Rc::new(|_id: &str| None));
    let header = reader.header().widget();
    let bar = reader.actions_widget();

    let mut ancestor = bar.parent();
    let mut inside_header = false;
    while let Some(widget) = ancestor {
        if widget == header {
            inside_header = true;
            break;
        }
        ancestor = widget.parent();
    }

    assert!(
        inside_header,
        "the reader's action bar is not in its header, so a message opened \
         from the list puts Reply at the bottom while the same message in a \
         conversation puts it at the top (#1435)"
    );
}

/// `From`, `To` and `Cc` line up (#1437).
///
/// **Measured, not eyeballed.** The markup was correct through two attempts
/// that both looked wrong on screen: the head is a flex row with an 8px state
/// square drawn before it by a pseudo-element, and the recipients were a grid
/// of their own, so `To` sat 18px left of `From` while every assertion about
/// the document passed. Alignment lives in the CSS, against a box no markup
/// test can see -- so this asks the engine where the boxes actually are.
pub fn from_to_and_cc_share_a_column() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let entry = postio_ui::reader::thread::Entry {
        scope: "1",
        sender: "Ada Norwood",
        address: "ada@example.com",
        when: "09:14",
        preview: "the first line",
        expanded: true,
        latest: false,
        blocked: 0,
        body: "<p>a body</p>",
        styles: "",
        recipients: "Quinn Abara <quinn.abara@example.net>",
        cc: "Grace Hopper <grace@example.com>",
    };
    let document = postio_ui::reader::thread::conversation_document(
        &[entry],
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );

    // The left edge of each label, and of each value beside it.
    let lefts = crate::webkit_probe::measure(
        &document,
        "(() => { const labels = [...document.querySelectorAll('.postio-recipients-label')]; \
          const values = labels.map(l => l.nextElementSibling); \
          return labels.map((l, i) => \
            Math.round(l.getBoundingClientRect().left) + ':' + \
            Math.round(values[i].getBoundingClientRect().left)).join(','); })()",
    );

    let rows: Vec<(i32, i32)> = lefts
        .split(',')
        .filter_map(|pair| pair.split_once(':'))
        .filter_map(|(label, value)| Some((label.trim().parse().ok()?, value.trim().parse().ok()?)))
        .collect();

    if rows.len() < 3 {
        eprintln!("skipping: this display reports no layout (got {lefts:?}) -- see #1307");
        return;
    }

    let (first_label, first_value) = rows[0];
    for (index, (label, value)) in rows.iter().enumerate() {
        assert_eq!(
            *label, first_label,
            "row {index}'s label starts at {label} and the first at \
             {first_label}; From, To and Cc must be one column"
        );
        assert_eq!(
            *value, first_value,
            "row {index}'s value starts at {value} and the first at \
             {first_value}; the addresses must begin together"
        );
    }
}
