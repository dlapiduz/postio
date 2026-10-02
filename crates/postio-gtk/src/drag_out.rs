//! Dragging mail out of Postio: the shared offer is
//! `postio_widgets::drag_out` (T245), re-exported here so the classic app's
//! paths still resolve. What stays here is the one thing the shared crate
//! cannot name: a message *part*, which is the classic parts panel's.

pub use postio_widgets::drag_out::*;

use std::rc::Rc;

use gtk::gio;

/// Turn one dragged message part into a file. The parts panel's export seam.
pub type MaterialisePart = Rc<
    dyn Fn(
        crate::parts::Node,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<gio::File>, String>> + 'static>,
    >,
>;

/// Offer the file for one message part, through the parts panel's seam.
pub fn lazy_part(node: crate::parts::Node, materialise: MaterialisePart) -> LazyFiles {
    LazyFiles::new(Rc::new(move || materialise(node.clone())))
}
