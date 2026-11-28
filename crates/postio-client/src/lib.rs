//! The frontend's side of the host split (ADR 0041): commands go down, events come up, reads are
//! answered from the host's local state.
pub mod api;
pub mod counting;
pub mod protocol;

pub use api::{Client, Disconnected, SendError, Transport};
