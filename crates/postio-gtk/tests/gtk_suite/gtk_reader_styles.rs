//! The reader's own behaviour in a real window: paging, its header, and
//! what a thread document says.
//!
//! The cases that asked WebKit's cascade and layout through a probe view
//! -- one sender's stylesheet against another's (#1326), the user's own
//! mark (#1241), the header column (#1437) and the accent segments (#1688)
//! -- are asked of the renderer that draws the reader now, in
//! `postio-render`'s `thread_document` suite.

use gtk::prelude::*;

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
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    window.set_child(Some(&reader.widget()));
    window.present();
    crate::pump();
    let message = |scope: &str| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: "Ada Norwood".to_owned(),
        address: "ada@example.com".to_owned(),
        when: "09:14".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "the first line".to_owned(),
        expanded: true,
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: postio_model::message::MessageBody {
            text: Some("a body long enough to have somewhere to scroll. ".repeat(200)),
            html: None,
        },
    };

    // The control: with nothing rendered, a page key must still do nothing.
    reader.page_down();
    assert_eq!(
        reader.scrolled_for_test(),
        0.0,
        "a page key moved a pane with nothing in it"
    );

    reader.render_thread(&[message("1"), message("2"), message("3")]);
    crate::settle_until("the thread to be drawn", || {
        reader
            .view()
            .document()
            .is_some_and(|d| d.size.height > 1000.0)
    });
    reader.page_down();
    assert_ne!(
        reader.scrolled_for_test(),
        0.0,
        "`space` and `Page_Down` do nothing in the one-document pane. The \
         guard reads `open`, which `render_thread` never sets (#1431)"
    );

    let after_down = reader.scrolled_for_test();
    reader.page_up();
    assert!(
        reader.scrolled_for_test() < after_down,
        "the page went down and would not come back up"
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

/// A message whose body has not arrived says so, in the thread document.
///
/// The single-message pane has explained this since `Absent::Partial` existed
/// -- "Downloading this message / Its headers are here; the body has not
/// arrived yet". ADR 0032 made a conversation one document, and that path
/// never called `absent_html`: a message with no body contributed an empty
/// section and the reader saw a message that would not open, with no reason
/// given. Measured against a real account, where 174 headers were in and one
/// body was.
///
/// Both halves matter. The words have to be the ones the other pane uses, or
/// the app has two ways of saying one thing; and only the *open* message may
/// say them, because `expanded_in_document` opens every message in a thread
/// and thirty plates explaining one fact is the noise this avoided.
pub fn a_body_that_has_not_arrived_says_so_in_the_thread() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = postio_gtk::reader::Reader::new(std::rc::Rc::new(|_id: &str| None));
    let waiting = |scope: &str, expanded: bool| postio_gtk::reader::view::ThreadMessage {
        scope: scope.to_owned(),
        sender: "Ada Norwood".to_owned(),
        address: "ada@example.com".to_owned(),
        when: "09:14".to_owned(),
        recipients: String::new(),
        cc: String::new(),
        preview: "the first line".to_owned(),
        expanded,
        absent: true,
        latest: false,
        draft: false,
        mine: false,
        body: postio_model::message::MessageBody::default(),
    };

    reader.render_thread(&[waiting("1", true), waiting("2", false)]);
    crate::settle();
    let document = reader.test_document();

    let expected =
        postio_ui::reader::document::absent_html(postio_ui::reader::document::Absent::Partial);
    let heading = "Downloading this message";
    assert!(
        document.contains(heading),
        "an open message with no body must say the body is still coming, in \
         the same words the single-message pane uses: {document}"
    );
    assert!(
        document.contains("aria-live"),
        "the plate carries its own live region, so a screen reader is told \
         when the body arrives: {document}"
    );
    assert_eq!(
        document.matches(heading).count(),
        1,
        "only the open message explains it -- every message in a thread is \
         expanded by policy, and one fact stated thirty times is noise"
    );
    assert!(
        expected.contains(heading),
        "sanity: the words come from `absent_html`, not from this test"
    );
}

/// A redraw of an unchanged thread sanitises nothing and loads nothing
/// (#1605).
///
/// `ConversationView`'s redraw asked `would_render_thread` -- which composed
/// the whole document to compare it -- and then `render_thread`, which
/// composed it again; each compose put every body through the sanitiser, on
/// the main thread, every time anything queued a redraw.
pub fn a_redraw_of_an_unchanged_thread_sanitises_nothing() {
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
        absent: false,
        latest: false,
        draft: false,
        mine: false,
        body: postio_model::message::MessageBody {
            text: None,
            html: Some(format!("<p>message {scope}</p>")),
        },
    };
    let thread = [message("1"), message("2"), message("3")];

    let before = postio_ui::test_support::bodies_sanitised();
    assert!(
        reader.render_thread_if_changed(&thread),
        "the first draw draws"
    );
    assert_eq!(
        postio_ui::test_support::bodies_sanitised() - before,
        3,
        "the first draw sanitises each body once, not once to decide and once to draw"
    );

    let loads = reader.loads();
    let before = postio_ui::test_support::bodies_sanitised();
    assert!(
        !reader.render_thread_if_changed(&thread),
        "an unchanged thread is not drawn again"
    );
    assert_eq!(
        postio_ui::test_support::bodies_sanitised() - before,
        0,
        "a redraw of an unchanged thread re-sanitised its bodies"
    );
    assert_eq!(
        reader.loads(),
        loads,
        "an unchanged thread reloaded the view"
    );

    let mut edited = thread.clone();
    edited[1].body.html = Some("<p>message 2, arrived whole</p>".to_owned());
    let before = postio_ui::test_support::bodies_sanitised();
    assert!(
        reader.render_thread_if_changed(&edited),
        "a changed body draws"
    );
    assert_eq!(
        postio_ui::test_support::bodies_sanitised() - before,
        1,
        "one body changed, and only it is sanitised again"
    );
}
