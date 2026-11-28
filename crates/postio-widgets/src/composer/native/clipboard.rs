//! Copy carries the document's own fragment, never the buffer's drawn bullets; paste and drop
//! arrive through `parse`, so what lands is what the subset can hold (FR-007).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use postio_body::{Block, Document, EditIntent, Inline, parse, slice};

use super::Inner;

const HTML: &str = "text/html";

pub(super) fn wire(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.view.connect_copy_clipboard(move |view| {
        view.stop_signal_emission_by_name("copy-clipboard");
        if let Some(inner) = weak.upgrade() {
            inner.copy();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.view.connect_cut_clipboard(move |view| {
        view.stop_signal_emission_by_name("cut-clipboard");
        if let Some(inner) = weak.upgrade()
            && inner.copy()
        {
            inner.dispatch(EditIntent::DeleteForward(postio_body::Unit::Char));
        }
    });
    let weak = Rc::downgrade(inner);
    inner.view.connect_paste_clipboard(move |view| {
        view.stop_signal_emission_by_name("paste-clipboard");
        if let Some(inner) = weak.upgrade() {
            paste(&inner);
        }
    });
}

impl Inner {
    fn copy(&self) -> bool {
        let selection = self.selection_now();
        if selection.is_collapsed() {
            return false;
        }
        let fragment = slice(&self.document.borrow(), &selection);
        let html = glib::Bytes::from_owned(fragment.to_html().into_bytes());
        let provider = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_bytes(HTML, &html),
            gdk::ContentProvider::for_value(&fragment.to_text().to_value()),
        ]);
        self.view.clipboard().set_content(Some(&provider)).is_ok()
    }
}

/// Asynchronous, like every clipboard read; the caret it lands at is the one when it arrives.
fn paste(inner: &Rc<Inner>) {
    let clipboard = inner.view.clipboard();
    let weak = Rc::downgrade(inner);
    if clipboard.formats().contain_mime_type(HTML) {
        clipboard.read_async(
            &[HTML],
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |read| {
                let (Some(inner), Ok((stream, _))) = (weak.upgrade(), read) else {
                    return;
                };
                let sink = gio::MemoryOutputStream::new_resizable();
                let kept = sink.clone();
                sink.splice_async(
                    &stream,
                    gio::OutputStreamSpliceFlags::CLOSE_SOURCE
                        | gio::OutputStreamSpliceFlags::CLOSE_TARGET,
                    glib::Priority::DEFAULT,
                    None::<&gio::Cancellable>,
                    move |done| {
                        if done.is_ok() {
                            let bytes = kept.steal_as_bytes();
                            let html = String::from_utf8_lossy(&bytes);
                            inner.dispatch(EditIntent::Paste(parse(&html)));
                        }
                    },
                );
            },
        );
        return;
    }
    clipboard.read_text_async(None::<&gio::Cancellable>, move |read| {
        if let (Some(inner), Ok(Some(text))) = (weak.upgrade(), read) {
            inner.dispatch(EditIntent::Paste(lines(&text)));
        }
    });
}

/// Each line a paragraph, as the WebView pasted plain text.
fn lines(text: &str) -> Document {
    Document {
        blocks: text
            .replace("\r\n", "\n")
            .split('\n')
            .map(|line| {
                Block::Paragraph(if line.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::Text(line.to_owned())]
                })
            })
            .collect(),
    }
}
