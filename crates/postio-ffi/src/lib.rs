//! The boundary the macOS frontend talks to.
//!
//! Postio's engine is toolkit-free by construction — `postio-session` is the
//! composition root with no GTK in it, and `postio-core` is the
//! commands-down/events-up contract over it (ADR 0010, `ARCHITECTURE.md` §9).
//! This crate is what lets something that is not Rust hold the other end of
//! that contract: a UniFFI surface over `postio-session`, consumed by the
//! native Swift application (ADR 0019).
//!
//! # What this is not
//!
//! **It is private to the macOS app, and it promises no stability.** It exists
//! to serve one frontend and is shaped by that frontend's needs. It is not an
//! embedding API, and a caller outside this repository has no contract here.
//!
//! That sentence is load-bearing rather than defensive. Ghostty — the design
//! this seam is modelled on — says the same of `include/ghostty.h`, and its
//! genuinely public library ended up a separate artifact with separate
//! documentation. A frontend seam that accretes general-purpose obligations
//! stops being able to change, and this one has a lot of changing left to do.
//!
//! # Why it builds on Linux
//!
//! There is deliberately no macOS-specific code here, so `cargo test -p
//! postio-ffi` runs anywhere. That is the mechanism which keeps a Linux
//! session from breaking the macOS seam without noticing: the shim for the
//! expensive platform is compiled on the cheap one, in the ordinary gate.
//!
//! # The shape it is growing into
//!
//! Two tiers, after Ghostty's `apprt`. A small **required floor** — open a
//! session, drain events, invoke a command, page the list, render a reader
//! document — where a missing piece is a compile error. And a large
//! **optional surface** of one-way events, each ignorable, so a frontend that
//! handles none of them still runs. Today the floor is one function; the
//! scaffolding around it is the part that had to be proven first.

mod event;
mod keys;
mod list;
mod logging;
mod mailbox;
mod palette;
mod reader;
mod registry;
mod session;

pub use event::{ConnectionStateFfi, FailureReasonFfi, UiEvent};
pub use keys::{KeyOutcomeFfi, ModifiersFfi};
pub use list::{RowFfi, ScopeFfi};
pub use logging::start_logging;
pub use mailbox::{MailboxFfi, MailboxRoleFfi};
pub use palette::PaletteEntryFfi;
pub use reader::{InlinePart, RemoteImagesFfi};
pub use registry::{CommandSpecFfi, MenuFfi, MenuSectionFfi, UiContext, UiRecovery, menus};
pub use session::{Session, SessionError, SessionOptions};

/// Every command the registry knows, in cheat-sheet order.
///
/// A free function because the registry is a `const` table: it is not session
/// state, and requiring a session to read it would be an accident of where the
/// method happens to live.
///
/// That matters more on macOS than it looks. Opening a session reads the
/// store's key from the OS keyring (ADR 0014), and an unsigned build has a new
/// code identity on every rebuild — so anything that needed a session to show
/// a command list would raise a Keychain prompt to draw a menu. The frontend
/// builds its palette, cheat sheet and menu bar from this, before it has a
/// store or a single secret.
#[uniffi::export]
pub fn commands() -> Vec<CommandSpecFfi> {
    registry::commands()
}

uniffi::setup_scaffolding!();

/// Answers with the name of this application.
///
/// A deliberate placeholder: the boundary needs one real export before it has
/// any exports at all, and every guarantee this crate makes — that the
/// scaffolding compiles under the workspace's `forbid(unsafe_code)`, that
/// `uniffi-bindgen` can read the cdylib's metadata, that the generated Swift
/// declares what Rust exported — is proven against it. It is replaced by the
/// real floor, not extended.
#[uniffi::export]
pub fn probe() -> String {
    "postio".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_is_callable_as_ordinary_rust() {
        // The scaffolding must not change what the function is from Rust's
        // side: `#[uniffi::export]` adds a C-ABI shim beside it, and a version
        // that wrapped or moved the original would break every in-process
        // caller and every test in this workspace.
        assert_eq!(probe(), "postio");
    }
}
