//! The Markdown source view (spec 014 US7): the draft as Markdown text, in a
//! plain `GtkTextView` that takes the rich editor's place in the column.
//!
//! What the text means is `postio_ui::editor::source`'s; this only draws it.
//! A carried quote is one line holding `U+E001` and the chip's words, tagged
//! so it cannot be edited, removed whole by Backspace or Delete as one undo
//! step, and never typed or pasted (FR-025).

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_body::Block;
use postio_ui::editor::markdown_edit::Edit;
use postio_ui::editor::source::{QUOTE_LINE, strip_quote_marks};

/// The tag a carried quote's line is drawn and guarded with.
const CHIP_TAG: &str = "postio-quote-chip";

/// The view, its strip and what its quote lines stand for.
pub struct SourceView {
    root: gtk::Box,
    strip: gtk::Label,
    view: gtk::TextView,
    quotes: RefCell<Vec<Block>>,
    /// Set while the view writes its own text, which may hold quote lines.
    writing: Rc<Cell<bool>>,
}

impl Default for SourceView {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceView {
    /// A hidden view, empty.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("postio-compose-source");
        root.set_vexpand(true);
        root.set_visible(false);

        let strip = gtk::Label::new(None);
        strip.add_css_class("postio-compose-source-strip");
        strip.add_css_class("dim-label");
        strip.set_xalign(0.0);
        strip.set_accessible_role(gtk::AccessibleRole::Status);
        root.append(&strip);

        let view = gtk::TextView::new();
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_vexpand(true);
        view.add_css_class("postio-compose-source-text");
        view.update_property(&[gtk::accessible::Property::Label("Message, as Markdown")]);
        let buffer = view.buffer();
        let chip = gtk::TextTag::new(Some(CHIP_TAG));
        chip.set_editable(false);
        chip.set_style(gtk::pango::Style::Italic);
        buffer.tag_table().add(&chip);

        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&scroller);

        let writing = Rc::new(Cell::new(false));
        // The quote line's mark cannot be typed or pasted (FR-025).
        buffer.connect_insert_text({
            let writing = Rc::clone(&writing);
            move |buffer, at, text| {
                if writing.get() || !text.contains(QUOTE_LINE) {
                    return;
                }
                glib::signal::signal_stop_emission_by_name(buffer, "insert-text");
                let clean = strip_quote_marks(text);
                writing.set(true);
                buffer.insert(at, &clean);
                writing.set(false);
            }
        });
        // Undo and redo put back what was there, quote lines included: the
        // mark is the view's own then, not something typed.
        for signal in ["undo", "redo"] {
            let writing = Rc::clone(&writing);
            buffer.connect_local(signal, false, move |_| {
                writing.set(true);
                let writing = Rc::clone(&writing);
                glib::idle_add_local_once(move || writing.set(false));
                None
            });
        }
        buffer.connect_changed(tag_chips);

        let this = SourceView {
            root,
            strip,
            view,
            quotes: RefCell::new(Vec::new()),
            writing,
        };
        this.install_chip_keys();
        this
    }

    /// The widget the composer's column holds.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The text view, for focus.
    pub fn text_view(&self) -> &gtk::TextView {
        &self.view
    }

    /// Show `text`, its quote lines standing for `quotes`, with the caret at
    /// the byte `caret`. A fresh view: what it held before cannot be undone
    /// into.
    pub fn show(&self, text: &str, quotes: Vec<Block>, caret: Option<usize>, strip: &str) {
        self.strip.set_label(strip);
        *self.quotes.borrow_mut() = quotes;
        let buffer = self.view.buffer();
        self.writing.set(true);
        buffer.begin_irreversible_action();
        buffer.set_text(text);
        buffer.end_irreversible_action();
        self.writing.set(false);
        tag_chips(&buffer);
        self.select(caret.unwrap_or(text.len())..caret.unwrap_or(text.len()));
        self.root.set_visible(true);
    }

    /// Hide the view.
    pub fn hide(&self) {
        self.root.set_visible(false);
    }

    /// Whether the view is shown.
    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

    /// The strip's words.
    pub fn strip(&self) -> String {
        self.strip.label().to_string()
    }

    /// The text as it stands.
    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    /// The carried quotes the quote lines stand for, in order.
    pub fn quotes(&self) -> Vec<Block> {
        self.quotes.borrow().clone()
    }

    /// The selection, as byte offsets into [`text`](Self::text).
    pub fn selection(&self) -> Range<usize> {
        let buffer = self.view.buffer();
        let text = self.text();
        let byte = |iter: &gtk::TextIter| {
            let chars = usize::try_from(iter.offset()).unwrap_or(0);
            text.char_indices()
                .nth(chars)
                .map_or(text.len(), |(at, _)| at)
        };
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let caret = buffer.iter_at_mark(&buffer.get_insert());
            (caret, caret)
        });
        byte(&start)..byte(&end)
    }

    /// Select the bytes `range` of the text.
    pub fn select(&self, range: Range<usize>) {
        let buffer = self.view.buffer();
        let text = self.text();
        let iter = |byte: usize| {
            let chars = text[..byte.min(text.len())].chars().count();
            buffer.iter_at_offset(i32::try_from(chars).unwrap_or(i32::MAX))
        };
        buffer.select_range(&iter(range.end), &iter(range.start));
        self.view.scroll_mark_onscreen(&buffer.get_insert());
    }

    /// Make `edit` the text, as one undo step, replacing only what changed.
    pub fn apply(&self, edit: Edit) {
        let before = self.text();
        let buffer = self.view.buffer();
        let head = before
            .bytes()
            .zip(edit.text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let head = floor_boundary(&before, floor_boundary(&edit.text, head));
        let tail = before[head..]
            .bytes()
            .rev()
            .zip(edit.text[head..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let mut tail = tail;
        while !before.is_char_boundary(before.len() - tail)
            || !edit.text.is_char_boundary(edit.text.len() - tail)
        {
            tail -= 1;
        }
        let iter = |byte: usize| {
            buffer.iter_at_offset(i32::try_from(before[..byte].chars().count()).unwrap_or(i32::MAX))
        };
        buffer.begin_user_action();
        self.writing.set(true);
        let (mut start, mut end) = (iter(head), iter(before.len() - tail));
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &edit.text[head..edit.text.len() - tail]);
        self.writing.set(false);
        buffer.end_user_action();
        self.select(edit.selection);
    }

    /// Undo the view's last edit.
    pub fn undo(&self) {
        self.view.buffer().undo();
    }

    /// Insert `text` at the caret as a paste does.
    pub fn paste(&self, text: &str) {
        let buffer = self.view.buffer();
        buffer.insert_interactive_at_cursor(text, true);
    }

    /// Replace the whole text as a person typing would, one undo step.
    pub fn set_text(&self, text: &str) {
        let buffer = self.view.buffer();
        buffer.begin_user_action();
        let (mut start, mut end) = buffer.bounds();
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, text);
        buffer.end_user_action();
    }

    /// Backspace or Delete beside or on a quote's line takes the line whole.
    fn install_chip_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view = self.view.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if remove_chip(&view.buffer(), key) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.view.add_controller(keys);
    }

    /// The key a person presses, for a test: what the view's own key
    /// controller does with it, or `false` when it leaves it to the view.
    #[doc(hidden)]
    pub fn test_key(&self, key: gtk::gdk::Key) -> bool {
        if remove_chip(&self.view.buffer(), key) {
            return true;
        }
        let buffer = self.view.buffer();
        match key {
            gtk::gdk::Key::BackSpace => {
                let mut caret = buffer.iter_at_mark(&buffer.get_insert());
                buffer.backspace(&mut caret, true, true)
            }
            _ => false,
        }
    }
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Remove the quote line the caret is on or beside, for `key`, as one undo
/// step. `false` when no quote line is in reach.
fn remove_chip(buffer: &gtk::TextBuffer, key: gtk::gdk::Key) -> bool {
    if buffer.has_selection() {
        return false;
    }
    let caret = buffer.iter_at_mark(&buffer.get_insert());
    let line_of = |iter: &gtk::TextIter| iter.line();
    let is_chip = |line: i32| {
        buffer
            .iter_at_line(line)
            .is_some_and(|start| start.char() == QUOTE_LINE)
    };
    let target = match key {
        gtk::gdk::Key::BackSpace => {
            if is_chip(line_of(&caret)) {
                Some(line_of(&caret))
            } else if caret.starts_line() && caret.line() > 0 && is_chip(caret.line() - 1) {
                Some(caret.line() - 1)
            } else {
                None
            }
        }
        gtk::gdk::Key::Delete | gtk::gdk::Key::KP_Delete => {
            if is_chip(line_of(&caret)) {
                Some(line_of(&caret))
            } else if caret.ends_line() && is_chip(caret.line() + 1) {
                Some(caret.line() + 1)
            } else {
                None
            }
        }
        _ => None,
    };
    let Some(line) = target else { return false };
    let Some(mut start) = buffer.iter_at_line(line) else {
        return false;
    };
    let mut end = start;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    // The line and one line end: the one after it, or before it at the end.
    if !end.forward_char() {
        start.backward_char();
    }
    buffer.begin_user_action();
    buffer.delete(&mut start, &mut end);
    buffer.end_user_action();
    true
}

/// Tag every quote line, so it cannot be edited.
fn tag_chips(buffer: &gtk::TextBuffer) {
    let (start, end) = buffer.bounds();
    buffer.remove_tag_by_name(CHIP_TAG, &start, &end);
    for line in 0..buffer.line_count() {
        let Some(from) = buffer.iter_at_line(line) else {
            continue;
        };
        if from.char() != QUOTE_LINE {
            continue;
        }
        let mut to = from;
        if !to.ends_line() {
            to.forward_to_line_end();
        }
        buffer.apply_tag_by_name(CHIP_TAG, &from, &to);
    }
}
