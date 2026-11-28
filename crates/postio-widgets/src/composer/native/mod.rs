//! The composer's native surface: a `GtkTextView` over `postio_body::project`, never the record.
//! Nothing writes its buffer but the patch of `apply`'s result (ADR 0039).

mod a11y;
mod clipboard;
mod history;
mod image;
mod input;
mod patch;
mod preedit;
mod quoted;
mod render;
mod selection;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use postio_body::markdown::DocPosition as SourcePosition;
use postio_body::{
    ContentId, Document, EditHistory, EditIntent, EditStep, Projection, Selection, project,
};

use super::surface::{ComposerSurface, Format, FormatState};
use crate::reader::BlobSource;

pub use patch::PatchStats;

const COALESCE: Duration = Duration::from_millis(700);

type ChangedHandler = Box<dyn Fn(&Document)>;

type FormatWatcher = Box<dyn Fn(FormatState)>;

pub struct NativeSurface {
    inner: Rc<Inner>,
}

pub(crate) struct Inner {
    view: gtk::TextView,
    buffer: gtk::TextBuffer,
    document: RefCell<Document>,
    projection: RefCell<Projection>,
    selection: RefCell<Selection>,
    history: RefCell<EditHistory>,
    last_edit: Cell<Option<Instant>>,
    coalesce: Duration,
    patching: Cell<bool>,
    stats: Cell<PatchStats>,
    changed: RefCell<Vec<ChangedHandler>>,
    format: Cell<FormatState>,
    format_watchers: RefCell<Vec<FormatWatcher>>,
    source: Rc<dyn BlobSource>,
    pending: RefCell<Vec<input::Pending>>,
    quote_folded: Cell<bool>,
}

impl NativeSurface {
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::with_coalesce(source, COALESCE)
    }

    pub fn with_coalesce(source: Rc<dyn BlobSource>, coalesce: Duration) -> Self {
        let buffer = gtk::TextBuffer::new(Some(&render::tags()));
        // The document's history is the only undo (ADR 0004 Q5): the buffer's would undo a patch.
        buffer.set_enable_undo(false);
        let view = gtk::TextView::with_buffer(&buffer);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_hexpand(true);
        view.set_vexpand(true);
        view.add_css_class("postio-editor-view");
        a11y::name_view(&view);
        let inner = Rc::new(Inner {
            view,
            buffer,
            document: RefCell::new(Document::new()),
            projection: RefCell::new(Projection::default()),
            selection: RefCell::new(Selection::default()),
            history: RefCell::new(EditHistory::new()),
            last_edit: Cell::new(None),
            coalesce,
            patching: Cell::new(false),
            stats: Cell::new(PatchStats::default()),
            changed: RefCell::new(Vec::new()),
            format: Cell::new(FormatState::default()),
            format_watchers: RefCell::new(Vec::new()),
            source,
            pending: RefCell::new(Vec::new()),
            quote_folded: Cell::new(true),
        });
        input::wire(&inner);
        selection::wire(&inner);
        clipboard::wire(&inner);
        quoted::wire(&inner);
        NativeSurface { inner }
    }

    pub fn view(&self) -> &gtk::TextView {
        &self.inner.view
    }

    pub fn last_patch(&self) -> PatchStats {
        self.inner.stats.get()
    }

    pub fn selection(&self) -> Selection {
        self.inner.selection_now()
    }

    pub fn dispatch(&self, intent: EditIntent) -> bool {
        self.inner.dispatch(intent)
    }
}

impl Inner {
    fn selection_now(&self) -> Selection {
        let projection = self.projection.borrow();
        let offset = |mark: &gtk::TextMark| self.buffer.iter_at_mark(mark).offset() as u32;
        let head = projection.position_of(offset(&self.buffer.get_insert()));
        let anchor = projection.position_of(offset(&self.buffer.selection_bound()));
        let held = self.selection.borrow();
        Selection {
            marks: held.marks.clone().filter(|_| held.head == head),
            preedit: held.preedit.clone(),
            ..Selection::range(anchor, head)
        }
    }

    fn dispatch(&self, intent: EditIntent) -> bool {
        let before = self.selection_now();
        self.dispatch_at(before, intent)
    }

    fn dispatch_at(&self, before: Selection, intent: EditIntent) -> bool {
        let applied = self.document.borrow_mut().edit(&before, intent);
        let changed = !applied.changed.is_empty();
        if changed {
            self.record(&applied, before.clone());
            self.repaint(applied.changed);
        }
        self.place(&applied.selection);
        *self.selection.borrow_mut() = applied.selection.clone();
        if changed {
            self.notify();
        } else {
            self.reflect();
        }
        changed || applied.selection != before
    }

    fn repaint(&self, changed: postio_body::BlockRange) {
        let patch = self
            .projection
            .borrow_mut()
            .patch(changed, &self.document.borrow());
        let stats = patch::apply(self, &patch);
        self.stats.set(PatchStats {
            blocks: changed.new_end - changed.start,
            ..stats
        });
    }

    fn place(&self, selection: &Selection) {
        let projection = self.projection.borrow();
        let head = self
            .buffer
            .iter_at_offset(projection.offset_of(&selection.head) as i32);
        let anchor = self
            .buffer
            .iter_at_offset(projection.offset_of(&selection.anchor) as i32);
        self.patching.set(true);
        self.buffer.select_range(&head, &anchor);
        self.patching.set(false);
    }

    fn notify(&self) {
        {
            let document = self.document.borrow();
            for handler in self.changed.borrow().iter() {
                handler(&document);
            }
        }
        self.reflect();
    }

    pub(super) fn reflect(&self) {
        let state =
            postio_ui::editor::format::format_state(&self.document.borrow(), &self.selection_now());
        if self.format.replace(state) != state {
            for watcher in self.format_watchers.borrow().iter() {
                watcher(state);
            }
        }
    }

    fn show(&self, document: Document, at: Selection) {
        let old = self.projection.borrow().len();
        *self.document.borrow_mut() = document;
        *self.projection.borrow_mut() = project(&self.document.borrow(), None);
        let patch = postio_body::Patch {
            delete: 0..old,
            insert: self.projection.borrow().text(),
            restyle: 0..self.projection.borrow().len(),
        };
        self.stats.set(patch::apply(self, &patch));
        self.place(&at);
        *self.selection.borrow_mut() = at;
    }
}

fn start_of(document: &Document) -> Selection {
    project(document, None)
        .lines()
        .next()
        .map(|line| Selection::caret(postio_body::DocPosition::new(line.path, 0)))
        .unwrap_or_default()
}

fn editable(mut document: Document) -> Document {
    fn unbreak(blocks: &mut [postio_body::Block]) {
        for block in blocks {
            match block {
                postio_body::Block::Paragraph(inlines)
                    if *inlines == [postio_body::Inline::Break] =>
                {
                    inlines.clear()
                }
                postio_body::Block::Quote(children) => unbreak(children),
                postio_body::Block::List { items, .. } => {
                    for item in items {
                        unbreak(&mut item.blocks);
                    }
                }
                _ => {}
            }
        }
    }
    unbreak(&mut document.blocks);
    let leaf = |block: &postio_body::Block| {
        matches!(
            block,
            postio_body::Block::Paragraph(_)
                | postio_body::Block::Heading { .. }
                | postio_body::Block::List { .. }
                | postio_body::Block::Quote(_)
                | postio_body::Block::Code { .. }
        )
    };
    if !document.blocks.first().is_some_and(leaf) {
        document
            .blocks
            .insert(0, postio_body::Block::Paragraph(Vec::new()));
    }
    document
}

impl ComposerSurface for NativeSurface {
    fn widget(&self) -> gtk::Widget {
        self.inner.view.clone().upcast()
    }

    fn load(&self, document: Document) {
        let document = editable(document);
        let at = start_of(&document);
        self.inner.history.borrow_mut().clear();
        self.inner.last_edit.set(None);
        self.inner.show(document, at);
    }

    fn document(&self) -> Document {
        self.inner.document.borrow().clone()
    }

    fn connect_changed(&self, handler: Box<dyn Fn(&Document)>) {
        self.inner.changed.borrow_mut().push(handler);
    }

    fn connect_format_state(&self, handler: Box<dyn Fn(FormatState)>) {
        self.inner.format_watchers.borrow_mut().push(handler);
    }

    fn undo(&self) {
        self.inner.step(true);
    }

    fn redo(&self) {
        self.inner.step(false);
    }

    fn can_undo(&self) -> bool {
        self.inner.history.borrow().can_undo()
    }

    fn format(&self, format: Format) {
        use postio_body::{BlockKind, HeadingLevel, Mark};
        self.inner.dispatch(match format {
            Format::Bold => EditIntent::ToggleMark(Mark::Strong),
            Format::Italic => EditIntent::ToggleMark(Mark::Emphasis),
            Format::Heading1 => EditIntent::ToggleBlock(BlockKind::Heading(HeadingLevel::One)),
            Format::BulletList => EditIntent::ToggleBlock(BlockKind::BulletList),
            Format::NumberedList => EditIntent::ToggleBlock(BlockKind::NumberedList),
            Format::QuoteBlock => EditIntent::ToggleBlock(BlockKind::Quote),
        });
    }

    fn create_link(&self, href: &str) -> bool {
        let allowed = ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| href.starts_with(scheme));
        let Some(href) = postio_body::Href::parse(href).filter(|_| allowed) else {
            return false;
        };
        self.inner.dispatch(EditIntent::SetLink(Some(href)));
        true
    }

    fn insert_image(&self, content_id: &ContentId, alt: &str) {
        self.inner.dispatch(EditIntent::InsertImage {
            content_id: content_id.clone(),
            alt: alt.to_owned(),
        });
    }

    fn place_caret_start(&self) {
        let at = start_of(&self.inner.document.borrow());
        self.inner.place(&at);
    }

    fn place_caret_end(&self) {
        let end = self.inner.buffer.end_iter();
        self.inner.buffer.place_cursor(&end);
    }

    fn flow_in(&self, _probes: Vec<(&'static str, gtk::Widget)>) {}

    fn caret(&self, done: Box<dyn FnOnce(Option<SourcePosition>)>) {
        done(self.test_caret());
    }

    fn show_at(&self, document: Document, at: Option<&SourcePosition>) {
        let before = self.inner.document.borrow().clone();
        let at = at
            .map(|at| {
                Selection::caret(postio_body::DocPosition::new(
                    at.path.iter().map(|&i| i as u32).collect::<Vec<_>>(),
                    at.offset as u32,
                ))
            })
            .filter(|at| at.is_valid_in(&document))
            .unwrap_or_else(|| start_of(&document));
        self.inner.history.borrow_mut().record(
            EditStep::between(&before, &document, self.inner.selection_now(), at.clone()),
            postio_body::Join::New,
        );
        self.inner.show(document, at);
        self.inner.notify();
    }

    fn warm(&self) {}

    fn is_warm(&self) -> bool {
        true
    }

    fn test_type(&self, text: &str) {
        let buffer = &self.inner.buffer;
        buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
        buffer.delete_selection(true, true);
        buffer.insert_interactive_at_cursor(text, true);
        self.inner.flush();
    }

    fn test_caret(&self) -> Option<SourcePosition> {
        let head = self.inner.selection_now().head;
        Some(SourcePosition {
            path: head.path.iter().map(|&i| i as usize).collect(),
            offset: head.offset as usize,
        })
    }

    fn test_select(&self, nth: u32, from: u32, to: u32) {
        let start = self
            .inner
            .projection
            .borrow()
            .runs()
            .filter(|(_, run)| matches!(run, postio_body::Run::Text { .. }))
            .nth(nth as usize)
            .map(|(at, _)| at);
        if let Some(start) = start {
            let buffer = &self.inner.buffer;
            buffer.select_range(
                &buffer.iter_at_offset((start + to) as i32),
                &buffer.iter_at_offset((start + from) as i32),
            );
        }
    }

    fn test_eval(&self, _script: &str) -> String {
        String::new()
    }
}
