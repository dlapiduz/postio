//! The command bus: one dispatch path, whatever the user touched.
//!
//! A keystroke, a `Ctrl+K` palette row, a context-menu item and a click on a
//! toolbar button all resolve — through the [`registry`](crate::registry) — to
//! the same [`Command`], and the bus routes it to the one handler registered
//! for its [`CommandId`]. Nothing else is allowed to mutate anything, which is
//! why the mouse and the keyboard cannot drift apart and why a test can drive a
//! whole workflow with no UI attached.
//!
//! # Handlers are local-first
//!
//! A handler does the local work and returns: write SQLite, enqueue the remote
//! operation, emit the event. It does not await the network — the sync engine
//! picks the operation up and reports back through its own events later. The
//! bus awaits each handler before starting the next command, so app state and
//! the undo stack see a total order without taking a lock across `.await`.
//!
//! # Failure is an event, not a panic
//!
//! Three things can go wrong, and all three come back to the UI as events:
//!
//! * the command cannot run right now (no selection, nothing to undo) —
//!   [`CommandError::rejected`], which becomes [`Event::CommandRejected`] and
//!   usually a quiet hint rather than a dialog;
//! * the command failed (the disk is full) — [`CommandError::failed`], which
//!   becomes [`Event::Error`];
//! * the handler panicked, which is a bug — reported as [`Event::Error`] while
//!   the bus stays up. One broken handler costs the user one action, not the
//!   application.
//!
//! ```
//! use postio_core::bridge::Bridge;
//! use postio_core::dispatch::{CommandError, Dispatcher};
//! use postio_core::{ActionId, Command, CommandId, Event};
//!
//! let dispatcher = Dispatcher::builder()
//!     .on(CommandId::Undo, |_invocation| async move {
//!         Err(CommandError::rejected("nothing to undo"))
//!     })
//!     .build();
//!
//! let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
//! bridge.commands().send(Command::Undo).expect("running");
//! bridge.shutdown();
//!
//! assert!(matches!(
//!     events.try_next(),
//!     Some(Event::CommandRejected { command: ActionId::Builtin(CommandId::Undo), .. })
//! ));
//! ```

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::bridge::{CommandHandler, EventSink, HandlerFuture};
use crate::{ActionId, Command, CommandId, Event, ExtId, registry};

/// The future a command handler returns.
pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<(), CommandError>> + Send + 'static>>;

/// Why a command did not do what it said.
///
/// The distinction is what the user sees: a rejection is ordinary — the
/// command was not applicable — while a failure is something that went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The command was not applicable: nothing selected, nothing to undo,
    /// no account configured. Answered with a quiet hint.
    Rejected(String),
    /// The command was applicable and failed anyway. Answered visibly.
    Failed(String),
}

impl CommandError {
    /// The command could not run. `reason` is user-facing prose.
    pub fn rejected(reason: impl Into<String>) -> Self {
        CommandError::Rejected(reason.into())
    }

    /// The command failed. `message` is user-facing prose and must never
    /// contain a secret — it goes on screen.
    pub fn failed(message: impl Into<String>) -> Self {
        CommandError::Failed(message.into())
    }

    /// The user-facing prose, whichever kind this is.
    pub fn message(&self) -> &str {
        match self {
            CommandError::Rejected(text) | CommandError::Failed(text) => text,
        }
    }

    /// How this failure reaches the UI.
    ///
    /// Takes anything that names an action, so a built-in and a registered
    /// extension are refused through the same event and answered by the same
    /// quiet hint.
    fn into_event(self, command: impl Into<ActionId>) -> Event {
        match self {
            CommandError::Rejected(reason) => Event::CommandRejected {
                command: command.into(),
                reason,
            },
            CommandError::Failed(message) => Event::Error { message },
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CommandError {}

/// One command, handed to its handler.
///
/// Carries the invocation itself and the way to answer: handlers emit the
/// events the UI repaints from, and return `Ok(())` or a [`CommandError`].
#[derive(Debug)]
pub struct Invocation {
    /// What was asked for, payload and all.
    pub command: Command,
    events: EventSink,
}

impl Invocation {
    /// The registry id being handled.
    pub fn id(&self) -> CommandId {
        self.command.id()
    }

    /// Tell the UI something changed. Never blocks; `false` means the frontend
    /// has already gone away.
    pub fn emit(&self, event: Event) -> bool {
        self.events.emit(event)
    }

    /// A sink a spawned background task can keep reporting through after this
    /// handler has returned — a body fetch, a send, a resync.
    pub fn events(&self) -> EventSink {
        self.events.clone()
    }
}

/// One extension command, handed to its handler.
///
/// Deliberately not an [`Invocation`]: that carries a [`Command`], which is
/// the closed vocabulary with a typed payload per variant. An extension
/// command has no such payload — nothing in this build knows its shape — so it
/// carries its id and the way to answer, and nothing else.
///
/// ADR 0002 keeps that distinction on purpose. If a real consumer turns out to
/// need an extension command to carry a built-in-shaped payload, that is the
/// signal the split is wrong, and it is the thing to revisit first.
#[derive(Debug)]
pub struct ExtInvocation {
    id: ExtId,
    events: EventSink,
}

impl ExtInvocation {
    /// Which registered command is being handled.
    pub fn id(&self) -> ExtId {
        self.id
    }

    /// Tell the UI something changed. Never blocks; `false` means the frontend
    /// has already gone away.
    pub fn emit(&self, event: Event) -> bool {
        self.events.emit(event)
    }

    /// A sink a spawned background task can keep reporting through.
    pub fn events(&self) -> EventSink {
        self.events.clone()
    }
}

/// What the bus does with one extension command.
pub trait ExtFn: Send + Sync + 'static {
    /// Handle one invocation.
    fn call(&self, invocation: ExtInvocation) -> DispatchFuture;
}

struct FnExt<F>(F);

impl<F, Fut> ExtFn for FnExt<F>
where
    F: Fn(ExtInvocation) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
{
    fn call(&self, invocation: ExtInvocation) -> DispatchFuture {
        Box::pin((self.0)(invocation))
    }
}

/// What the bus does with one command.
///
/// Implement it for a struct that needs state of its own; closures go through
/// [`DispatcherBuilder::on`] instead.
pub trait CommandFn: Send + Sync + 'static {
    /// Handle one invocation.
    fn call(&self, invocation: Invocation) -> DispatchFuture;
}

struct FnCommand<F>(F);

impl<F, Fut> CommandFn for FnCommand<F>
where
    F: Fn(Invocation) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
{
    fn call(&self, invocation: Invocation) -> DispatchFuture {
        Box::pin((self.0)(invocation))
    }
}

/// Wires handlers to command ids. Every command the application supports is
/// registered here exactly once.
#[derive(Default)]
pub struct DispatcherBuilder {
    handlers: HashMap<CommandId, Arc<dyn CommandFn>>,
    /// A parallel map, so the built-in path does not slow down or change
    /// shape to accommodate a vocabulary it does not share.
    ext: HashMap<ExtId, Arc<dyn ExtFn>>,
}

impl fmt::Debug for DispatcherBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DispatcherBuilder")
            .field("wired", &self.handlers.len())
            .finish()
    }
}

impl DispatcherBuilder {
    /// An empty bus.
    pub fn new() -> Self {
        DispatcherBuilder::default()
    }

    /// Register an async closure for one command.
    ///
    /// Panics if that command already has a handler: two handlers for one id
    /// is a wiring bug, and it is much cheaper to find at startup than as a
    /// key that quietly does the wrong thing.
    #[track_caller]
    pub fn on<F, Fut>(self, command: CommandId, handler: F) -> Self
    where
        F: Fn(Invocation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
    {
        self.on_handler(command, FnCommand(handler))
    }

    /// Register one closure for several commands — archive and archive-thread
    /// differ only in what they resolve their target to.
    #[track_caller]
    pub fn on_each<I, F, Fut>(mut self, commands: I, handler: F) -> Self
    where
        I: IntoIterator<Item = CommandId>,
        F: Fn(Invocation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
    {
        let shared: Arc<dyn CommandFn> = Arc::new(FnCommand(handler));
        for command in commands {
            self = self.insert(command, Arc::clone(&shared));
        }
        self
    }

    /// Register a handler that is a type of its own rather than a closure.
    #[track_caller]
    pub fn on_handler(self, command: CommandId, handler: impl CommandFn) -> Self {
        self.insert(command, Arc::new(handler))
    }

    #[track_caller]
    fn insert(mut self, command: CommandId, handler: Arc<dyn CommandFn>) -> Self {
        assert!(
            self.handlers.insert(command, handler).is_none(),
            "`{command}` already has a handler"
        );
        self
    }

    /// Register a handler for a command registered at runtime.
    ///
    /// The other half of `registry::register`: that makes the command
    /// discoverable, this makes it run. Panics on a duplicate for the same
    /// reason [`on`](Self::on) does.
    #[track_caller]
    pub fn on_ext<F, Fut>(mut self, command: ExtId, handler: F) -> Self
    where
        F: Fn(ExtInvocation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
    {
        assert!(
            self.ext.insert(command, Arc::new(FnExt(handler))).is_none(),
            "`{command}` already has a handler"
        );
        self
    }

    /// Finish the bus.
    pub fn build(self) -> Dispatcher {
        Dispatcher {
            handlers: self.handlers,
            ext: self.ext,
        }
    }
}

/// The command bus: routes each [`Command`] to its handler.
///
/// Give it to [`Bridge::new`](crate::bridge::Bridge::new) and it becomes the
/// application's only path from intent to change.
#[derive(Clone)]
pub struct Dispatcher {
    handlers: HashMap<CommandId, Arc<dyn CommandFn>>,
    ext: HashMap<ExtId, Arc<dyn ExtFn>>,
}

impl fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatcher")
            .field("wired", &self.handlers.len())
            .finish()
    }
}

impl Dispatcher {
    /// Start wiring a bus.
    pub fn builder() -> DispatcherBuilder {
        DispatcherBuilder::new()
    }

    /// Whether this command has a handler.
    pub fn handles(&self, command: CommandId) -> bool {
        self.handlers.contains_key(&command)
    }

    /// Whether this registered command has a handler.
    pub fn handles_ext(&self, command: ExtId) -> bool {
        self.ext.contains_key(&command)
    }

    /// Every command that has one, in registry order.
    ///
    /// The application asserts on this: a registry entry with no handler is a
    /// palette row that does nothing.
    pub fn wired(&self) -> impl Iterator<Item = CommandId> + '_ {
        CommandId::ALL
            .iter()
            .copied()
            .filter(|command| self.handles(*command))
    }

    /// Run one command to completion, turning every outcome into events.
    ///
    /// Must be called from inside a tokio runtime — the bus runs on the
    /// [`Bridge`](crate::bridge::Bridge), which is what provides one.
    pub async fn dispatch(&self, command: Command, events: EventSink) {
        let id = command.id();
        let Some(handler) = self.handlers.get(&id).cloned() else {
            // Silence here would show up as a dead keystroke with nothing in
            // the log to explain it.
            events.emit(Event::CommandRejected {
                command: ActionId::Builtin(id),
                reason: format!(
                    "`{}` is not wired up in this build",
                    registry::get(id).title
                ),
            });
            return;
        };

        let invocation = Invocation {
            command,
            events: events.clone(),
        };
        // Running the handler as its own task turns a panic into a `JoinError`
        // instead of unwinding the pump and taking the whole bus with it.
        // Awaiting it right here keeps dispatch serialized.
        match tokio::spawn(handler.call(invocation)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                events.emit(error.into_event(id));
            }
            Err(join_error) if join_error.is_panic() => {
                events.emit(Event::Error {
                    message: format!("{} failed unexpectedly", registry::get(id).title),
                });
            }
            // Cancelled: the runtime is shutting down and there is no one left
            // to tell.
            Err(_) => {}
        }
    }
}

impl Dispatcher {
    /// Run one extension command to completion.
    ///
    /// The parallel of [`dispatch`](Self::dispatch), and the reason a
    /// registered command is a command rather than a palette row that does
    /// nothing. Every outcome becomes an event, exactly as for a built-in.
    pub async fn dispatch_ext(&self, command: ExtId, events: EventSink) {
        let title = registry::spec(ActionId::Ext(command))
            .map(|spec| spec.title)
            .unwrap_or_else(|| command.as_str());
        let Some(handler) = self.ext.get(&command).cloned() else {
            events.emit(Event::CommandRejected {
                command: ActionId::Ext(command),
                reason: format!("`{title}` is not wired up in this build"),
            });
            return;
        };

        let invocation = ExtInvocation {
            id: command,
            events: events.clone(),
        };
        match tokio::spawn(handler.call(invocation)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                events.emit(error.into_event(ActionId::Ext(command)));
            }
            Err(join_error) if join_error.is_panic() => {
                events.emit(Event::Error {
                    message: format!("{title} failed unexpectedly"),
                });
            }
            Err(_) => {}
        }
    }
}

impl CommandHandler for Dispatcher {
    fn handle(&self, command: Command, events: EventSink) -> HandlerFuture {
        let bus = self.clone();
        Box::pin(async move { bus.dispatch(command, events).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejection_and_a_failure_are_different_events() {
        assert_eq!(
            CommandError::rejected("nothing to undo").into_event(CommandId::Undo),
            Event::CommandRejected {
                command: CommandId::Undo.into(),
                reason: "nothing to undo".into(),
            }
        );
        assert_eq!(
            CommandError::failed("the disk is full").into_event(CommandId::SaveDraft),
            Event::Error {
                message: "the disk is full".into(),
            }
        );
    }

    #[test]
    #[should_panic(expected = "already has a handler")]
    fn registering_a_command_twice_is_a_wiring_bug() {
        Dispatcher::builder()
            .on(CommandId::Archive, |_| async { Ok(()) })
            .on(CommandId::Archive, |_| async { Ok(()) });
    }

    #[test]
    fn wired_commands_are_reported_in_registry_order() {
        let bus = Dispatcher::builder()
            .on(CommandId::Undo, |_| async { Ok(()) })
            .on(CommandId::Archive, |_| async { Ok(()) })
            .build();
        assert_eq!(
            bus.wired().collect::<Vec<_>>(),
            vec![CommandId::Archive, CommandId::Undo]
        );
        assert!(!bus.handles(CommandId::Send));
    }
}
