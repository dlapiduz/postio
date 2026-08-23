//! Postio's UI-agnostic runtime: **commands in, events out**.
//!
//! The frontend sends a [`Command`] down and repaints from the [`Event`]s that
//! come back. It never reaches into storage, never speaks a protocol and never
//! awaits the network. That is what keeps a second frontend possible, and CI
//! enforces the half of it that can be enforced: this crate must not depend on
//! `gtk4` or `libadwaita` (`scripts/check-crate-boundaries.py`).
//!
//! # The registry is the source of truth
//!
//! spec.md §8 requires every command to have a keyboard shortcut, a
//! command-palette entry and an accessible action. Rather than three lists that
//! drift, there is one enumerable table — [`registry`] — and the keymap, the
//! `Ctrl+K` palette, the `?` cheat sheet, the right-click menu and the key
//! hints on the focused row are all generated from it. A command that is not in
//! the registry does not exist.
//!
//! ```
//! use postio_core::{Command, CommandId, Context, registry};
//!
//! // The palette, generated rather than hand-written:
//! let rows: Vec<(&str, &str)> = registry::for_context(Context::List)
//!     .map(|spec| (spec.title, spec.default_binding))
//!     .collect();
//! assert!(rows.contains(&("Archive", "a")));
//!
//! // Choosing a row yields the command to dispatch.
//! assert_eq!(
//!     Command::default_for(CommandId::Archive).id(),
//!     CommandId::Archive
//! );
//! ```
//!
//! # Commands go through the bridge
//!
//! [`bridge`] owns the tokio runtime and is the only place the asynchronous
//! backend touches the UI main loop: the frontend sends on a [`CommandSender`]
//! (never blocking) and drains an [`EventStream`] from its own loop, so no
//! backend work runs on the UI thread and no GTK type reaches this crate.
//!
//! # What lives elsewhere
//!
//! App state and the undo stack are separate beads in epic E6; this module is
//! the vocabulary they share.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bridge;
pub mod command;
pub mod context;
pub mod event;
pub mod registry;

pub use bridge::{Bridge, CommandHandler, CommandSender, EventSink, EventStream};
pub use command::{Command, CommandId, MessageTarget, UnknownCommand};
pub use context::{Context, ContextSet, UnknownContext};
pub use event::{ConnectionState, Event};
pub use registry::{CommandSpec, Recovery};
