//! The preview pane on a real display: what it shows, and what it drops.
//!
//! Where the query matched is worked out purely, in `postio-search` and in
//! `search::mark_html`, and tested there. What needs a display is the pane's
//! behaviour as the focus moves: that the highlighting answers the query the
//! box is actually holding, that arrowing to a new result does not leave the
//! previous message's body on screen, that a body arriving late for a result
//! nobody is looking at any more is thrown away, and that every state says
//! something rather than going blank.
//!
//! Skips without a display. Nothing here touches the network — and the pane
//! renders through the hardened reader, which is the reason it cannot.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::search::View;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{MailboxId, MessageId, ThreadId};
use postio_model::{EmailAddress, MessageBody};
use postio_search::SearchHit;
use postio_search::highlight::{MATCH_END, MATCH_START};

/// The interaction budget from CLAUDE.md. Arrowing a row has to fit inside it.
const INTERACTION_BUDGET: Duration = Duration::from_millis(16);

#[test]
fn the_preview_follows_the_focus_and_answers_the_query_on_screen() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let view = View::attach(&window.shell(), &window.finder());
    window.present();
    pump();

    let finder = window.finder();
    let preview = view.preview();

    let opened: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    preview.connect_open({
        let opened = opened.clone();
        move |message| opened.borrow_mut().push(message)
    });

    // -- nothing focused is not nothing to say ----------------------------

    assert_eq!(preview.focused(), None);
    assert!(
        note(&window).contains("Arrow through the results"),
        "an empty pane still has to name a way forward: {:?}",
        note(&window)
    );

    // -- the terms come from the query the box is holding -----------------

    window.open_finder(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: "maildir from:lena is:unread".to_owned(),
    });
    pump();
    assert_eq!(
        view.terms(),
        ["maildir", "lena"],
        "`is:unread` has no text in the message to point at"
    );

    // -- a hit shows its subject and its snippet, matches picked out ------

    let first = hit(
        1,
        "Re: maildir index rebuild",
        "a 40k-message [maildir] takes",
    );
    view.set_focused(Some(&first));
    pump();
    assert_eq!(preview.focused(), Some(MessageId::new(1)));
    assert_eq!(subject(&window), "Re: maildir index rebuild");
    assert_eq!(
        snippet_runs(&window),
        [
            ("a 40k-message ".to_string(), false),
            ("maildir".to_string(), true),
            (" takes".to_string(), false),
        ],
        "the markers FTS5 put around the match become the highlight"
    );
    assert!(
        note(&window).contains("not been fetched"),
        "the body is not here yet, and the pane says so rather than looking broken"
    );

    // -- arrowing through results keeps up --------------------------------

    let second = hit(
        2,
        "Flamegraphs from the cold run",
        "the [maildir] walk dominates",
    );
    let moved = Instant::now();
    view.set_focused(Some(&second));
    let elapsed = moved.elapsed();
    pump();
    assert!(
        elapsed < INTERACTION_BUDGET,
        "moving the preview took {elapsed:?}, over the {INTERACTION_BUDGET:?} budget"
    );
    assert_eq!(preview.focused(), Some(MessageId::new(2)));
    assert_eq!(subject(&window), "Flamegraphs from the cold run");

    // -- a body lands only in the preview that asked for it ---------------

    preview.set_body(
        MessageId::new(1),
        &body("the rebuild walks every maildir once per folder"),
        Some("lena@example.com"),
    );
    pump();
    assert!(
        note(&window).contains("not been fetched"),
        "that body belongs to the result the user already arrowed past"
    );

    preview.set_body(
        MessageId::new(2),
        &body("the maildir walk dominates the profile"),
        Some("lena@example.com"),
    );
    pump();
    assert!(
        !note_visible(&window),
        "the body arrived, so there is nothing left to apologise for"
    );

    // -- and it renders through the hardened reader, not beside it --------

    let reader = find(&window.clone().upcast(), &|widget| {
        widget.type_().name().contains("WebKitWebView")
    })
    .expect("the preview renders in a WebView");
    let web_view: webkit6::WebView = reader.downcast().expect("a WebView");
    assert!(
        !webkit6::prelude::WebViewExt::settings(&web_view)
            .expect("a WebView has settings")
            .enables_javascript(),
        "a preview is still someone else's HTML"
    );

    // -- staying on the same result keeps the body it already has ---------

    view.set_focused(Some(&second));
    pump();
    assert!(
        !note_visible(&window),
        "a redraw of the same result must not throw away a body that arrived"
    );

    // -- Ret opens what is previewed, through the registry's own verb -----

    preview.open();
    assert_eq!(opened.borrow().as_slice(), [MessageId::new(2)]);

    // -- leaving search takes the preview down ----------------------------

    window.close_finder();
    pump();
    assert_eq!(
        preview.focused(),
        None,
        "nothing is previewed once there is no search"
    );

    window.destroy();
}

fn hit(id: i64, subject: &str, snippet: &str) -> SearchHit {
    SearchHit {
        message_id: MessageId::new(id),
        thread_id: Some(ThreadId::new(id)),
        mailbox_id: MailboxId::new(1),
        subject: Some(subject.to_owned()),
        from: Some(EmailAddress::new(Some("Lena Tomlin"), "lena@example.com")),
        received_at: chrono::Utc::now(),
        snippet: snippet
            .replace('[', &MATCH_START.to_string())
            .replace(']', &MATCH_END.to_string()),
        score: -1.0,
    }
}

fn body(text: &str) -> MessageBody {
    MessageBody {
        text: Some(text.to_owned()),
        html: None,
    }
}

fn subject(window: &Window) -> String {
    find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-preview-subject")
    })
    .and_then(|widget| widget.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .unwrap_or_default()
}

/// What the pane is saying about itself, or `""` when it is saying nothing.
///
/// Visibility, not just text: a hidden label keeps whatever it last said, so
/// reading the text alone would pass for a note that is no longer on screen.
fn note(window: &Window) -> String {
    let Some(label) = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-preview-note")
    }) else {
        return String::new();
    };
    if !label.property::<bool>("visible") {
        return String::new();
    }
    label
        .downcast::<gtk::Label>()
        .map(|label| label.text().to_string())
        .unwrap_or_default()
}

fn note_visible(window: &Window) -> bool {
    !note(window).is_empty()
}

/// The snippet as `(text, matched)` runs, read back off the Pango markup the
/// label was actually given — so this asserts on what is drawn rather than on
/// the function that computed it.
fn snippet_runs(window: &Window) -> Vec<(String, bool)> {
    let label = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-preview-snippet")
    })
    .expect("the preview has a snippet");
    let markup = label
        .downcast::<gtk::Label>()
        .expect("a label")
        .label()
        .to_string();

    let mut runs = Vec::new();
    let mut rest = markup.as_str();
    while let Some(start) = rest.find("<b>") {
        if start > 0 {
            runs.push((rest[..start].to_string(), false));
        }
        rest = &rest[start + 3..];
        let end = rest.find("</b>").expect("a closed bold run");
        runs.push((rest[..end].to_string(), true));
        rest = &rest[end + 4..];
    }
    if !rest.is_empty() {
        runs.push((rest.to_string(), false));
    }
    runs
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
