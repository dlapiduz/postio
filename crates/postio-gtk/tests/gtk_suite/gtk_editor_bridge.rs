//! The editor bridge, end to end: #337.
//!
//! A [`Document`] goes in, WebKit's editing dialect comes out over the
//! script-message channel, and the bridge parses it straight back into the
//! canonical document — the DOM as a working copy, never the record
//! (ADR 0004 Q3). This proves the loop against a live WebKit: an edit
//! reaches [`Editor::document`], a typing run coalesces into one undo step,
//! a pause starts a new one, and `undo`/`redo` move both the record and the
//! DOM together.
//!
//! Coalescing is tested with injected windows (`Editor::with_coalesce`),
//! never against the wall clock: a huge window must merge two edits, a zero
//! window must split them — deterministic on any machine, per the
//! under-load doctrine.
//!
//! One test function: GTK is single-threaded and initialised once.

use crate::settle_until as settle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Document, Inline};
use postio_gtk::editor::Editor;
use webkit6::prelude::*;

fn eval(view: &webkit6::WebView, script: &str) -> String {
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let slot = result.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            let value = outcome.expect("the script runs").to_str().to_string();
            *slot.borrow_mut() = Some(value);
        },
    );
    settle("the script to answer", || result.borrow().is_some());
    result.borrow_mut().take().unwrap()
}

/// Type `text` at the end of the body, through the same editing machinery a
/// keystroke uses, so the bridge script's `input` listener fires.
fn type_text(view: &webkit6::WebView, text: &str) {
    eval(
        view,
        &format!(
            "(() => {{ \
               const sel = window.getSelection(); \
               const range = document.createRange(); \
               range.selectNodeContents(document.body); \
               range.collapse(false); \
               sel.removeAllRanges(); sel.addRange(range); \
               document.execCommand('insertText', false, '{text}'); \
               return 'typed'; \
             }})()"
        ),
    );
}

fn paragraph(text: &str) -> Block {
    Block::Paragraph(vec![Inline::Text(text.into())])
}

pub fn an_edit_becomes_the_document_and_undo_walks_typing_runs() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    // A huge window first: every edit in this phase is one typing run.
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::from_secs(3600));
    window.set_child(Some(editor.widget()));
    window.present();

    let loaded = Rc::new(RefCell::new(false));
    editor.widget().connect_load_changed({
        let loaded = loaded.clone();
        move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *loaded.borrow_mut() = true;
            }
        }
    });

    let changes = Rc::new(RefCell::new(0u32));
    editor.connect_changed({
        let changes = changes.clone();
        move |_| *changes.borrow_mut() += 1
    });

    let mut start = Document::new();
    start.blocks.push(paragraph("hello"));
    editor.load(start.clone());
    settle("the draft to load", || *loaded.borrow());
    assert_eq!(editor.document(), start, "load must not count as an edit");
    assert_eq!(*changes.borrow(), 0);

    // ── an edit crosses the bridge and becomes the document ───────────────
    type_text(editor.widget(), " world");
    settle("the edit to reach the record", || {
        editor.document() != start
    });
    assert_eq!(
        editor.document().blocks,
        vec![paragraph("hello world")],
        "the reported dialect did not parse to the edit that was made"
    );
    assert!(*changes.borrow() >= 1, "no change notification fired");

    // ── the same run amends, one undo takes the whole run back ────────────
    type_text(editor.widget(), " again");
    settle("the second edit to land", || {
        editor.document().blocks == vec![paragraph("hello world again")]
    });
    editor.undo();
    settle("undo to restore the pre-run document", || {
        editor.document() == start
    });
    assert!(
        !editor.can_undo(),
        "two edits inside one coalescing window must be one step"
    );
    settle("undo to reseed the DOM too", || {
        eval(editor.widget(), "document.body.textContent") == "hello"
    });

    // ── redo walks forward again ──────────────────────────────────────────
    editor.redo();
    settle("redo to reapply the run", || {
        editor.document().blocks == vec![paragraph("hello world again")]
    });

    // ── a zero window splits runs ─────────────────────────────────────────
    let split = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::ZERO);
    window.set_child(Some(split.widget()));
    let reloaded = Rc::new(RefCell::new(false));
    split.widget().connect_load_changed({
        let reloaded = reloaded.clone();
        move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *reloaded.borrow_mut() = true;
            }
        }
    });
    split.load(Document::new());
    settle("the second editor to load", || *reloaded.borrow());

    type_text(split.widget(), "one");
    settle("the first split edit", || {
        split.document().blocks == vec![paragraph("one")]
    });
    type_text(split.widget(), " two");
    settle("the second split edit", || {
        split.document().blocks == vec![paragraph("one two")]
    });
    split.undo();
    settle("one undo to unwind only the second edit", || {
        split.document().blocks == vec![paragraph("one")]
    });
    assert!(
        split.can_undo(),
        "a zero coalescing window must keep the runs separate"
    );

    // ── The fold is chrome, and must not become the message ─────────────
    //
    // A reply's quote is wrapped in `<details><summary>Quoted message</summary>`
    // by `postio_ui::editor::document::fold_quotes`, so that is what sits in
    // the editor's DOM. The bridge posts that DOM back and `parse` reads it,
    // and `summary` is not a tag the authoring subset knows -- so its text was
    // collected as ordinary content and the draft grew a line saying "Quoted
    // message". Typing one character into a reply was enough, and the
    // recipient got it.
    //
    // Driven through the editor rather than over `parse` directly, because the
    // fold is added by the document assembly on the way *in*: a unit test over
    // `parse` would have to hand-write the very markup whose shape is the
    // thing in question.
    let mut reply = Document::new();
    reply.blocks.push(paragraph("Acknowledged."));
    reply.blocks.push(Block::Quoted(postio_body::quote_of(
        Some("<p>Do not reset.</p>"),
        "Do not reset.",
        "q1",
    )));
    editor.load(reply);
    settle("the folded reply to load", || {
        eval(
            editor.widget(),
            "document.querySelectorAll('details').length",
        ) == "1"
    });

    // Typing is what makes this happen, and is the whole point: the host's
    // document is only rebuilt from the DOM when the bridge reports an edit,
    // so reading it back without typing returns what was loaded and asserts
    // nothing.
    // Into the paragraph the reply is written in, not at the end of `body` --
    // that lands after the `<details>`, where an insert has nowhere to go.
    eval(
        editor.widget(),
        "(() => { const p = document.body.firstChild; \
           const r = document.createRange(); \
           r.setStart(p.firstChild, p.firstChild.length); r.collapse(true); \
           const s = window.getSelection(); s.removeAllRanges(); s.addRange(r); \
           document.execCommand('insertText', false, ' typed'); \
           return 'typed'; })()",
    );
    settle("the edit to cross the bridge", || {
        editor.document().to_text().contains("typed")
    });

    let after = editor.document();
    let text = after.to_text();
    assert!(
        !text.contains("Quoted message"),
        "the fold's summary became part of the message: {text:?}"
    );
    assert!(
        after
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quoted(_))),
        "and the quote itself must survive the round trip: {after:?}"
    );
    assert!(
        text.contains("Acknowledged."),
        "the reply's own words went missing: {text:?}"
    );
}
