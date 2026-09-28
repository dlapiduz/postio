//! Find in the message on screen (spec 006 FR-018): a search bar over the
//! reading pane, matching over the snapshot's text index -- folded for case
//! and diacritics -- and drawn as overlays, so it survives a re-render
//! (FR-021f): the matches are recomputed from the query on every snapshot.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::BodyView;

/// Every match, faintly; the current one, clearly.
const MATCH: gdk::RGBA = gdk::RGBA::new(0.98, 0.80, 0.25, 0.35);
const CURRENT: gdk::RGBA = gdk::RGBA::new(0.98, 0.62, 0.10, 0.60);

/// What a find is looking for, and where.
#[derive(Default)]
pub(super) struct FindState {
    query: String,
    matches: Vec<std::ops::Range<usize>>,
    current: Option<usize>,
}

/// The find bar a reading pane shows above its `BodyView`.
pub struct FindBar {
    bar: gtk::SearchBar,
    entry: gtk::SearchEntry,
    view: glib::WeakRef<BodyView>,
}

impl FindBar {
    /// A bar that finds in `view`.
    pub fn new(view: &BodyView) -> FindBar {
        let entry = gtk::SearchEntry::new();
        let bar = gtk::SearchBar::builder().child(&entry).build();
        bar.connect_entry(&entry);
        let weak = view.downgrade();
        entry.connect_search_changed({
            let weak = weak.clone();
            move |entry| {
                if let Some(view) = weak.upgrade() {
                    view.set_find_query(&entry.text());
                }
            }
        });
        entry.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.find_step(true);
                }
            }
        });
        entry.connect_next_match({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.find_step(true);
                }
            }
        });
        entry.connect_previous_match({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.find_step(false);
                }
            }
        });
        entry.connect_stop_search({
            let bar = bar.downgrade();
            let weak = weak.clone();
            move |_| {
                if let Some(bar) = bar.upgrade() {
                    bar.set_search_mode(false);
                }
                if let Some(view) = weak.upgrade() {
                    view.set_find_query("");
                }
            }
        });
        FindBar {
            bar,
            entry,
            view: weak,
        }
    }

    /// The bar, to place above the view.
    pub fn widget(&self) -> &gtk::SearchBar {
        &self.bar
    }

    /// The query entry.
    pub fn entry(&self) -> &gtk::SearchEntry {
        &self.entry
    }

    /// Show the bar with the entry focused (`find_in_message`).
    pub fn open(&self) {
        self.bar.set_search_mode(true);
        self.entry.grab_focus();
        // The bar reveals with a transition; its entry takes focus once it
        // is shown.
        let entry = self.entry.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(entry) = entry.upgrade() {
                entry.grab_focus();
            }
        });
    }

    /// Hide the bar and clear the highlights.
    pub fn close(&self) {
        self.bar.set_search_mode(false);
        self.entry.set_text("");
        if let Some(view) = self.view.upgrade() {
            view.set_find_query("");
        }
    }
}

impl BodyView {
    /// Look for `query`; an empty one clears the find.
    pub fn set_find_query(&self, query: &str) {
        self.imp().find.borrow_mut().query = query.to_owned();
        self.refresh_find();
        self.scroll_to_current();
    }

    /// Recompute the matches from the query, over the snapshot on screen.
    pub(super) fn refresh_find(&self) {
        let matches = match self.document() {
            Some(doc) if !self.imp().find.borrow().query.is_empty() => {
                doc.text.find(&self.imp().find.borrow().query)
            }
            _ => Vec::new(),
        };
        let mut find = self.imp().find.borrow_mut();
        find.current = match find.current {
            Some(at) if at < matches.len() => Some(at),
            _ if !matches.is_empty() => Some(0),
            _ => None,
        };
        find.matches = matches;
        drop(find);
        self.queue_draw();
    }

    /// Every match's highlight rectangles, in document coordinates.
    pub fn find_rects(&self) -> Vec<postio_render::Rect> {
        let Some(doc) = self.document() else {
            return Vec::new();
        };
        let find = self.imp().find.borrow();
        find.matches
            .iter()
            .flat_map(|range| doc.text.rects(range.clone()))
            .collect()
    }

    /// The current match, as a range of the text index.
    pub fn current_match(&self) -> Option<std::ops::Range<usize>> {
        let find = self.imp().find.borrow();
        find.matches.get(find.current?).cloned()
    }

    /// Make the next match current (`forward`), or the previous, and bring
    /// it into view.
    pub fn find_step(&self, forward: bool) {
        {
            let mut find = self.imp().find.borrow_mut();
            let count = find.matches.len();
            if count == 0 {
                return;
            }
            find.current = Some(match (find.current, forward) {
                (None, _) => 0,
                (Some(at), true) => (at + 1) % count,
                (Some(at), false) => (at + count - 1) % count,
            });
        }
        self.scroll_to_current();
        self.queue_draw();
    }

    fn scroll_to_current(&self) {
        if let (Some(range), Some(doc)) = (self.current_match(), self.document())
            && let Some(rect) = doc.text.rects(range).first()
        {
            self.scroll_into_view(*rect);
        }
    }

    /// Draw the matches over the tiles.
    pub(super) fn draw_find(&self, snapshot: &gtk::Snapshot, left: f64, top: f64) {
        let Some(doc) = self.document() else { return };
        let find = self.imp().find.borrow();
        for (at, range) in find.matches.iter().enumerate() {
            let colour = if Some(at) == find.current {
                &CURRENT
            } else {
                &MATCH
            };
            for rect in doc.text.rects(range.clone()) {
                snapshot.append_color(
                    colour,
                    &gtk::graphene::Rect::new(
                        (rect.x0 - left) as f32,
                        (rect.y0 - top) as f32,
                        rect.width() as f32,
                        rect.height() as f32,
                    ),
                );
            }
        }
    }
}

impl BodyView {
    /// Highlight `range` of the text on screen -- the sentence a marker
    /// quotes (specs/007-postio-focus research R2) -- and bring it into view
    /// a third of the way down; `None` takes the highlight away. The range
    /// is the rendered text's (`TextIndex::locate` finds it), and belongs to
    /// the message on screen: showing another clears it.
    pub fn set_highlight(&self, range: Option<std::ops::Range<usize>>) {
        self.imp().highlight.replace(range);
        let rects = self.highlight_rects();
        if let (Some(first), Some(adjustment)) =
            (rects.first(), self.imp().vadjustment.borrow().clone())
        {
            let y0 = rects.iter().map(|rect| rect.y0).fold(first.y0, f64::min);
            let y1 = rects.iter().map(|rect| rect.y1).fold(first.y1, f64::max);
            let (top, page) = (adjustment.value(), adjustment.page_size());
            if y0 < top || y1 > top + page {
                adjustment.set_value((y0 - page / 3.0).max(0.0));
            }
        }
        self.queue_draw();
    }

    /// The highlight's rectangles, in document coordinates.
    pub fn highlight_rects(&self) -> Vec<postio_render::Rect> {
        match (self.imp().highlight.borrow().clone(), self.document()) {
            (Some(range), Some(document)) => document.text.rects(range),
            _ => Vec::new(),
        }
    }

    /// Draw the highlight under the find's matches: a faint ground in the
    /// accent and a line under it, the marker's colour (screen 04), in the
    /// scheme on screen.
    pub(super) fn draw_highlight(&self, snapshot: &gtk::Snapshot, left: f64, top: f64) {
        let rects = self.highlight_rects();
        if rects.is_empty() {
            return;
        }
        let manager = adw::StyleManager::default();
        let accent = manager.accent_color().to_standalone_rgba(manager.is_dark());
        let mut ground = accent;
        ground.set_alpha(0.14);
        for rect in rects {
            let (x, y) = ((rect.x0 - left) as f32, (rect.y0 - top) as f32);
            let (width, height) = (rect.width() as f32, rect.height() as f32);
            snapshot.append_color(&ground, &gtk::graphene::Rect::new(x, y, width, height));
            snapshot.append_color(
                &accent,
                &gtk::graphene::Rect::new(x, y + height - UNDERLINE, width, UNDERLINE),
            );
        }
    }
}

/// How thick the highlight's line is.
const UNDERLINE: f32 = 1.5;
