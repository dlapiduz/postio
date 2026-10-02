//! Dragging messages out of Focus's list (T245; `classic-parity.md` row 22).
//!
//! A row grabbed and pulled out of the window offers the message it is, or
//! the messages selected when it is one of them, to a file manager as `.eml`
//! files. The offer is `postio_widgets::drag_out::LazyFiles` over
//! `postio_widgets::present::export`, the classic list's own: nothing is
//! written until a drop somewhere asks, so picking mail up and putting it
//! back writes nothing.

use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Selection;
use postio_widgets::drag_out::LazyFiles;

use crate::window::FocusWindow;

impl FocusWindow {
    /// What a drag of the row at `position` offers a receiver: the selection
    /// when the row is in it, else that row alone. `None` where there is
    /// nothing to offer.
    ///
    /// Files are offered only for a selection that can be **named**. "Select
    /// all" is a predicate over the whole mailbox, and a predicate has no
    /// file form: resolving one would write an `.eml` for every message in a
    /// folder that may hold a hundred thousand. So a select-all drag offers
    /// nothing to the desktop, a drop that never highlights rather than one
    /// that fails after the fact.
    ///
    /// A conversation's row stands for the message it shows. Public because
    /// it is what the drag actually does, and a test that drove anything else
    /// would be testing a copy of it.
    pub fn drag_offer(&self, position: u32) -> Option<gdk::ContentProvider> {
        let imp = self.imp();
        let pane = self.pane()?;
        let row = pane
            .feed()
            .list()
            .item(position)
            .and_downcast::<crate::list::model::RowObject>()
            .and_then(|row| row.item())?;
        // A digest's row has its own verbs and no message of its own.
        row.as_conversation()?;
        let messages = match imp.picked.selection() {
            Selection::These(picked) if picked.contains(&row.id()) => picked,
            Selection::These(_) => vec![row.id()],
            Selection::Everything { except } if except.contains(&row.id()) => vec![row.id()],
            Selection::Everything { .. } => return None,
        };
        let client = imp.client.borrow().clone()?;
        let runtime = imp.runtime.borrow().clone()?;
        let materialise = postio_widgets::present::export::materialiser(
            runtime,
            client,
            postio_session::paths::export_dir,
        );
        Some(LazyFiles::for_messages(messages, materialise).upcast())
    }

    /// Let a row be dragged out of the list.
    pub(crate) fn connect_drag_out(&self, pane: &crate::list::ListPane) {
        let window = self.downgrade();
        pane.connect_row_drag(move |position| window.upgrade()?.drag_offer(position));
    }
}
