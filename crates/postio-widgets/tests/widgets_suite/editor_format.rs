use crate::support_compose::settle_until as settle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use postio_body::{Block, Document, HeadingLevel, Inline};
use postio_widgets::composer::editor::Editor;
use postio_widgets::composer::surface::Format;
use webkit6::prelude::*;

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

/// `load` is asynchronous: a gesture fired between loads lands on the page that is leaving.
fn load_synced(editor: &Editor, document: Document, expected_text: &str) {
    editor.load(document);
    // The script too, not only the text (#1716): a gesture that lands
    // before the editor script is listening is never reported.
    settle("the page swap to commit, its script listening", || {
        eval_str(editor.widget(), "document.body.textContent") == expected_text
            && eval_str(editor.widget(), crate::support_compose::EDITOR_LISTENING) == "true"
    });
    // Also on screen and laid out (#1800): a selection in unlaid-out text collapses to a caret and
    // `execCommand` toggles a typing style.
    settle("the editor to be on screen with its text laid out", || {
        editor.widget().is_mapped()
            && eval_str(
                editor.widget(),
                "String(document.body.firstChild.getClientRects().length > 0)",
            ) == "true"
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

/// A command on a collapsed selection toggles the typing style, so nothing changes and nothing
/// arrives; fail within seconds, naming the command (#1859).
fn lands(editor: &Editor, command: &str, landed: impl Fn() -> bool) {
    let limit = postio_test_support::scaled(Duration::from_secs(10));
    let start = std::time::Instant::now();
    loop {
        crate::support_compose::settle();
        if landed() {
            return;
        }
        if let Some(reason) = postio_widgets::composer::web_process::take_death() {
            panic!("a WebKit web process died ({reason}) while waiting for {command}");
        }
        if start.elapsed() >= limit {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let selection = eval_str(
        editor.widget(),
        "(() => { const s = window.getSelection(); \
           return 'collapsed=' + s.isCollapsed + ' ranges=' + s.rangeCount \
             + ' selected_chars=' + String(s).length; })()",
    );
    panic!(
        "{command} produced no document change within {limit:?}; the selection \
         at that point was: {selection} (a collapsed selection means the \
         command toggled a typing style instead of formatting text, #1859)"
    );
}

/// POSTIO-MEASUREMENT: a sweep of every formatting command through the real editor,
/// 30 s on a CI runner and the widgets suite's longest-running flake
/// (#1800), so it runs on the nightly timer; the per-command cases stay.
pub fn every_formatting_command_lands_as_canonical_structure() {
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::ZERO);
    window.set_child(Some(editor.widget()));
    window.present();

    load_synced(
        &editor,
        text_paragraph("make this strong"),
        "make this strong",
    );
    select(editor.widget(), 5, 9);
    editor.format(Format::Bold);
    lands(&editor, "bold to land as Strong", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
        )
    });

    load_synced(&editor, text_paragraph("lean on this"), "lean on this");
    select(editor.widget(), 0, 4);
    editor.format(Format::Italic);
    lands(&editor, "italic to land as Emphasis", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if inlines.iter().any(|inline| matches!(inline, Inline::Emphasis(_)))
        )
    });

    load_synced(&editor, text_paragraph("item one"), "item one");
    select(editor.widget(), 0, 0);
    editor.format(Format::BulletList);
    lands(&editor, "a bulleted list", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::List {
                kind: postio_body::ListKind::Bullet,
                ..
            })
        )
    });
    load_synced(&editor, text_paragraph("first"), "first");
    select(editor.widget(), 0, 0);
    editor.format(Format::NumberedList);
    lands(&editor, "a numbered list", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::List {
                kind: postio_body::ListKind::Ordered { .. },
                ..
            })
        )
    });

    load_synced(&editor, text_paragraph("Agenda"), "Agenda");
    select(editor.widget(), 0, 0);
    editor.format(Format::Heading1);
    lands(&editor, "a first-level heading", || {
        matches!(
            editor.document().blocks.first(),
            Some(Block::Heading {
                level: HeadingLevel::One,
                ..
            })
        )
    });
    editor.format(Format::Heading1);
    lands(&editor, "the heading toggled back to a paragraph", || {
        matches!(editor.document().blocks.first(), Some(Block::Paragraph(_)))
    });

    load_synced(&editor, text_paragraph("as you said"), "as you said");
    select(editor.widget(), 0, 0);
    editor.format(Format::QuoteBlock);
    lands(&editor, "a quote block", || {
        matches!(editor.document().blocks.first(), Some(Block::Quote(_)))
    });
    editor.format(Format::QuoteBlock);
    lands(&editor, "the quote toggled back to a paragraph", || {
        matches!(editor.document().blocks.first(), Some(Block::Paragraph(_)))
    });

    load_synced(
        &editor,
        text_paragraph("read this later"),
        "read this later",
    );
    select(editor.widget(), 5, 9);
    assert!(editor.create_link("https://example.com/doc"));
    lands(&editor, "the link to land", || {
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
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        editor.document(),
        before,
        "a refused link still changed the document"
    );
}
