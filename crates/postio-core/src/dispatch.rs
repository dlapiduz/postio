//! One dispatch path: every command reaches its single handler, which writes locally and
//! returns without awaiting the network. Failures come back as events, never panics.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::bridge::{CommandHandler, EventSink, HandlerFuture};
use crate::invocation::{InvocationId, InvocationOutcome};
use crate::{ActionId, Command, CommandId, Event, registry};

pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<(), CommandError>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Not applicable (nothing selected, nothing to undo); answered with a quiet hint.
    Rejected(String),
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

    pub fn message(&self) -> &str {
        match self {
            CommandError::Rejected(text) | CommandError::Failed(text) => text,
        }
    }

    fn into_event(self, command: impl Into<ActionId>) -> Event {
        match self {
            CommandError::Rejected(reason) => Event::CommandRejected {
                command: command.into(),
                reason,
            },
            CommandError::Failed(message) => Event::Error {
                message,
                account: None,
            },
        }
    }
}

impl From<CommandError> for InvocationOutcome {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::Rejected(reason) => InvocationOutcome::Rejected { reason },
            CommandError::Failed(message) => InvocationOutcome::Failed { message },
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug)]
pub struct Invocation {
    pub command: Command,
    events: EventSink,
}

impl Invocation {
    pub fn id(&self) -> CommandId {
        self.command.id()
    }

    /// `None` for a fire-and-forget command, which is every command the GTK frontend sends.
    pub fn invocation_id(&self) -> Option<InvocationId> {
        self.events.origin()
    }

    /// Tell the UI something changed. Never blocks; `false` means the frontend
    /// has already gone away.
    pub fn emit(&self, event: Event) -> bool {
        self.events.emit(event)
    }

    pub fn events(&self) -> EventSink {
        self.events.clone()
    }
}

pub trait CommandFn: Send + Sync + 'static {
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

#[derive(Default)]
pub struct DispatcherBuilder {
    handlers: HashMap<CommandId, Arc<dyn CommandFn>>,
}

impl fmt::Debug for DispatcherBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DispatcherBuilder")
            .field("wired", &self.handlers.len())
            .finish()
    }
}

impl DispatcherBuilder {
    pub fn new() -> Self {
        DispatcherBuilder::default()
    }

    /// Panics if the command already has a handler: a wiring bug is cheaper to find at startup.
    #[track_caller]
    pub fn on<F, Fut>(self, command: CommandId, handler: F) -> Self
    where
        F: Fn(Invocation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), CommandError>> + Send + 'static,
    {
        self.on_handler(command, FnCommand(handler))
    }

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

    pub fn build(self) -> Dispatcher {
        Dispatcher {
            handlers: self.handlers,
        }
    }
}

#[derive(Clone)]
pub struct Dispatcher {
    handlers: HashMap<CommandId, Arc<dyn CommandFn>>,
}

impl fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatcher")
            .field("wired", &self.handlers.len())
            .finish()
    }
}

impl Dispatcher {
    pub fn builder() -> DispatcherBuilder {
        DispatcherBuilder::new()
    }

    pub fn handles(&self, command: CommandId) -> bool {
        self.handlers.contains_key(&command)
    }

    /// The application asserts on this: a registry entry with no handler is a palette row that does
    /// nothing.
    pub fn wired(&self) -> impl Iterator<Item = CommandId> + '_ {
        CommandId::ALL
            .iter()
            .copied()
            .filter(|command| self.handles(*command))
    }

    /// Must be called inside a tokio runtime.
    pub async fn dispatch(&self, command: Command, events: EventSink) {
        let id = command.id();
        let Some(handler) = self.handlers.get(&id).cloned() else {
            // Silence here would show up as a dead keystroke with nothing in
            // the log to explain it.
            let reason = format!(
                "`{}` is not wired up in this build",
                registry::get(id).title
            );
            events.emit(Event::CommandRejected {
                command: ActionId::Builtin(id),
                reason: reason.clone(),
            });
            finish(&events, InvocationOutcome::Rejected { reason });
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
            Ok(Ok(())) => finish(&events, InvocationOutcome::Completed),
            Ok(Err(error)) => {
                events.emit(error.clone().into_event(id));
                finish(&events, error.into());
            }
            Err(join_error) if join_error.is_panic() => {
                let message = format!("{} failed unexpectedly", registry::get(id).title);
                events.emit(Event::Error {
                    message: message.clone(),
                    account: None,
                });
                finish(&events, InvocationOutcome::Failed { message });
            }
            Err(_) => {}
        }
    }
}

/// A tracked invocation always gets an ending, including the panic and missing-handler cases:
/// a waiter on an answer that never comes hangs.
fn finish(events: &EventSink, outcome: InvocationOutcome) {
    if let Some(invocation) = events.origin() {
        events.emit(Event::InvocationFinished {
            invocation,
            outcome,
        });
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
                account: None,
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
