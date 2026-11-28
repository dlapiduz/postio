//! Commands in, events out. The registry is the one source for keys, palette, menus and
//! hints (PRODUCT §8); no GTK here (`check-crate-boundaries.py`).

pub mod action;
pub mod aim;
pub mod blocking;
pub mod bridge;
pub mod command;
pub mod config;
pub mod context;
pub mod dispatch;
pub mod event;
pub mod invocation;
pub mod menu;
pub mod perf_budget;
pub mod registry;
pub mod retention;
pub mod state;
pub mod test_support;
pub mod undo;

pub use action::ActionId;
pub use bridge::{Bridge, CommandHandler, CommandSender, EventSink, EventStream};
pub use command::{
    Command, CommandId, ContactAddressAction, ContactEditAction, ContactGroupNewAction,
    ContactJoinAction, ContactNewAction, KeptKey, KeptRule, MessageTarget, UnknownCommand,
};
pub use config::{ConfigChange, ConfigService, Keymap, SharedConfig};
pub use context::{Context, ContextSet, UnknownContext};
pub use dispatch::{CommandError, Dispatcher, Invocation};
pub use event::{ConnectionState, Event, FailureReason, MailFootprint, Sections};
pub use invocation::{EventEnvelope, InvocationId, InvocationOutcome};
pub use registry::{Availability, CommandSpec, Frontend, Recovery, Requirement, RequirementSet};
pub use state::{AppState, Resolved, Scope, Selection, SharedState, StateSnapshot, ViewMode};
pub use undo::{UndoEntry, UndoKind, UndoStack};
