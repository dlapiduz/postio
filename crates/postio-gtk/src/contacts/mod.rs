//! The Contacts screen (specs/005-contacts): a pane that takes over the
//! reading pane, listing people over a windowed model and showing one of them
//! in detail.
//!
//! The view layer speaks no SQL: rows arrive through [`ContactPageSource`] and
//! details through the app, which reads the store on the runtime. What a row
//! says and which command means what on which row is decided in
//! `postio_ui::contacts`.

pub mod detail;
pub mod editor;
pub mod join;
pub mod model;
pub mod pane;
pub mod row;

pub use detail::DetailView;
pub use editor::ContactEditor;
pub use join::JoinPanel;
pub use model::{ContactItem, ContactPageSource, ContactsModel};
pub use pane::{CONTACTS_OPEN_CLASS, ContactsPane};
pub use row::ContactRowView;

use crate::window::Window;

/// Builds the Contacts screen and mounts it in `window`'s reading pane,
/// hidden until it is opened.
pub fn install(window: &Window) -> ContactsPane {
    let pane = ContactsPane::new();
    pane.mount(window);
    pane
}
