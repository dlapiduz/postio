//! The accessible text of the reading surface (spec 006 FR-020): the
//! snapshot's text index, the same reading order copy and find use, so the
//! screen reader, the clipboard and find cannot disagree (research R7).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{AccessibleTextGranularity, AccessibleTextRange, glib};

use super::{BodyView, imp};

/// `[start, end)` in characters, clamped to `len`.
fn span(start: u32, end: u32, len: usize) -> std::ops::Range<usize> {
    let start = (start as usize).min(len);
    let end = (end as usize).min(len).max(start);
    start..end
}

impl gtk::subclass::accessible_text::AccessibleTextImpl for imp::BodyView {
    fn contents(&self, start: u32, end: u32) -> Option<glib::Bytes> {
        let text = self.obj().accessible_contents(start, end)?;
        Some(glib::Bytes::from_owned(text.into_bytes()))
    }

    fn contents_at(
        &self,
        offset: u32,
        granularity: AccessibleTextGranularity,
    ) -> Option<(u32, u32, glib::Bytes)> {
        let document = self.obj().document()?;
        let text = &document.text;
        let len = text.text.chars().count();
        let at = (offset as usize).min(len);
        let range = match granularity {
            AccessibleTextGranularity::Character => at..(at + 1).min(len),
            AccessibleTextGranularity::Word => text.word_at(at),
            AccessibleTextGranularity::Line => text.line_at(at),
            // Sentences and paragraphs: the text index's own lines of
            // reading order, which end at every block.
            _ => {
                let chars: Vec<char> = text.text.chars().collect();
                let mut start = at;
                while start > 0 && chars[start - 1] != '\n' {
                    start -= 1;
                }
                let mut end = at;
                while end < len && chars[end] != '\n' {
                    end += 1;
                }
                start..end
            }
        };
        let slice = text.slice(range.clone()).to_owned();
        Some((
            range.start as u32,
            range.end as u32,
            glib::Bytes::from_owned(slice.into_bytes()),
        ))
    }

    fn caret_position(&self) -> u32 {
        self.obj().accessible_caret()
    }

    fn selection(&self) -> Vec<AccessibleTextRange> {
        self.obj()
            .accessible_selection()
            .into_iter()
            .map(|range| AccessibleTextRange::new(range.start, range.len()))
            .collect()
    }

    fn extents(&self, start: u32, end: u32) -> Option<gtk::graphene::Rect> {
        self.obj().accessible_extents(start, end)
    }

    fn offset(&self, point: &gtk::graphene::Point) -> Option<u32> {
        let view = self.obj();
        let document = view.document()?;
        document
            .text
            .hit(view.document_point(*point))
            .map(|offset| offset as u32)
    }
}

impl BodyView {
    /// The accessible text between two character offsets.
    #[doc(hidden)]
    pub fn accessible_contents(&self, start: u32, end: u32) -> Option<String> {
        let document = self.document()?;
        let len = document.text.text.chars().count();
        Some(document.text.slice(span(start, end, len)).to_owned())
    }

    /// The accessible selection, as ranges of the text.
    #[doc(hidden)]
    pub fn accessible_selection(&self) -> Vec<std::ops::Range<usize>> {
        self.selection().into_iter().collect()
    }

    /// The accessible caret: the selection's end, or the start.
    #[doc(hidden)]
    pub fn accessible_caret(&self) -> u32 {
        self.selection().map_or(0, |range| range.end as u32)
    }

    /// Where a range of the text is drawn, in the view's coordinates: the
    /// union of its clusters, less the scroll.
    #[doc(hidden)]
    pub fn accessible_extents(&self, start: u32, end: u32) -> Option<gtk::graphene::Rect> {
        let document = self.document()?;
        let len = document.text.text.chars().count();
        let union = document
            .text
            .rects(span(start, end, len))
            .into_iter()
            .reduce(|a, b| a.union(b))?;
        let origin = self.document_point(gtk::graphene::Point::new(0.0, 0.0));
        Some(gtk::graphene::Rect::new(
            (union.x0 - origin.x) as f32,
            (union.y0 - origin.y) as f32,
            union.width() as f32,
            union.height() as f32,
        ))
    }

    /// Tell assistive technology the whole text changed: a new snapshot.
    pub(super) fn announce_contents(&self, old_len: usize) {
        let len = self
            .document()
            .map_or(0, |doc| doc.text.text.chars().count());
        if old_len > 0 {
            self.update_contents(gtk::AccessibleTextContentChange::Remove, 0, old_len as u32);
        }
        if len > 0 {
            self.update_contents(gtk::AccessibleTextContentChange::Insert, 0, len as u32);
        }
        self.announce_selection();
    }

    /// Tell assistive technology the caret or selection moved.
    pub(super) fn announce_selection(&self) {
        self.update_caret_position();
        self.update_selection_bound();
    }
}
