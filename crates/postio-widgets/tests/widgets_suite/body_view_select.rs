//! Selection, copy and links over the snapshot (spec 006 FR-017, FR-019).

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_widgets::body_view::BodyView;

use crate::body_view::{content, until};

fn shown(name: &str) -> (gtk::Window, gtk::ScrolledWindow, BodyView) {
    let view = BodyView::new(crate::support_reader::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content(name));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    (window, scroller, view)
}

fn clipboard_text(clipboard: &gdk::Clipboard) -> Option<String> {
    let text = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = text.clone();
    clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
        *slot.borrow_mut() = Some(result.ok().flatten().map(|s| s.to_string()));
    });
    until(|| text.borrow().is_some());
    text.borrow().clone().flatten()
}

pub fn a_drag_across_cells_selects_and_copies_them_as_rows() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, _scroller, view) = shown("html-transactional-receipt");
    let doc = view.document().expect("a snapshot");
    let from = doc.text.rects(doc.text.find("The Salt Road")[0].clone())[0];
    let to = doc.text.rects(doc.text.find("38.00")[0].clone())[0];
    view.drag_select(
        gtk::graphene::Point::new((from.x0 + 1.0) as f32, from.center().y as f32),
        gtk::graphene::Point::new((to.x1 - 1.0) as f32, to.center().y as f32),
    );
    let range = view.selection().expect("the drag selected something");
    let text = doc.text.slice(range.clone()).to_owned();
    assert!(text.starts_with("The Salt Road"), "{text:?}");
    assert!(
        text.contains("\t1\t14.00\nField Guide to Lichens\t2\t38.00"),
        "{text:?}"
    );
    assert!(!view.selection_rects().is_empty(), "no highlight is drawn");

    let display = gdk::Display::default().expect("a display");
    assert_eq!(
        clipboard_text(&display.primary_clipboard()).as_deref(),
        Some(text.as_str()),
        "the primary clipboard holds the selection"
    );
    view.activate_action("clipboard.copy", None)
        .expect("a copy action");
    assert_eq!(
        clipboard_text(&display.clipboard()).as_deref(),
        Some(text.as_str())
    );
    window.destroy();
}

pub fn double_and_triple_clicks_select_a_word_and_a_line() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, _scroller, view) = shown("html-transactional-receipt");
    let doc = view.document().expect("a snapshot");
    let lichens = doc.text.find("Lichens")[0].clone();
    let at = doc.text.rects(lichens.clone())[0].center();
    let point = gtk::graphene::Point::new(at.x as f32, at.y as f32);
    view.click_select(point, 2);
    assert_eq!(view.selection(), Some(lichens.clone()));
    view.click_select(point, 3);
    let line = doc.text.line_at(lichens.start);
    assert_eq!(view.selection(), Some(line));
    let _ = glib::MainContext::default();
    window.destroy();
}

/// A conversation of one message whose body has `body`, composed by the
/// reader's own thread document.
fn thread(body: &str) -> postio_widgets::body_view::Content {
    use postio_ui::reader::thread::{Entry, conversation_document};
    let sanitized = postio_body::sanitize::sanitize_body_in(
        body,
        postio_body::RemoteImages::Blocked,
        Some("7"),
    );
    // Quoted text is folded after sanitizing, as the reader does it.
    let folded = postio_body::fold_html_quotes(&sanitized.html);
    let entry = Entry {
        scope: "7",
        sender: "Ada",
        address: "ada@example.com",
        when: "09:14",
        preview: "the first line",
        expanded: true,
        draft: false,
        mine: false,
        latest: false,
        blocked: 0,
        styles: "",
        recipients: "",
        cc: "",
        sheet: postio_ui::reader::document::Sheet::Theme,
        body: &folded,
    };
    let mut content = content("plain-text-simple");
    content.document = conversation_document(
        &[entry],
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );
    content
}

fn show(content: postio_widgets::body_view::Content) -> (gtk::Window, BodyView) {
    let view = BodyView::new(crate::support_reader::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content);
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    (window, view)
}

pub fn links_are_followed_by_pointer_and_keyboard() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, view) = show(thread(
        "<p>See <a href=\"https://example.com/plan\">the plan</a>.</p>\
         <p><a href=\"javascript:alert(1)\">not a link</a></p>",
    ));
    let launched = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    view.set_launcher({
        let launched = launched.clone();
        move |url| launched.borrow_mut().push(url.to_owned())
    });
    let verbs = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, String)>::new()));
    view.connect_message_verb({
        let verbs = verbs.clone();
        move |_, scope, verb| verbs.borrow_mut().push((scope.to_owned(), verb.to_owned()))
    });
    let doc = view.document().expect("a snapshot");
    // Hover: the tooltip names where the link really goes.
    let plan = doc
        .links
        .iter()
        .find(|l| matches!(&l.target, postio_render::LinkTarget::External(u) if u.as_str() == "https://example.com/plan"))
        .expect("the plan link");
    let at = gtk::graphene::Point::new(plan.rect.center().x as f32, plan.rect.center().y as f32);
    view.hover(at);
    assert_eq!(
        view.tooltip_text().as_deref(),
        Some("https://example.com/plan")
    );
    // A script URL is not a link at all.
    assert!(
        !doc.links
            .iter()
            .any(|l| format!("{:?}", l.target).contains("javascript"))
    );
    // Tab walks the links in document order; Return follows the focused one.
    let order: Vec<String> = (0..doc.links.len())
        .map(|_| {
            view.focus_next_link(true);
            view.focused_link_target().expect("a focused link")
        })
        .collect();
    assert_eq!(order.len(), doc.links.len());
    while view.focused_link_target().as_deref() != Some("https://example.com/plan") {
        view.focus_next_link(true);
    }
    view.activate_focused_link();
    assert_eq!(*launched.borrow(), ["https://example.com/plan"]);
    // A click on a verb link dispatches the verb for its message.
    let reply = doc
        .links
        .iter()
        .find(|l| {
            matches!(
                l.target,
                postio_render::LinkTarget::Verb {
                    verb: postio_render::Verb::Reply,
                    ..
                }
            )
        })
        .expect("the reply verb");
    view.click_select(
        gtk::graphene::Point::new(reply.rect.center().x as f32, reply.rect.center().y as f32),
        1,
    );
    assert_eq!(*verbs.borrow(), [("7".to_owned(), "reply".to_owned())]);
    window.destroy();
}

pub fn clicking_a_fold_opens_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, view) = show(thread(
        "<p>my reply</p><blockquote>the words they wrote before</blockquote>",
    ));
    let doc = view.document().expect("a snapshot");
    assert!(
        !doc.text.text.contains("the words they wrote before"),
        "the quote starts closed"
    );
    let fold = doc
        .folds
        .iter()
        .find(|f| f.id.ends_with("-q0"))
        .expect("the quote's fold");
    let generation = doc.generation;
    view.click_select(
        gtk::graphene::Point::new(
            fold.summary_rect.center().x as f32,
            fold.summary_rect.center().y as f32,
        ),
        1,
    );
    assert!(
        until(|| view
            .document()
            .is_some_and(|d| d.generation > generation
                && d.text.text.contains("the words they wrote before"))),
        "the opened fold's text is not in the next snapshot"
    );
    window.destroy();
}

/// 001 FR-034 to FR-037 without script: the current message is the one
/// with the most of it on screen, a rail row scrolls its message to the
/// top, and page down moves one real page.
pub fn the_rail_follows_the_snapshot() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    use postio_ui::reader::thread::{Entry, conversation_document};
    let long = "<p>A paragraph that goes on for a while.</p>".repeat(40);
    let bodies: Vec<String> = ["7", "11", "15"]
        .iter()
        .map(|scope| {
            postio_body::sanitize::sanitize_body_in(
                &long,
                postio_body::RemoteImages::Blocked,
                Some(scope),
            )
            .html
        })
        .collect();
    let entries: Vec<Entry<'_>> = ["7", "11", "15"]
        .iter()
        .zip(&bodies)
        .map(|(scope, body)| Entry {
            scope,
            sender: "Ada",
            address: "ada@example.com",
            when: "09:14",
            preview: "the first line",
            expanded: true,
            draft: false,
            mine: false,
            latest: false,
            blocked: 0,
            styles: "",
            recipients: "",
            cc: "",
            sheet: postio_ui::reader::document::Sheet::Theme,
            body,
        })
        .collect();
    let mut thread = content("plain-text-simple");
    thread.document = conversation_document(
        &entries,
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );
    let (window, view) = show(thread);
    let doc = view.document().expect("a snapshot");
    let second = doc
        .messages
        .iter()
        .find(|m| m.scope == "11")
        .expect("message 11")
        .rect;
    assert_eq!(view.current_message().as_deref(), Some("7"));

    view.scroll_to_message("11");
    let adjustment = view.vadjustment().expect("scrolled");
    assert!(
        (adjustment.value() - second.y0).abs() < 1.0,
        "message 11 is not at the top"
    );
    assert_eq!(view.current_message().as_deref(), Some("11"));

    let before = adjustment.value();
    view.page(true);
    assert!(
        (adjustment.value() - before - adjustment.page_size()).abs() < 1.0,
        "page down did not move one page"
    );
    view.page(false);
    assert!(
        (adjustment.value() - before).abs() < 1.0,
        "page up did not come back"
    );
    window.destroy();
}
