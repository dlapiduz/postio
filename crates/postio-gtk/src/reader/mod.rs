//! The reading pane: a hardened `WebView` for message bodies.
//!
//! [`view::Reader`] is the module's one public entry point: JavaScript and
//! network access off, inline images resolved locally through a
//! [`scheme::BlobSource`], markup passed through [`sanitize::sanitize_body`]
//! and then [`quote::fold_html_quotes`], a click routed to the system
//! browser instead of ever navigating the pane.

pub mod quote;
pub mod sanitize;
pub mod scheme;
pub mod view;

pub use sanitize::RemoteImages;
pub use scheme::BlobSource;
pub use view::Reader;
