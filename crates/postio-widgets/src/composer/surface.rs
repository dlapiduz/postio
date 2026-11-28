//! What the composer asks of its editing surface, so the WebView and the native text view are one
//! seam and the composer cannot reach past it.

use gtk::prelude::*;
use postio_body::markdown::DocPosition;
use postio_body::{ContentId, Document};

/// `bold` is true when the selection is inside `Strong`, not when a mode is armed: there are no
/// modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub heading_1: bool,
    pub bullet_list: bool,
    pub numbered_list: bool,
    pub quote_block: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Heading1,
    BulletList,
    NumberedList,
    QuoteBlock,
}

pub trait ComposerSurface {
    fn widget(&self) -> gtk::Widget;
    fn load(&self, document: Document);
    fn document(&self) -> Document;
    fn connect_changed(&self, handler: Box<dyn Fn(&Document)>);
    /// Called only on change, never per keystroke.
    fn connect_format_state(&self, handler: Box<dyn Fn(FormatState)>);
    fn undo(&self);
    fn redo(&self);
    fn can_undo(&self) -> bool;
    fn format(&self, format: Format);
    /// Anything but http, https and mailto is refused rather than dropped by a later parse.
    fn create_link(&self, href: &str) -> bool;
    fn insert_image(&self, content_id: &ContentId, alt: &str);
    fn place_caret_start(&self);
    fn place_caret_end(&self);
    fn flow_in(&self, probes: Vec<(&'static str, gtk::Widget)>);
    /// `None` when there is no caret, or it is in a carried quote.
    fn caret(&self, done: Box<dyn FnOnce(Option<DocPosition>)>);
    /// One edit on the record, the caret at `at`.
    fn show_at(&self, document: Document, at: Option<&DocPosition>);
    fn warm(&self);
    fn is_warm(&self) -> bool;
    fn test_type(&self, text: &str);
    fn test_caret(&self) -> Option<DocPosition>;
    fn test_select(&self, nth: u32, from: u32, to: u32);
    /// The WebView's script; a native surface runs none and answers empty.
    fn test_eval(&self, script: &str) -> String;
}

impl ComposerSurface for super::editor::Editor {
    fn widget(&self) -> gtk::Widget {
        super::editor::Editor::widget(self).clone().upcast()
    }
    fn load(&self, document: Document) {
        super::editor::Editor::load(self, document)
    }
    fn document(&self) -> Document {
        super::editor::Editor::document(self)
    }
    fn connect_changed(&self, handler: Box<dyn Fn(&Document)>) {
        super::editor::Editor::connect_changed(self, handler)
    }
    fn connect_format_state(&self, handler: Box<dyn Fn(FormatState)>) {
        super::editor::Editor::connect_format_state(self, handler)
    }
    fn undo(&self) {
        super::editor::Editor::undo(self)
    }
    fn redo(&self) {
        super::editor::Editor::redo(self)
    }
    fn can_undo(&self) -> bool {
        super::editor::Editor::can_undo(self)
    }
    fn format(&self, format: Format) {
        super::editor::Editor::format(self, format)
    }
    fn create_link(&self, href: &str) -> bool {
        super::editor::Editor::create_link(self, href)
    }
    fn insert_image(&self, content_id: &ContentId, alt: &str) {
        super::editor::Editor::insert_image(self, content_id, alt)
    }
    fn place_caret_start(&self) {
        super::editor::Editor::place_caret_start(self)
    }
    fn place_caret_end(&self) {
        super::editor::Editor::place_caret_end(self)
    }
    fn flow_in(&self, probes: Vec<(&'static str, gtk::Widget)>) {
        super::editor::Editor::flow_in(self, probes)
    }
    fn caret(&self, done: Box<dyn FnOnce(Option<DocPosition>)>) {
        super::editor::Editor::caret(self, done)
    }
    fn show_at(&self, document: Document, at: Option<&DocPosition>) {
        super::editor::Editor::show_at(self, document, at)
    }
    fn warm(&self) {
        super::editor::Editor::warm(self)
    }
    fn is_warm(&self) -> bool {
        super::editor::Editor::is_warm(self)
    }
    fn test_type(&self, text: &str) {
        super::editor::Editor::test_type(self, text)
    }
    fn test_caret(&self) -> Option<DocPosition> {
        super::editor::Editor::test_caret(self)
    }
    fn test_select(&self, nth: u32, from: u32, to: u32) {
        super::editor::Editor::test_select(self, nth, from, to)
    }
    fn test_eval(&self, script: &str) -> String {
        super::editor::Editor::test_eval(self, script)
    }
}
