//! The Contacts view (specs/011-focus-contacts S13, handoff screens 01 to
//! 03): `g c` puts a people list, a card and a strip where the inbox was.
//!
//! What it says and what a key does are the controller's (`postio_focus`)
//! and `postio_ui`'s -- the same rows, card and words the Mac draws. This
//! module is only how they look in GTK, and it reports a click as the input
//! a key would have been, so a pointer and a keyboard reach the same rule.

mod avatar;
mod card;
mod form;
mod list;
mod merge;
pub mod popover;
mod view;

pub use form::Slot;
pub use view::ContactsView;
pub(crate) use view::texts_of;
