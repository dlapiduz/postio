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
    /// The next match found is the last, not the first: a find carried
    /// into a message from the one after it (spec 013 US6).
    from_end: bool,
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
        {
            let mut find = self.imp().find.borrow_mut();
            find.query = query.to_owned();
            // A new query starts from where the person is reading.
            find.current = None;
        }
        self.refresh_find();
        self.scroll_to_current();
    }

    /// Look for `query` as [`set_find_query`](Self::set_find_query) does, the
    /// first match to be the last of the message when `from_end`: a find
    /// that walks backwards into a message lands on its last match.
    pub fn set_find_query_from(&self, query: &str, from_end: bool) {
        self.imp().find.borrow_mut().from_end = from_end;
        self.set_find_query(query);
    }

    /// The current match's place among the message's matches, and how many
    /// there are.
    pub fn find_state(&self) -> (Option<usize>, usize) {
        let find = self.imp().find.borrow();
        (find.current, find.matches.len())
    }

    /// Step to the next match (`forward`) or the previous, *without*
    /// wrapping: whether there was one. False at either end, which is where
    /// a search's window goes on to the next matched message.
    pub fn find_walk(&self, forward: bool) -> bool {
        {
            let mut find = self.imp().find.borrow_mut();
            let count = find.matches.len();
            let next = match (find.current, forward) {
                (_, _) if count == 0 => return false,
                (None, true) => 0,
                (None, false) => count - 1,
                (Some(at), true) if at + 1 < count => at + 1,
                (Some(at), false) if at > 0 => at - 1,
                _ => return false,
            };
            find.current = Some(next);
        }
        self.scroll_to_current();
        self.queue_draw();
        true
    }

    /// Recompute the matches from the query, over the snapshot on screen.
    pub(super) fn refresh_find(&self) {
        let matches = match self.document() {
            Some(doc) if !self.imp().find.borrow().query.is_empty() => {
                doc.text.find(&self.imp().find.borrow().query)
            }
            _ => Vec::new(),
        };
        // The first match from the top of what is in view, not from the
        // top of the message: finding keeps the reading position (T203).
        let from_here = self.document().and_then(|doc| {
            let (top, _) = self.window();
            matches.iter().position(|range| {
                doc.text
                    .rects(range.clone())
                    .first()
                    .is_some_and(|rect| rect.y0 >= top)
            })
        });
        let mut find = self.imp().find.borrow_mut();
        let from_end = find.from_end && !matches.is_empty();
        find.current = match find.current {
            Some(at) if at < matches.len() => Some(at),
            _ if from_end => Some(matches.len() - 1),
            _ if !matches.is_empty() => Some(from_here.unwrap_or(0)),
            _ => None,
        };
        if from_end {
            find.from_end = false;
        }
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
    ///
    /// The highlight is often set as a new document arrives, before the
    /// column has grown to it; the scroll then waits for the column to grow
    /// rather than stopping wherever the old document ended.
    pub fn set_highlight(&self, range: Option<std::ops::Range<usize>>) {
        self.mark(range);
        if !self.reveal_highlight() {
            self.reveal_when_laid_out();
        }
    }

    /// Highlight `range`, as [`set_highlight`](Self::set_highlight) does,
    /// without moving: for a redraw of a message whose sentence the person
    /// was already taken to, where the place is now theirs.
    pub fn mark(&self, range: Option<std::ops::Range<usize>>) {
        self.forget_reveal();
        self.imp().highlight.replace(range);
        self.queue_draw();
    }

    /// Bring the highlight into view, a third of the way down, as far as
    /// [`keep_in_view_on_reveal`](Self::keep_in_view_on_reveal) lets the
    /// column go. False when the scroll could not get there yet.
    fn reveal_highlight(&self) -> bool {
        let rects = self.highlight_rects();
        let Some(first) = rects.first() else {
            return true;
        };
        let y0 = rects.iter().map(|rect| rect.y0).fold(first.y0, f64::min);
        let y1 = rects.iter().map(|rect| rect.y1).fold(first.y1, f64::max);
        let kept = self
            .imp()
            .keep
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade());
        if let (Some(kept), Some(scroller)) = (kept, self.flow_scroller()) {
            // In the column's coordinates, where the header is.
            let (Some(view), Some(header)) = (
                self.compute_bounds(&scroller),
                kept.compute_bounds(&scroller),
            ) else {
                return false;
            };
            let adjustment = scroller.vadjustment();
            let value = adjustment.value();
            let start = value + f64::from(view.y());
            let limit = value + f64::from(header.y());
            return match reveal_to(
                (start + y0, start + y1),
                value,
                adjustment.page_size(),
                Some(limit),
            ) {
                None => true,
                Some(want) => {
                    adjustment.set_value(want);
                    (adjustment.value() - want).abs() < 0.5
                }
            };
        }
        let (top, page) = self.window();
        match reveal_to((y0, y1), top, page, None) {
            None => true,
            Some(want) => self.scroll_document_reaching(want),
        }
    }

    /// Never let a highlight's reveal scroll `widget` off the top of the
    /// column this view flows in: a message window's header -- its subject
    /// and who sent it -- stays in view as the window opens (#2125), and
    /// the sentence is revealed only as far as that allows.
    pub fn keep_in_view_on_reveal(&self, widget: &impl IsA<gtk::Widget>) {
        self.imp().keep.replace(Some(widget.as_ref().downgrade()));
    }

    /// Try [`reveal_highlight`](Self::reveal_highlight) again each time
    /// what scrolls the document changes its range -- the layout catching
    /// up with the document -- until it gets there.
    fn reveal_when_laid_out(&self) {
        let Some(adjustment) = self.scrolling() else {
            return;
        };
        let view = self.downgrade();
        let handler = adjustment.connect_changed(move |_| {
            if let Some(view) = view.upgrade()
                && view.reveal_highlight()
            {
                view.forget_reveal();
            }
        });
        self.imp().reveal.replace(Some((adjustment, handler)));
    }

    /// Stop waiting to bring a highlight into view.
    pub(super) fn forget_reveal(&self) {
        if let Some((adjustment, handler)) = self.imp().reveal.take() {
            adjustment.disconnect(handler);
        }
    }

    /// The range the highlight covers, if one is set.
    pub fn highlighted(&self) -> Option<std::ops::Range<usize>> {
        self.imp().highlight.borrow().clone()
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

/// Where a highlight's reveal takes the scroll, or `None` to leave it.
///
/// `sentence` is the highlight's top and bottom, `value` and `page` what is
/// in view, all in one coordinate space. The sentence comes a third of the
/// way down (FR-035). With `keep`, the furthest the scroll may go and keep
/// a header in view, it comes only if a third of the way down is within
/// that; otherwise nothing moves and the window stays at its top (#2125).
fn reveal_to(sentence: (f64, f64), value: f64, page: f64, keep: Option<f64>) -> Option<f64> {
    let (y0, y1) = sentence;
    if y0 >= value && y1 <= value + page {
        return None;
    }
    let third = (y0 - page / 3.0).max(0.0);
    match keep {
        Some(limit) if third > limit => None,
        _ => Some(third),
    }
}

#[cfg(test)]
mod reveal_tests {
    use super::reveal_to;

    // A 563px page over a column whose header ends at 579, the sentence
    // further down (#2125's fixture).
    const PAGE: f64 = 563.0;

    #[test]
    fn a_sentence_already_in_view_moves_nothing() {
        assert_eq!(reveal_to((100.0, 120.0), 0.0, PAGE, None), None);
        assert_eq!(reveal_to((100.0, 120.0), 0.0, PAGE, Some(0.0)), None);
    }

    #[test]
    fn with_nothing_to_keep_the_sentence_comes_a_third_of_the_way_down() {
        // FR-035 as it was, for a view with no header over it.
        assert_eq!(
            reveal_to((900.0, 920.0), 0.0, PAGE, None),
            Some(900.0 - PAGE / 3.0)
        );
    }

    #[test]
    fn the_reveal_never_scrolls_past_what_keeps_the_header_in_view() {
        // #2125: the header starts at 40; a third of the way down would
        // take the column to 712 and the header with it. Nor does it creep
        // as far as it may and stop short of the sentence: the column stays
        // at the top, and the marker card says what the sentence is.
        assert_eq!(reveal_to((900.0, 920.0), 0.0, PAGE, Some(40.0)), None);
    }

    #[test]
    fn a_sentence_just_under_the_fold_leaves_the_column_at_the_top_too() {
        // A third of the way down is past the header, so nothing moves --
        // not even the few pixels that would show the sentence's last line,
        // which take the space above the header with them and leave the
        // window opened somewhere other than its top.
        assert_eq!(reveal_to((PAGE, PAGE + 20.0), 0.0, PAGE, Some(40.0)), None);
    }

    #[test]
    fn where_a_third_of_the_way_down_keeps_the_header_it_is_used() {
        assert_eq!(
            reveal_to((PAGE + 100.0, PAGE + 120.0), 0.0, PAGE, Some(500.0)),
            Some(PAGE + 100.0 - PAGE / 3.0)
        );
    }
}
