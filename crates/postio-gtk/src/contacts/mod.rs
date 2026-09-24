//! The Contacts screen (specs/005-contacts): a pane that takes over the
//! reading pane, listing people over a windowed model and showing one of them
//! in detail.
//!
//! The view layer speaks no SQL: rows arrive through [`ContactPageSource`]
//! and details through the app, which reads the store on the runtime. What a
//! row says and which command means what on which row is decided in
//! `postio_ui::contacts`.

pub mod model;

pub use model::{ContactItem, ContactPageSource, ContactsModel};
