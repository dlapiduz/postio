use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::{Block, Document, Inline};
use postio_widgets::composer::surface::ComposerSurface;

use super::composer_native_input::shown;
use super::storyboard_support::until;

fn p(s: &str) -> Block {
    Block::Paragraph(if s.is_empty() {
        vec![]
    } else {
        vec![Inline::Text(s.into())]
    })
}

fn offer_html(view: &gtk::TextView, html: &str) {
    let bytes = glib::Bytes::from_owned(html.as_bytes().to_vec());
    let provider = gdk::ContentProvider::for_bytes("text/html", &bytes);
    view.clipboard().set_content(Some(&provider)).unwrap();
}

pub fn hostile_html_pastes_narrowed() {
    let shown = shown(Document {
        blocks: vec![p("")],
    });
    let view = shown.surface.view();
    offer_html(
        view,
        "<p style=\"color:red\" onclick=\"steal()\">hi<script>steal()</script>\
         <img src=\"https://tracker.example.com/p.gif\"> <a href=\"javascript:x()\">there</a></p>",
    );
    view.emit_paste_clipboard();
    assert!(until(|| shown.surface.document()
        != Document {
            blocks: vec![p("")]
        }));
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("hi there")]
        }
    );
    shown.assert_in_step();
}

pub fn plain_text_pastes_a_paragraph_per_line() {
    let shown = shown(Document {
        blocks: vec![p("")],
    });
    let view = shown.surface.view();
    view.clipboard().set_text("one\ntwo");
    view.emit_paste_clipboard();
    assert!(until(|| shown.surface.document().blocks.len() == 2));
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("one"), p("two")]
        }
    );
}

pub fn cut_then_paste_gives_the_marks_back() {
    let bold = Block::Paragraph(vec![
        Inline::Text("a ".into()),
        Inline::Strong(vec![Inline::Text("bold".into())]),
        Inline::Text(" word".into()),
    ]);
    let shown = shown(Document {
        blocks: vec![bold.clone()],
    });
    let view = shown.surface.view();
    let buffer = view.buffer();
    buffer.select_range(&buffer.iter_at_offset(2), &buffer.iter_at_offset(6));
    view.emit_cut_clipboard();
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("a  word")]
        }
    );
    view.emit_paste_clipboard();
    assert!(until(|| shown.surface.document()
        == Document {
            blocks: vec![bold.clone()]
        }));
    shown.assert_in_step();
}

pub fn a_copy_carries_no_drawn_bullet() {
    let shown = shown(Document {
        blocks: vec![Block::List {
            kind: postio_body::ListKind::Bullet,
            items: vec![postio_body::ListItem::new(vec![p("milk")])],
        }],
    });
    let view = shown.surface.view();
    let buffer = view.buffer();
    buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
    view.emit_copy_clipboard();
    let read = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = read.clone();
    view.clipboard()
        .read_text_async(None::<&gtk::gio::Cancellable>, move |text| {
            *slot.borrow_mut() = Some(text.ok().flatten().map(|t| t.to_string()));
        });
    assert!(until(|| read.borrow().is_some()));
    let text = read.borrow().clone().flatten().unwrap_or_default();
    assert!(
        !text.contains('\u{2022}'),
        "the bullet is drawn, not copied: {text:?}"
    );
    assert!(text.contains("milk"));
}
