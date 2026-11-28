//! The Contacts view: `g c` puts a people list, a card and a strip where the inbox was. What it
//! says and what a key does are the controller's; a click is reported as the input a key would have
//! been, so pointer and keyboard reach the same rule.

mod avatar;
mod card;
mod form;
mod merge;
pub mod popover;
mod view;

pub use form::Slot;
pub use view::ContactsView;
pub(crate) use view::texts_of;
