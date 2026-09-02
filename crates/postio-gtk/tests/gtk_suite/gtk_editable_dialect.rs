//! The spike ADR 0003 demands before the rich composer is locked: can a
//! `contenteditable` WebView be constrained to emit markup
//! [`postio_body::parse`] absorbs into the canonical subset, losslessly for
//! everything the subset supports?
//!
//! This drives a real WebKit view — JavaScript **on**, which is the composer
//! profile ADR 0003 licenses and the reader never gets — through the editing
//! gestures a person actually performs (typing, Enter, bold, lists, links, a
//! hostile paste), reads back the DOM WebKit actually produced, and holds it
//! to the round-trip law from ADR 0004 Q3: `parse(to_html(d)) == d`, with
//! `to_html(parse(webkit_dialect))` as the normal form.
//!
//! What it pins, beyond "it works":
//!
//! * WebKit honours `defaultParagraphSeparator = 'p'`, so the editor script
//!   can force `<p>` paragraphs instead of the `<div>` dialect `parse`'s
//!   container-recursion would flatten.
//! * `styleWithCSS = false` keeps bold/italic as elements (`<b>`/`<i>`,
//!   which `parse` maps to `Strong`/`Emphasis`) rather than styled spans,
//!   which the subset would drop.
//! * A hostile insertion — script, remote image, styled markup — narrows to
//!   its text on the way into `Document`, through the DOM rather than the
//!   string path the existing corpus tests already cover.
//!
//! One test function: GTK is single-threaded and initialised once.

use crate::settle_until as settle;
use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Document, Inline, parse};
use webkit6::prelude::*;

/// Run `script` in the view and hand back its string result.
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

/// The editing gestures, expressed as the editor script will express them.
fn gesture(view: &webkit6::WebView, body: &str) -> String {
    eval(
        view,
        &format!(
            "(() => {{ \
               document.execCommand('defaultParagraphSeparator', false, 'p'); \
               document.execCommand('styleWithCSS', false, 'false'); \
               document.body.innerHTML = ''; \
               document.body.focus(); \
               const range = document.createRange(); \
               range.selectNodeContents(document.body); \
               range.collapse(false); \
               const selection = window.getSelection(); \
               selection.removeAllRanges(); \
               selection.addRange(range); \
               {body} \
               return document.body.innerHTML; \
             }})()"
        ),
    )
}

/// The law every captured dialect must obey: parsing it yields a document
/// whose serialisation re-parses to itself.
fn round_trips(dialect: &str) -> Document {
    let document = parse(dialect);
    let html = document.to_html();
    let again = parse(&html);
    assert_eq!(
        again, document,
        "the serialiser is not a normal form for what WebKit produced:\n\
         webkit: {dialect}\nnormal: {html}"
    );
    document
}

pub fn webkit_editing_gestures_stay_inside_the_canonical_subset() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    let window = gtk::Window::new();
    window.set_default_size(600, 400);

    // The composer profile ADR 0003 licenses: JavaScript on — it is Postio's
    // own script, not message content — network still off by construction
    // (nothing here registers a scheme or loads a remote origin).
    let view = webkit6::WebView::new();
    let settings = webkit6::prelude::WebViewExt::settings(&view).expect("settings");
    settings.set_enable_javascript(true);
    window.set_child(Some(&view));
    window.present();

    let loaded = Rc::new(RefCell::new(false));
    view.connect_load_changed({
        let loaded = loaded.clone();
        move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *loaded.borrow_mut() = true;
            }
        }
    });
    view.load_html("<body contenteditable=\"true\"><p>seed</p></body>", None);
    settle("the editing surface to load", || *loaded.borrow());

    // ── Enter makes <p> paragraphs, not the <div> dialect ─────────────────
    let dialect = gesture(
        &view,
        "document.execCommand('insertText', false, 'first line'); \
         document.execCommand('insertParagraph'); \
         document.execCommand('insertText', false, 'second line');",
    );
    let document = round_trips(&dialect);
    assert_eq!(
        document.blocks,
        vec![
            Block::Paragraph(vec![Inline::Text("first line".into())]),
            Block::Paragraph(vec![Inline::Text("second line".into())]),
        ],
        "webkit dialect was: {dialect}"
    );

    // ── bold and italic arrive as elements the subset knows ───────────────
    let dialect = gesture(
        &view,
        "document.body.innerHTML = '<p>plain strong after</p>'; \
         const text = document.body.firstChild.firstChild; \
         const sel = window.getSelection(); \
         sel.setBaseAndExtent(text, 6, text, 12); \
         document.execCommand('bold');",
    );
    let document = round_trips(&dialect);
    let Block::Paragraph(inlines) = &document.blocks[0] else {
        panic!("bold gesture did not stay a paragraph: {dialect}");
    };
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline, Inline::Strong(_))),
        "no Strong out of execCommand bold; webkit dialect was: {dialect}"
    );

    // ── lists arrive as lists ─────────────────────────────────────────────
    let dialect = gesture(
        &view,
        "document.body.innerHTML = '<p>one</p>'; \
         const caret = document.createRange(); \
         caret.selectNodeContents(document.body.firstChild); \
         caret.collapse(false); \
         const sel = window.getSelection(); \
         sel.removeAllRanges(); \
         sel.addRange(caret); \
         document.execCommand('insertUnorderedList'); \
         document.execCommand('insertParagraph'); \
         document.execCommand('insertText', false, 'two');",
    );
    let document = round_trips(&dialect);
    assert!(
        document.blocks.iter().any(
            |block| matches!(block, Block::List { ordered: false, items } if !items.is_empty())
        ),
        "no List out of insertUnorderedList; webkit dialect was: {dialect}"
    );

    // ── links keep their href, on the allowed schemes ─────────────────────
    let dialect = gesture(
        &view,
        "document.body.innerHTML = '<p>read this</p>'; \
         const text = document.body.firstChild.firstChild; \
         const sel = window.getSelection(); \
         sel.setBaseAndExtent(text, 5, text, 9); \
         document.execCommand('createLink', false, 'https://example.com/doc');",
    );
    let document = round_trips(&dialect);
    let Block::Paragraph(inlines) = &document.blocks[0] else {
        panic!("link gesture did not stay a paragraph: {dialect}");
    };
    assert!(
        inlines.iter().any(|inline| matches!(
            inline,
            Inline::Link { href, .. } if href.as_str() == "https://example.com/doc"
        )),
        "no Link out of createLink; webkit dialect was: {dialect}"
    );

    // ── a hostile insertion narrows to its words ──────────────────────────
    let dialect = gesture(
        &view,
        "document.execCommand('insertHTML', false, \
           '<script>alert(1)<\\u002fscript>\
            <img src=\"https://tracker.example.net/p.gif\">\
            <p style=\"color:red\" onclick=\"x()\">hello</p>');",
    );
    let document = round_trips(&dialect);
    let html = document.to_html();
    assert!(
        !html.contains("script") && !html.contains("tracker.example") && !html.contains("style"),
        "hostile paste survived into the document: {html}"
    );
    assert!(
        document.blocks.iter().any(|block| matches!(
            block,
            Block::Paragraph(inlines) if inlines.iter().any(
                |inline| matches!(inline, Inline::Text(text) if text.contains("hello"))
            )
        )),
        "the hostile paste's legitimate words were lost: {dialect}"
    );
}
