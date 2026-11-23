use crate::support_compose::settle_until as settle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use postio_body::{Block, Document, Inline};
use postio_widgets::composer::editor::Editor;
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
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
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

    editor.redo();
    settle("redo to reapply the run", || {
        editor.document().blocks == vec![paragraph("hello world again")]
    });

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

    // The fold is chrome and must not become the message: `summary` is not in the authoring subset,
    // so its text was collected and the draft grew a "Quoted message" line.
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

    // Typing makes the host rebuild its document from the DOM, so reading it back without typing
    // asserts nothing.
    // Typed into the reply's paragraph, not the end of `body`, which is after the `<details>`.
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

    // The fold sits inside the editing surface, where a click places a caret; `<summary
    // contenteditable="false">` is the mitigation, and whether it suffices is a WebKit question.
    // No script runs the fold, so `open` flipping is the browser's doing.
    let open = || eval(editor.widget(), "document.querySelector('details').open");
    assert_eq!(open(), "false", "a reply's quote starts folded");

    eval(
        editor.widget(),
        "(() => { document.querySelector('details > summary').click();            return 'clicked'; })()",
    );
    settle("the fold to open", || open() == "true");
    assert_eq!(
        open(),
        "true",
        "clicking the summary did not open the quote"
    );

    eval(
        editor.widget(),
        "(() => { document.querySelector('details > summary').click();            return 'clicked'; })()",
    );
    settle("the fold to close", || open() == "false");
    assert_eq!(open(), "false", "the quote would not fold back up");
}

/// The page's script is injected at document end, after the body is already editable; typing into
/// that gap is never reported (#1716). `editor.js` sets the marker last.
pub fn the_editor_page_says_when_its_script_is_listening() {
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::from_millis(1));
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
    editor.load(Document::new());
    settle("the editor page to load", || *loaded.borrow());
    assert_eq!(
        eval(editor.widget(), crate::support_compose::EDITOR_LISTENING),
        "true",
        "a loaded editor page does not say its script is listening"
    );
}

/// A reply is written above the quote. A caret requested by a script sent straight after `load` ran
/// against the page being replaced, so typing went nowhere.
pub fn a_loaded_page_takes_typing_at_the_start_of_the_body() {
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::from_millis(1));
    window.set_child(Some(editor.widget()));
    window.present();

    let mut quoted = Document::new();
    quoted.blocks.push(paragraph("the quote"));
    editor.load(quoted);
    settle("the editor page to listen", || {
        eval(editor.widget(), crate::support_compose::EDITOR_LISTENING) == "true"
    });

    assert_eq!(
        eval(
            editor.widget(),
            "(() => { const s = window.getSelection(); \
               if (s.rangeCount === 0) return 'no caret'; \
               const r = document.createRange(); \
               r.selectNodeContents(document.body); r.collapse(true); \
               return s.isCollapsed && s.getRangeAt(0).compareBoundaryPoints( \
                 Range.START_TO_START, r) === 0 ? 'start' : 'elsewhere'; })()",
        ),
        "start",
        "a freshly loaded page has no caret at the start of the body"
    );

    eval(
        editor.widget(),
        "(() => { document.execCommand('insertText', false, 'typed '); return 'ok'; })()",
    );
    settle("the typing to reach the document", || {
        editor.document().blocks == vec![paragraph("typed the quote")]
    });
    assert_eq!(
        editor.document().blocks,
        vec![paragraph("typed the quote")],
        "typing into a fresh page did not land at the start of the body"
    );
}

/// The page keeps a selection from load so typing lands at the start; WebKit draws it as a caret
/// even while the keyboard is in To, so the page paints it transparent until the view has the
/// keyboard.
pub fn the_caret_is_drawn_only_while_the_body_has_the_keyboard() {
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let address = gtk::Entry::new();
    let editor = Editor::with_coalesce(Rc::new(|_: &str| None), Duration::from_millis(1));
    editor.widget().set_vexpand(true);
    row.append(&address);
    row.append(editor.widget());
    window.set_child(Some(&row));
    window.present();
    address.grab_focus();

    let mut quoted = Document::new();
    quoted.blocks.push(paragraph("the quote"));
    editor.load(quoted);
    settle("the editor page to listen", || {
        eval(editor.widget(), crate::support_compose::EDITOR_LISTENING) == "true"
    });

    let caret = "getComputedStyle(document.body).caretColor";
    let selection = "String(window.getSelection().rangeCount)";
    settle("the caret to be hidden while To has the keyboard", || {
        eval(editor.widget(), caret) == "rgba(0, 0, 0, 0)"
    });
    assert_eq!(
        eval(editor.widget(), selection),
        "1",
        "hiding the caret must keep the selection, so typing lands at the start"
    );

    editor.widget().grab_focus();
    settle(
        "the caret to be drawn once the body has the keyboard",
        || eval(editor.widget(), caret) != "rgba(0, 0, 0, 0)",
    );

    address.grab_focus();
    settle(
        "the caret to be hidden again when the keyboard leaves",
        || eval(editor.widget(), caret) == "rgba(0, 0, 0, 0)",
    );
}
