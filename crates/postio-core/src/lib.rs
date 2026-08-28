//! Postio's UI-agnostic runtime: **commands in, events out**.
//!
//! The frontend sends a [`Command`] down and repaints from the [`Event`]s that
//! come back. It never reaches into storage, never speaks a protocol and never
//! awaits the network. That is what keeps a second frontend possible, and CI
//! enforces the half of it that can be enforced: this crate must not depend on
//! `gtk4` or `libadwaita` (`scripts/checks/check-crate-boundaries.py`).
//!
//! # The registry is the source of truth
//!
//! docs/PRODUCT.md §8 requires every command to have a keyboard shortcut, a
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
//! # One path from intent to change
//!
//! [`dispatch`] routes every command — from a key, the palette, a menu or the
//! mouse — to the single handler registered for it, and turns every failure
//! into an [`Event`] rather than a panic.
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
//! App state and the undo stack are separate issues in epic E6; this module is
//! the vocabulary they share.

pub mod action;
pub mod aim;
pub mod bridge;
pub mod command;
pub mod config;
pub mod context;
pub mod dispatch;
pub mod event;
pub mod invocation;
pub mod perf_budget;
pub mod registry;
pub mod state;
pub mod undo;

pub use action::{ActionId, ExtId};
pub use bridge::{Bridge, CommandHandler, CommandSender, EventSink, EventStream};
pub use command::{Command, CommandId, MessageTarget, UnknownCommand};
pub use config::{ConfigChange, ConfigService, Keymap, SharedConfig};
pub use context::{Context, ContextSet, UnknownContext};
pub use dispatch::{CommandError, Dispatcher, Invocation};
pub use event::{ConnectionState, Event, FailureReason, MailFootprint};
pub use invocation::{EventEnvelope, InvocationId, InvocationOutcome};
pub use registry::{Availability, CommandSpec, Recovery, Requirement};
pub use state::{AppState, Resolved, Scope, Selection, SharedState, ViewMode};
pub use undo::{UndoEntry, UndoKind, UndoStack};
