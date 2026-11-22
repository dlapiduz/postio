use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Document, HeadingLevel, Inline};
use postio_widgets::composer::editor::Editor;
use postio_widgets::reader::BlobSource;
use webkit6::prelude::*;

use crate::support_compose::settle_until as settle;

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

/// One character at a time: the transformation fires per `insertText`.
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

fn ready(editor: &Editor) {
    editor.load(Document::default());
    // Wait for the editor's page and its script, not just an empty body: a fresh view's blank page
    // is empty too, and the script that reports edits arrives at document end (#1716).
    settle(
        "the editor's empty page to commit, its script listening",
        || {
            eval_str(
                editor.widget(),
                "(document.body && document.body.isContentEditable \
              && document.body.textContent === '' \
              && window.postioEditorReady === true) ? 'ready' : ''",
            ) == "ready"
        },
    );
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
    // The markers are how formatting was asked for, not part of what was written.
    let text = document.to_text();
    assert!(
        !text.contains("**"),
        "the literal markers survived into the message: {text:?}"
    );
    assert!(
        text.contains("a b c"),
        "the surrounding words did not survive: {text:?}"
    );

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

    // Only the heading: after Return the next line is a paragraph again (#1485).
    ready(&editor);
    type_text(editor.widget(), "# Agenda");
    settle("the heading to cross the bridge", || {
        matches!(
            first_block(&editor),
            Some(Block::Heading { level: HeadingLevel::One, inlines })
                if inlines == [Inline::Text("Agenda".to_owned())]
        )
    });
    eval_str(
        editor.widget(),
        "(() => { document.execCommand('insertParagraph'); return 'broke'; })()",
    );
    type_text(editor.widget(), "First");
    settle("the line after the heading to cross the bridge", || {
        editor.document().to_text().contains("First")
    });
    // `ready`'s hand-built empty paragraph leaves a stray `<br>` behind any block gesture.
    assert!(
        matches!(
            editor.document().blocks.as_slice(),
            [
                Block::Heading { level: HeadingLevel::One, .. },
                Block::Paragraph(words),
                ..
            ] if words == &[Inline::Text("First".to_owned())]
        ),
        "the line after `# Agenda` is not a paragraph of its own: {:?}",
        editor.document()
    );

    ready(&editor);
    type_text(editor.widget(), "see # 4 and #5");
    settle("the prose to cross the bridge", || {
        editor.document().to_text().contains("#5")
    });
    assert!(
        matches!(first_block(&editor), Some(Block::Paragraph(_))),
        "a mid-sentence `# ` made a heading: {:?}",
        editor.document()
    );
    assert_eq!(editor.document().to_text().trim(), "see # 4 and #5");

    // One undo puts the literal characters back, once, not three times for the transformation's
    // internal edits.
    ready(&editor);
    type_text(editor.widget(), "a **b**");
    settle("the bold run to cross the bridge", || {
        has_strong(&editor.document().blocks, "b")
    });
    // The host's undo, not the engine's: `EditHistory` is Postio's own stack over the `Document`.
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

    // What was not converted is sent as shown: no marker hunting in prose.
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

    // Two conversions with one space between them (#1486): `parse` dropped a whitespace-only text
    // node between two loose inlines.
    // `to_text` renders bold as its text, so a surviving `**` would mean markers kept and
    // formatting applied.
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
