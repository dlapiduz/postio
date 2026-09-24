//! What every Postio frontend holds.
//!
//! A frontend never opens the store: exactly one process does, the host in
//! `postio-host` (ADR 0041). This crate is the frontend's side of that
//! arrangement -- commands go down, events come up, and reads are answered
//! from the host's local state. `specs/005-tui-frontend/contracts/protocol.md`
//! is the contract.
pub mod api;
pub mod counting;
pub mod protocol;

pub use api::{Client, Disconnected, SendError, Transport};
