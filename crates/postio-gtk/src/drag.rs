//! Dragging messages out of Focus's list as `.eml` files. Nothing is written until a drop somewhere
//! asks, so picking mail up and putting it back writes nothing.

use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Selection;
use postio_widgets::drag_out::LazyFiles;

use crate::window::FocusWindow;

impl FocusWindow {
    /// `None` where there is nothing to offer. Only a selection that can be named is offered:
    /// select-all is a predicate over the whole mailbox, and resolving it would write an `.eml` for
    /// every message in the folder, so a select-all drag offers nothing. A conversation's row
    /// stands for the message it shows.
    pub fn drag_offer(&self, position: u32) -> Option<gdk::ContentProvider> {
        let imp = self.imp();
        let pane = self.pane()?;
        let row = pane
            .feed()
            .shown()
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

    pub(crate) fn connect_drag_out(&self, pane: &crate::list::ListPane) {
        let window = self.downgrade();
        pane.connect_row_drag(move |position| window.upgrade()?.drag_offer(position));
    }
}
