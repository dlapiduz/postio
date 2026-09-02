//! The formatting commands, through the bridge: #338.
//!
//! Each registry command lands as [`Editor::format`] or
//! [`Editor::create_link`], runs the editing machinery the dialect contract
//! pins, crosses the bridge as an ordinary edit, and shows up as canonical
//! [`Document`] structure. This is the round trip the issue's acceptance
//! names — command in, `Strong`/`List`/`Quote`/`Link` out — plus the scheme
//! gate: an address outside http/https/mailto is refused before it can
//! reach markup, and says so by return value.
//!
//! One test function: GTK is single-threaded and initialised once.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::{Block, Document, Inline};
use postio_gtk::editor::{Editor, Format};
use webkit6::prelude::*;

fn settle(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "timed out waiting for {what}");
}

fn eval_str(view: &webkit6::WebView, script: &str) -> String {
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let slot = result.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            let value = outcome
                .map(|value| value.to_str().to_string())
                .unwrap_or_default();
            *slot.borrow_mut() = Some(value);
        },
    );
    settle("the script to answer", || result.borrow().is_some());
    result.borrow_mut().take().unwrap_or_default()
}

/// Load `document` and wait for the page swap to actually commit — `load`
/// is asynchronous, and a gesture fired between loads lands on the page
/// that is leaving.
fn load_synced(editor: &Editor, document: Document, expected_text: &str) {
    editor.load(document);
    settle("the page swap to commit", || {
        eval_str(editor.widget(), "document.body.textContent") == expected_text
    });
}

fn eval(view: &webkit6::WebView, script: &str) {
    let done = Rc::new(RefCell::new(false));
    let slot = done.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            outcome.expect("the script runs");
            *slot.borrow_mut() = true;
        },
    );
    settle("the script to answer", || *done.borrow());
}

/// Select `from..to` inside the first paragraph's text node.
fn select(view: &webkit6::WebView, from: u32, to: u32) {
    eval(
        view,
        &format!(
            "(() => {{ const text = document.body.firstChild.firstChild; \
               const sel = window.getSelection(); \
               sel.setBaseAndExtent(text, {from}, text, {to}); \
               return 'selected'; }})()"
        ),
    );
}

fn text_paragraph(text: &str) -> Document {
    let mut document = Document::new();
    document
        .blocks
        .push(Block::Paragraph(vec![Inline::Text(text.into())]));
    document
}

pub fn every_formatting_command_lands_as_canonical_structure() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::ZERO);
    window.set_child(Some(editor.widget()));
    window.present();

    // ── bold on a selection becomes Strong ────────────────────────────────
    load_synced(
        &editor,
        text_paragraph("make this strong"),
        "make this strong",
    );
    select(editor.widget(), 5, 9);
    editor.format(Format::Bold);
    settle("bold to land as Strong", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
        )
    });

    // ── italic likewise ───────────────────────────────────────────────────
    load_synced(&editor, text_paragraph("lean on this"), "lean on this");
    select(editor.widget(), 0, 4);
    editor.format(Format::Italic);
    settle("italic to land as Emphasis", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if inlines.iter().any(|inline| matches!(inline, Inline::Emphasis(_)))
        )
    });

    // ── the list toggles, both kinds ──────────────────────────────────────
    load_synced(&editor, text_paragraph("item one"), "item one");
    select(editor.widget(), 0, 0);
    editor.format(Format::BulletList);
    settle("a bulleted list", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::List { ordered: false, .. })
        )
    });
    load_synced(&editor, text_paragraph("first"), "first");
    select(editor.widget(), 0, 0);
    editor.format(Format::NumberedList);
    settle("a numbered list", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::List { ordered: true, .. })
        )
    });

    // ── the quote block toggles on and back off ───────────────────────────
    load_synced(&editor, text_paragraph("as you said"), "as you said");
    select(editor.widget(), 0, 0);
    editor.format(Format::QuoteBlock);
    settle("a quote block", || {
        matches!(editor.document().blocks.first(), Some(Block::Quote(_)))
    });
    editor.format(Format::QuoteBlock);
    settle("the quote toggled back to a paragraph", || {
        matches!(editor.document().blocks.first(), Some(Block::Paragraph(_)))
    });

    // ── a link on the selection, and the scheme gate ──────────────────────
    load_synced(
        &editor,
        text_paragraph("read this later"),
        "read this later",
    );
    select(editor.widget(), 5, 9);
    assert!(editor.create_link("https://example.com/doc"));
    settle("the link to land", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Paragraph(inlines)) if inlines.iter().any(|inline| matches!(
                inline,
                Inline::Link { href, .. } if href.as_str() == "https://example.com/doc"
            ))
        )
    });

    let before = editor.document();
    assert!(
        !editor.create_link("javascript:alert(1)"),
        "a scheme outside the subset must be refused, not narrowed later"
    );
    assert!(
        !editor.create_link("file:///etc/passwd"),
        "file: is not mail either"
    );
    // Refused means untouched: give any stray report a beat to arrive.
    for _ in 0..10 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        editor.document(),
        before,
        "a refused link still changed the document"
    );
}
