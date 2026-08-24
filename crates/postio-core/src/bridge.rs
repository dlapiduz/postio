//! The tokio↔glib bridge: where the asynchronous half of Postio meets the
//! single-threaded UI main loop.
//!
//! GTK's main loop is not tokio's, and neither one may be run inside the
//! other. The bridge is the only place the two touch:
//!
//! ```text
//!   GTK main loop                    tokio worker threads
//!   ------------------               --------------------------
//!   CommandSender::send  --------->  pump task --> CommandHandler
//!   EventStream::next    <---------  EventSink::emit
//! ```
//!
//! Both directions are *unbounded, non-blocking channels*, which is the whole
//! point: `send` returns immediately, so the UI thread never awaits the
//! backend, and `next` is awaited from `glib::spawn_future_local` on the GTK
//! side, so no backend work ever runs on the UI thread.
//!
//! # Using it from GTK
//!
//! The frontend keeps the [`CommandSender`] in its widgets and drains the
//! [`EventStream`] in one local task:
//!
//! ```ignore
//! let (bridge, events) = Bridge::new(dispatcher)?;
//! glib::spawn_future_local(async move {
//!     while let Some(event) = events.next().await {
//!         window.apply(event);   // repaint; < 16 ms
//!     }
//! });
//! ```
//!
//! No GTK type appears here, and none may: `postio-core` must stay
//! UI-agnostic (`scripts/check-crate-boundaries.py`), which is what keeps a
//! macOS frontend possible.
//!
//! # Why commands are handled one at a time
//!
//! The pump awaits each handler before taking the next command, so `archive`
//! then `undo` cannot race — the undo stack and app state see a total order
//! without a lock. That is affordable because every handler is *local-first*:
//! a SQLite write, an enqueued operation, an event. Anything slow — a fetch, a
//! send, a resync — is `tokio::spawn`ed by the handler and reports back later
//! through its own events, so it never becomes head-of-line blocking.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, thread};

use crate::invocation::{EventEnvelope, InvocationId};
use crate::{Command, Event};

/// How long [`Bridge::shutdown`] waits for in-flight work before it stops
/// caring. Quitting is not a good time to hang on a wedged socket.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// The future a [`CommandHandler`] returns.
///
/// Boxed so the handler can be a trait object: the frontend holds one
/// `Bridge`, not one per handler type.
pub type HandlerFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// What the runtime does with a command.
///
/// One implementation matters in the application — the command bus — but the
/// bridge is deliberately generic over it, so a test can drive the whole
/// runtime with a closure and no UI, no database and no network.
pub trait CommandHandler: Send + Sync + 'static {
    /// Handle one command, emitting whatever the UI needs to know about.
    ///
    /// Runs on a tokio worker thread, inside the runtime, so `tokio::spawn`
    /// works here. Handlers report failure as an [`Event`] rather than by
    /// panicking or returning an error: a command that cannot run is
    /// [`Event::CommandRejected`], and a failure the user should see is
    /// [`Event::Error`].
    fn handle(&self, command: Command, events: EventSink) -> HandlerFuture;
}

/// A [`CommandHandler`] made from an async closure.
///
/// Built by [`handler_fn`].
pub struct FnHandler<F>(F);

impl<F, Fut> CommandHandler for FnHandler<F>
where
    F: Fn(Command, EventSink) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn handle(&self, command: Command, events: EventSink) -> HandlerFuture {
        Box::pin((self.0)(command, events))
    }
}

/// Turn an async closure into a [`CommandHandler`].
///
/// ```
/// use postio_core::bridge::{Bridge, handler_fn};
/// use postio_core::{Command, Event};
///
/// let (bridge, events) = Bridge::new(handler_fn(|command: Command, events: postio_core::bridge::EventSink| async move {
///     events.emit(Event::ActionCompleted {
///         description: command.id().to_string(),
///         undoable: false,
///     });
/// }))
/// .expect("the runtime starts");
///
/// bridge.commands().send(Command::Refresh).expect("running");
/// bridge.shutdown();
///
/// assert!(matches!(events.try_next(), Some(Event::ActionCompleted { .. })));
/// ```
pub fn handler_fn<F, Fut>(handler: F) -> FnHandler<F>
where
    F: Fn(Command, EventSink) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    FnHandler(handler)
}

/// The runtime is no longer accepting commands.
///
/// Only happens after [`Bridge::shutdown`] or once the `Bridge` has been
/// dropped — during teardown, in other words. A frontend that sees this should
/// stop, not retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStopped;

impl fmt::Display for RuntimeStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the core runtime has stopped and accepts no more commands")
    }
}

impl std::error::Error for RuntimeStopped {}

/// One command on the queue, and whether anybody is waiting on the answer.
///
/// Private: the queue's shape is the bridge's business, and both ways of
/// sending are methods on [`CommandSender`].
#[derive(Debug)]
struct Queued {
    command: Command,
    invocation: Option<InvocationId>,
}

/// The UI's end of the command channel: clone it into every widget.
///
/// [`send`](CommandSender::send) never blocks and never awaits, so it is safe
/// to call from a GTK signal handler.
#[derive(Debug, Clone)]
pub struct CommandSender(async_channel::Sender<Queued>);

impl CommandSender {
    /// Queue a command for the runtime. Returns immediately.
    ///
    /// Fire-and-forget: the events it causes are indistinguishable from
    /// everything else on the stream. That is what the frontend wants — a
    /// repaint does not care which keystroke caused it — and it is what
    /// [`send_tracked`](Self::send_tracked) is for when it is not.
    pub fn send(&self, command: Command) -> Result<(), RuntimeStopped> {
        self.0
            .try_send(Queued {
                command,
                invocation: None,
            })
            .map_err(|_| RuntimeStopped)
    }

    /// Queue a command and get back the id its events will carry.
    ///
    /// For a caller that has to know how *its own* command ended while the
    /// sync engine is emitting events of its own: read the stream with
    /// [`EventStream::next_tracked`] and keep what
    /// [`EventEnvelope::is_from`] agrees with. Exactly one
    /// [`Event::InvocationFinished`] arrives per tracked send, whatever
    /// happened, so waiting for the answer terminates.
    ///
    /// Costs the untracked path nothing: an id is one relaxed increment, and
    /// it is only taken when somebody asks.
    pub fn send_tracked(&self, command: Command) -> Result<InvocationId, RuntimeStopped> {
        let invocation = InvocationId::next();
        self.0
            .try_send(Queued {
                command,
                invocation: Some(invocation),
            })
            .map_err(|_| RuntimeStopped)?;
        Ok(invocation)
    }

    /// Whether the runtime has stopped accepting commands.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// A handler's end of the event channel.
///
/// Cloneable and `Send`, so a spawned background task can keep reporting after
/// the handler that started it has returned.
///
/// A sink may be *tagged* with the invocation it is answering, in which case
/// everything emitted through it — including from a task spawned much later —
/// carries that origin. That is what makes a body fetch attributable to the
/// command that asked for it.
#[derive(Debug, Clone)]
pub struct EventSink {
    events: async_channel::Sender<EventEnvelope>,
    origin: Option<InvocationId>,
}

impl EventSink {
    /// Tell the frontend something happened. Never blocks.
    ///
    /// Returns `false` when there is no longer a frontend listening — the
    /// window closed while this handler was in flight. That is an ordinary
    /// teardown race, not an error worth propagating.
    pub fn emit(&self, event: Event) -> bool {
        self.events
            .try_send(EventEnvelope {
                event,
                origin: self.origin,
            })
            .is_ok()
    }

    /// The invocation everything emitted through this sink answers.
    pub fn origin(&self) -> Option<InvocationId> {
        self.origin
    }

    /// The same channel, answering `invocation`.
    ///
    /// The bridge tags a handler's sink for it. This is here for the
    /// extension path, which dispatches through
    /// [`Dispatcher::dispatch_ext`](crate::dispatch::Dispatcher::dispatch_ext)
    /// rather than through the command queue.
    pub fn with_origin(&self, invocation: InvocationId) -> Self {
        EventSink {
            events: self.events.clone(),
            origin: Some(invocation),
        }
    }

    /// Whether anyone is still listening.
    pub fn is_closed(&self) -> bool {
        self.events.is_closed()
    }
}

/// The frontend's end of the event channel: exactly one consumer.
///
/// Not `Clone` on purpose. The channel delivers each event to a *single*
/// receiver, so a second handle would steal repaints from the first; fanning
/// out to several widgets is the frontend's job, and it has a main loop to do
/// it on.
#[derive(Debug)]
pub struct EventStream(async_channel::Receiver<EventEnvelope>);

impl EventStream {
    /// Await the next event; `None` once the runtime has stopped and every
    /// queued event has been read.
    ///
    /// This is what `glib::spawn_future_local` drives on the GTK thread. It
    /// discards the correlation envelope, because a repaint does not care
    /// which send caused it; a caller that does care reads
    /// [`next_tracked`](Self::next_tracked) instead.
    pub async fn next(&self) -> Option<Event> {
        self.next_tracked().await.map(|envelope| envelope.event)
    }

    /// Take an event if one is already queued, without awaiting.
    pub fn try_next(&self) -> Option<Event> {
        self.try_next_tracked().map(|envelope| envelope.event)
    }

    /// Block until the next event arrives; `None` once the runtime has stopped
    /// and every queued event has been read.
    ///
    /// For headless consumers — tests, a future CLI. Never call it from the UI
    /// thread: that is precisely the freeze this module exists to prevent.
    pub fn next_blocking(&self) -> Option<Event> {
        self.next_blocking_tracked().map(|envelope| envelope.event)
    }

    /// [`next`](Self::next), keeping the invocation each event answers.
    pub async fn next_tracked(&self) -> Option<EventEnvelope> {
        self.0.recv().await.ok()
    }

    /// [`try_next`](Self::try_next), keeping the invocation each event answers.
    pub fn try_next_tracked(&self) -> Option<EventEnvelope> {
        self.0.try_recv().ok()
    }

    /// [`next_blocking`](Self::next_blocking), keeping the invocation each
    /// event answers.
    pub fn next_blocking_tracked(&self) -> Option<EventEnvelope> {
        self.0.recv_blocking().ok()
    }

    /// Whether the runtime has stopped *and* the queue has been drained.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed() && self.0.is_empty()
    }

    /// How many events are waiting to be applied.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no event is waiting.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// An event channel with no runtime behind it.
///
/// The [`Bridge`] builds its own; this is for the pieces that emit events
/// without owning a runtime — the config watcher on its own thread — and for
/// tests that want the frontend's end without starting one.
pub fn event_channel() -> (EventSink, EventStream) {
    let (sender, receiver) = async_channel::unbounded();
    (
        EventSink {
            events: sender,
            origin: None,
        },
        EventStream(receiver),
    )
}

/// How to build a [`Bridge`]. The defaults are what the application uses.
#[derive(Debug, Clone)]
pub struct BridgeBuilder {
    worker_threads: Option<usize>,
    shutdown_timeout: Duration,
}

impl Default for BridgeBuilder {
    fn default() -> Self {
        BridgeBuilder {
            worker_threads: None,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }
}

impl BridgeBuilder {
    /// A builder with the application's defaults.
    pub fn new() -> Self {
        BridgeBuilder::default()
    }

    /// How many worker threads to run. Defaults to tokio's choice, one per
    /// core; tests that only need ordering set this to 1 to stay cheap.
    pub fn worker_threads(mut self, threads: usize) -> Self {
        self.worker_threads = Some(threads.max(1));
        self
    }

    /// How long teardown waits for in-flight work before abandoning it.
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Start the runtime and its command pump.
    ///
    /// Fails only if the OS will not give us threads.
    pub fn build<H: CommandHandler>(self, handler: H) -> io::Result<(Bridge, EventStream)> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        // Both drivers, explicitly. This is the runtime the sync engine runs
        // on, so every socket the application opens is opened here, and a
        // runtime with only the timer enabled panics the first time one is --
        // which is what 0.1.0 did on first launch, the moment onboarding
        // tried to reach a server. `enable_all` would do the same thing today
        // but says less: it enables whatever drivers happen to be compiled
        // in, so it would go quietly back to timer-only if the `net` feature
        // ever left this crate's dependency.
        builder.enable_io().enable_time().thread_name("postio-core");
        if let Some(threads) = self.worker_threads {
            builder.worker_threads(threads);
        }
        let runtime = builder.build()?;

        // Unbounded in both directions: the UI must never block on a full
        // queue, and a burst of IDLE updates must never stall the sync engine.
        let (command_tx, command_rx) = async_channel::unbounded::<Queued>();
        let (event_tx, event_rx) = async_channel::unbounded::<EventEnvelope>();

        let handler = Arc::new(handler);
        let sink = EventSink {
            events: event_tx,
            origin: None,
        };
        let pump = runtime.spawn(async move {
            // `recv` keeps yielding buffered commands after the sender closes,
            // so everything already queued at quit still gets handled.
            while let Ok(queued) = command_rx.recv().await {
                // Tagging the sink rather than the handler is what makes
                // tracking free for everyone else: the handler signature does
                // not change, and a handler that never heard of correlation
                // still emits attributable events.
                let sink = match queued.invocation {
                    Some(invocation) => sink.with_origin(invocation),
                    None => sink.clone(),
                };
                handler.handle(queued.command, sink).await;
            }
        });

        let bridge = Bridge {
            runtime: Some(runtime),
            pump: Some(pump),
            commands: command_tx,
            shutdown_timeout: self.shutdown_timeout,
        };
        Ok((bridge, EventStream(event_rx)))
    }
}

/// The core runtime: owns the tokio threads, the command queue and the pump.
///
/// Hold it for as long as the application runs. Dropping it shuts the runtime
/// down, so a `Bridge` that goes out of scope takes the backend with it.
#[derive(Debug)]
pub struct Bridge {
    runtime: Option<tokio::runtime::Runtime>,
    pump: Option<tokio::task::JoinHandle<()>>,
    commands: async_channel::Sender<Queued>,
    shutdown_timeout: Duration,
}

impl Bridge {
    /// Start a runtime that drives `handler`, and the stream of events it
    /// produces.
    pub fn new<H: CommandHandler>(handler: H) -> io::Result<(Bridge, EventStream)> {
        BridgeBuilder::new().build(handler)
    }

    /// Configure a runtime before starting it.
    pub fn builder() -> BridgeBuilder {
        BridgeBuilder::new()
    }

    /// A sender for the command queue. Clone one into every widget.
    pub fn commands(&self) -> CommandSender {
        CommandSender(self.commands.clone())
    }

    /// A handle for spawning long-running work on the runtime — the sync
    /// engine's IDLE loop, a body fetch — from outside a handler.
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime
            .as_ref()
            .expect("the runtime outlives every method on Bridge")
            .handle()
            .clone()
    }

    /// Spawn a background task on the runtime.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle().spawn(future)
    }

    /// Stop accepting commands, finish what is already queued, and join.
    ///
    /// Blocks the calling thread — at quit, on the UI thread, which is the one
    /// moment that is the right thing to do — for at most the shutdown
    /// timeout. Detached background work does not extend it: a wedged socket
    /// must not keep the window on screen.
    ///
    /// Do not call this from inside the runtime; tokio forbids dropping a
    /// runtime from an async context.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // Closing the queue is what ends the pump loop, once it has drained.
        self.commands.close();

        if let (Some(runtime), Some(pump)) = (self.runtime.as_ref(), self.pump.take()) {
            let timeout = self.shutdown_timeout;
            let drained =
                runtime.block_on(async move { tokio::time::timeout(timeout, pump).await.is_ok() });
            debug_assert!(drained, "the command pump did not drain within {timeout:?}");
        }

        if let Some(runtime) = self.runtime.take() {
            // Detached tasks get dropped rather than waited for; the timeout
            // only covers threads that are mid-blocking-call.
            runtime.shutdown_timeout(self.shutdown_timeout);
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if thread::panicking() {
            // A panicking test thread may already be inside the runtime;
            // blocking here would turn one failure into an abort.
            self.commands.close();
            return;
        }
        self.stop();
    }
}
