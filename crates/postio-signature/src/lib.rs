//! Message signatures checked on this computer: bytes and keys in, a verdict out. A pure leaf with
//! no store, toolkit, runtime or network, because everything it parses was chosen by a sender.
#![forbid(unsafe_code)]
pub mod scope;

pub use scope::{Format, SignedScope, signed_scope};
