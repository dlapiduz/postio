//! Markdown as input (spec 002, US6), through a live engine.
//!
//! The point of the feature is that it changes nothing about what a draft is:
//! a typed sequence runs one of the formatting commands the toolbar and the
//! palette already offer, and what comes out the other side of the bridge is
//! the same canonical [`Document`] structure those commands produce. So every
//! assertion here is about the `Document`, not about the DOM — the DOM is a
//! working copy and never the record (ADR 0004 Q3).
//!
//! Its own file rather than a case added to `gtk_editor_format.rs`: that one
//! is the suite's known slow case and the one #957 catches flaking, and
//! lengthening it would make a bad situation worse.
//!
//! One test function: GTK is single-threaded and initialised once.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Document, Inline};
use postio_gtk::editor::Editor;
use postio_gtk::reader::scheme::BlobSource;
use webkit6::prelude::*;

use crate::settle_until as settle;

struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
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

/// Type `text` one character at a time, the way a person does — which is the
/// only way the transformation can see it, since it fires per `insertText`.
fn type_text(view: &webkit6::WebView, text: &str) {
    for character in text.chars() {
        let escaped = character
            .to_string()
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        eval_str(
            view,
            &format!(
                "(() => {{ document.execCommand('insertText', false, '{escaped}'); \
                      return 'typed'; }})()"
            ),
        );
    }
}

/// An editor with the caret in an empty body, ready to be typed into.
fn ready(editor: &Editor) {
    editor.load(Document::default());
    settle("the empty page to commit", || {
        eval_str(editor.widget(), "document.body.textContent").is_empty()
    });
    eval_str(
        editor.widget(),
        "(() => { document.body.focus(); \
          const p = document.createElement('p'); \
          p.appendChild(document.createTextNode('')); \
          document.body.replaceChildren(p); \
          const r = document.createRange(); \
          r.setStart(p.firstChild, 0); r.collapse(true); \
          const s = window.getSelection(); s.removeAllRanges(); s.addRange(r); \
          return 'ready'; })()",
    );
}

fn first_block(editor: &Editor) -> Option<Block> {
    editor.document().blocks.into_iter().next()
}

/// Whether `blocks` holds a `Strong` run saying `text`, however nested.
fn has_strong(blocks: &[Block], text: &str) -> bool {
    blocks.iter().any(|block| match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            inlines.iter().any(|inline| match inline {
                Inline::Strong(inner) => inner
                    .iter()
                    .any(|i| matches!(i, Inline::Text(t) if t == text)),
                _ => false,
            })
        }
        Block::List { items, .. } => items.iter().any(|item| has_strong(item, text)),
        Block::Quote(inner) => has_strong(inner, text),
        _ => false,
    })
}

pub fn typed_markdown_becomes_the_formatting_its_command_produces() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(
        Rc::new(NoBlobs) as Rc<dyn BlobSource>,
        Duration::from_millis(1),
    );
    window.set_child(Some(editor.widget()));
    window.present();

    // ── FR-067: `**` reaches the bold command ────────────────────────────
    ready(&editor);
    type_text(editor.widget(), "a **b** c");
    settle("the bold run to cross the bridge", || {
        has_strong(&editor.document().blocks, "b")
    });

    let document = editor.document();
    assert!(
        has_strong(&document.blocks, "b"),
        "`**b**` did not become bold: {document:?}"
    );
    // FR-069: the markers are gone. They are how the formatting was asked
    // for, not part of what was written, and a recipient seeing `**b**`
    // in bold is the failure this whole feature would be.
    let text = document.to_text();
    assert!(
        !text.contains("**"),
        "the literal markers survived into the message: {text:?}"
    );
    assert!(
        text.contains("a b c"),
        "the surrounding words did not survive: {text:?}"
    );

    // ── FR-067: `- ` reaches the bulleted-list command ───────────────────
    ready(&editor);
    type_text(editor.widget(), "- a");
    settle("the list text to cross the bridge", || {
        !editor.document().blocks.is_empty()
    });
    settle("the list to cross the bridge", || {
        matches!(
            first_block(&editor),
            Some(Block::List { ordered: false, .. })
        )
    });
    assert!(
        matches!(
            first_block(&editor),
            Some(Block::List { ordered: false, .. })
        ),
        "`- ` did not start a bulleted list: {:?}",
        editor.document()
    );
    assert!(
        !editor.document().to_text().starts_with("- -"),
        "the marker was left behind as well as acted on: {:?}",
        editor.document().to_text()
    );

    // ── FR-067: `1. ` reaches the numbered-list command ──────────────────
    ready(&editor);
    type_text(editor.widget(), "1. a");
    settle("the ordered list to cross the bridge", || {
        matches!(
            first_block(&editor),
            Some(Block::List { ordered: true, .. })
        )
    });
    assert!(
        matches!(
            first_block(&editor),
            Some(Block::List { ordered: true, .. })
        ),
        "`1. ` did not start a numbered list: {:?}",
        editor.document()
    );

    // ── FR-067: `> ` reaches the quote command ───────────────────────────
    ready(&editor);
    type_text(editor.widget(), "> a");
    settle("the quote to cross the bridge", || {
        matches!(first_block(&editor), Some(Block::Quote(_)))
    });
    assert!(
        matches!(first_block(&editor), Some(Block::Quote(_))),
        "`> ` did not start a quote: {:?}",
        editor.document()
    );

    // ── FR-070: one undo puts the literal characters back ────────────────
    //
    // The escape hatch, and the thing that makes an automatic conversion
    // tolerable at all: if you meant the asterisks, you press undo once and
    // they are there, unconverted. Once -- not three times for the three
    // edits the transformation happened to make internally, which is what
    // the user would face if this were built out of separate operations
    // the undo stack sees separately.
    ready(&editor);
    type_text(editor.widget(), "a **b**");
    settle("the bold run to cross the bridge", || {
        has_strong(&editor.document().blocks, "b")
    });
    // The host's undo, not the engine's. `EditHistory` is Postio's own stack
    // over the `Document` -- `edit.rs` is explicit that the widget's is not
    // the record -- so `document.execCommand('undo')` would be testing
    // something the user never reaches.
    editor.undo();
    settle("the undo to cross the bridge", || {
        !has_strong(&editor.document().blocks, "b")
    });
    let undone = editor.document().to_text();
    assert!(
        undone.contains("**b**"),
        "one undo did not put the literal markers back, so someone who meant \
         the asterisks has no way to keep them: {undone:?}"
    );
    assert!(
        !has_strong(&editor.document().blocks, "b"),
        "the undo restored the markers and left the formatting on: {:?}",
        editor.document()
    );

    // ── FR-071: what was not converted is sent as it was shown ───────────
    //
    // A hyphen mid-sentence is a hyphen, and an asterisk with no partner is
    // an asterisk. The transformation must not go looking for markers in
    // prose -- someone writing about a shell glob is writing about a shell
    // glob.
    ready(&editor);
    type_text(editor.widget(), "a -v b *.eml");
    settle("the prose to cross the bridge", || {
        editor.document().to_text().contains("-v")
    });
    let text = editor.document().to_text();
    assert!(
        text.contains("a -v b *.eml"),
        "prose that merely looks like markdown was rewritten: {text:?}"
    );
    assert!(
        !matches!(first_block(&editor), Some(Block::List { .. })),
        "a mid-sentence hyphen started a list: {:?}",
        editor.document()
    );

    // ── FR-072: the plain-text alternative carries no stray markers ──────
    //
    // Two conversions with a single space between them was #1486, and the
    // cause was not in this file at all: `parse` dropped a whitespace-only
    // text node between two *loose* inlines, which is exactly what the
    // editor's DOM holds after a formatting command.
    //
    // The half nobody looks at, and the one where a doubled marker shows up:
    // `to_text` renders a bold run as its text, so a surviving `**` here
    // would mean the markers were kept *and* the formatting applied.
    ready(&editor);
    type_text(editor.widget(), "**x** *y*");
    settle("both runs to cross the bridge", || {
        has_strong(&editor.document().blocks, "x")
    });
    let text = editor.document().to_text();
    assert_eq!(
        text.matches('*').count(),
        0,
        "markers reached the plain-text alternative: {text:?}"
    );
    assert!(
        text.contains("x y"),
        "the words did not survive the conversion: {text:?}"
    );
}
