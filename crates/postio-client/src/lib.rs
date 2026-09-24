//! What every Postio frontend holds.
//!
//! One app at a time opens the store -- the desktop app or the terminal --
//! and starts its host (`postio-host`) in its own process (ADR 0041). This
//! crate is the frontend's side of that: commands go down, events come up,
//! and reads are answered from the host's local state.
//! `specs/005-tui-frontend/contracts/protocol.md` is the contract.
pub mod api;
pub mod counting;
pub mod protocol;

pub use api::{Client, Disconnected, SendError, Transport};
