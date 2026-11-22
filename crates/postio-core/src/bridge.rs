//! The only place tokio meets the glib main loop: unbounded, non-blocking channels, so the UI
//! never awaits the backend. Every subscriber of `EventHub` sees every event (ADR 0013).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{fmt, thread};

use crate::invocation::{EventEnvelope, InvocationId};
use crate::{Command, Event};

/// How long `Bridge::shutdown` waits for in-flight work; quitting must not hang on a wedged socket.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Queue depth at which a subscriber is named in the log. Queues are unbounded so the UI never
/// blocks a handler, so crossing this means a consumer stopped reading (ADR 0013 Q2).
pub const SUBSCRIBER_DEPTH_WATERMARK: usize = 1024;

pub type HandlerFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub trait CommandHandler: Send + Sync + 'static {
    /// Runs on a tokio worker, so `tokio::spawn` works. Failure is an `Event`, not a panic or
    /// `Err`.
    fn handle(&self, command: Command, events: EventSink) -> HandlerFuture;
}

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

#[doc(hidden)]
pub fn handler_fn<F, Fut>(handler: F) -> FnHandler<F>
where
    F: Fn(Command, EventSink) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    FnHandler(handler)
}

/// Only after shutdown or drop; a frontend seeing this should stop, not retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStopped;

impl fmt::Display for RuntimeStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the core runtime has stopped and accepts no more commands")
    }
}

impl std::error::Error for RuntimeStopped {}

#[derive(Debug)]
struct Queued {
    command: Command,
    invocation: Option<InvocationId>,
}

/// Cloneable; `send` never blocks, so it is safe in a GTK signal handler.
#[derive(Debug, Clone)]
pub struct CommandSender(async_channel::Sender<Queued>);

impl CommandSender {
    /// Fire-and-forget; use `send_tracked` to learn how this command ended.
    pub fn send(&self, command: Command) -> Result<(), RuntimeStopped> {
        self.0
            .try_send(Queued {
                command,
                invocation: None,
            })
            .map_err(|_| RuntimeStopped)
    }

    /// Read the stream with `EventStream::next_tracked` and keep what `EventEnvelope::is_from`
    /// matches. Exactly one `Event::InvocationFinished` arrives per tracked send.
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

    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// Commands a `CommandSender` queued, for a frontend that runs them outside a `Bridge` (ADR 0041).
#[derive(Debug)]
pub struct CommandReceiver(async_channel::Receiver<Queued>);

impl CommandReceiver {
    /// The next command, once one is sent; `None` when every sender is gone.
    pub async fn recv(&self) -> Option<Command> {
        self.0.recv().await.ok().map(|queued| queued.command)
    }

    pub fn try_recv(&self) -> Option<Command> {
        self.0.try_recv().ok().map(|queued| queued.command)
    }
}

/// Untracked only: a tracked id taken here would not match what the eventual runtime reports.
pub fn command_channel() -> (CommandSender, CommandReceiver) {
    let (sender, receiver) = async_channel::unbounded();
    (CommandSender(sender), CommandReceiver(receiver))
}

/// A sink tagged with an invocation stamps that origin on everything emitted through it, even
/// from a task spawned later.
#[derive(Debug, Clone)]
pub struct EventSink {
    events: EventTarget,
    origin: Option<InvocationId>,
}

#[derive(Debug, Clone)]
enum EventTarget {
    Direct(async_channel::Sender<EventEnvelope>),
    Hub(Arc<HubInner>),
}

impl EventSink {
    /// `false` when nothing took the event: the frontend is gone (a teardown race), or a hub has
    /// no subscription.
    pub fn emit(&self, event: Event) -> bool {
        let envelope = EventEnvelope {
            event,
            origin: self.origin,
        };
        match &self.events {
            EventTarget::Direct(events) => events.try_send(envelope).is_ok(),
            EventTarget::Hub(hub) => hub.deliver(envelope),
        }
    }

    pub fn origin(&self) -> Option<InvocationId> {
        self.origin
    }

    /// For the extension path, which dispatches without the command queue.
    pub fn with_origin(&self, invocation: InvocationId) -> Self {
        EventSink {
            events: self.events.clone(),
            origin: Some(invocation),
        }
    }

    /// On a hub, no subscription can take an event, including before the first one joins.
    pub fn is_closed(&self) -> bool {
        match &self.events {
            EventTarget::Direct(events) => events.is_closed(),
            EventTarget::Hub(hub) => hub.is_closed(),
        }
    }
}

/// Not `Clone`: each event goes to one receiver, so a second handle would steal repaints.
#[derive(Debug)]
pub struct EventStream(async_channel::Receiver<EventEnvelope>);

impl EventStream {
    /// `None` once the runtime has stopped and the queue is drained.
    pub async fn next(&self) -> Option<Event> {
        self.next_tracked().await.map(|envelope| envelope.event)
    }

    pub fn try_next(&self) -> Option<Event> {
        self.try_next_tracked().map(|envelope| envelope.event)
    }

    /// Never call it from the UI thread.
    pub fn next_blocking(&self) -> Option<Event> {
        self.next_blocking_tracked().map(|envelope| envelope.event)
    }

    pub async fn next_tracked(&self) -> Option<EventEnvelope> {
        self.0.recv().await.ok()
    }

    pub fn try_next_tracked(&self) -> Option<EventEnvelope> {
        self.0.try_recv().ok()
    }

    pub fn next_blocking_tracked(&self) -> Option<EventEnvelope> {
        self.0.recv_blocking().ok()
    }

    pub fn is_closed(&self) -> bool {
        self.0.is_closed() && self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// An event channel with no runtime, for the config watcher and tests.
pub fn event_channel() -> (EventSink, EventStream) {
    let (sender, receiver) = async_channel::unbounded();
    (
        EventSink {
            events: EventTarget::Direct(sender),
            origin: None,
        },
        EventStream(receiver),
    )
}

struct Subscription {
    /// Diagnostics only; nothing may dispatch on a label (ADR 0013 Q3).
    label: String,
    events: async_channel::Sender<EventEnvelope>,
    /// One warning per crossing, not per event.
    warned: AtomicBool,
}

impl Subscription {
    fn send(&self, envelope: EventEnvelope) -> bool {
        if self.events.try_send(envelope).is_err() {
            return false;
        }
        let depth = self.events.len();
        if depth >= SUBSCRIBER_DEPTH_WATERMARK {
            if !self.warned.swap(true, Ordering::Relaxed) {
                // Label and counts only: a log of somebody's mail is a leak.
                tracing::warn!(
                    subscriber = %self.label,
                    depth,
                    watermark = SUBSCRIBER_DEPTH_WATERMARK,
                    "an event subscriber is not draining its queue"
                );
            }
        } else if depth < SUBSCRIBER_DEPTH_WATERMARK / 2 {
            // Hysteresis: no line per event once it starts draining.
            self.warned.store(false, Ordering::Relaxed);
        }
        true
    }
}

#[derive(Default)]
struct HubInner {
    subscribers: RwLock<Vec<Subscription>>,
}

impl HubInner {
    /// A poisoned lock must not lose an event; nothing under the lock can panic.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vec<Subscription>> {
        self.subscribers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<Subscription>> {
        self.subscribers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn deliver(&self, envelope: EventEnvelope) -> bool {
        let subscribers = self.read();
        // The last subscriber takes the envelope itself, so the one-window case clones nothing.
        let Some((last, rest)) = subscribers.split_last() else {
            return false;
        };
        let mut delivered = false;
        for subscription in rest {
            delivered |= subscription.send(envelope.clone());
        }
        delivered | last.send(envelope)
    }

    fn is_closed(&self) -> bool {
        self.read()
            .iter()
            .all(|subscription| subscription.events.is_closed())
    }
}

#[derive(Clone, Default)]
pub struct EventHub {
    inner: Arc<HubInner>,
}

impl EventHub {
    pub fn new() -> Self {
        EventHub::default()
    }

    pub fn sink(&self) -> EventSink {
        EventSink {
            events: EventTarget::Hub(Arc::clone(&self.inner)),
            origin: None,
        }
    }

    /// The label is diagnostics, never dispatch. The subscription starts at now; earlier events
    /// are in the store (ADR 0013 Q4).
    pub fn subscribe(&self, label: impl Into<String>) -> EventStream {
        self.inner.subscribe(label)
    }

    pub fn subscribers(&self) -> usize {
        self.inner
            .read()
            .iter()
            .filter(|subscription| !subscription.events.is_closed())
            .count()
    }
}

impl HubInner {
    fn subscribe(&self, label: impl Into<String>) -> EventStream {
        let (sender, receiver) = async_channel::unbounded();
        let mut subscribers = self.write();
        // Closed queues are pruned under the write lock a subscribe already takes, so emit stays a
        // read lock.
        subscribers.retain(|subscription| !subscription.events.is_closed());
        subscribers.push(Subscription {
            label: label.into(),
            events: sender,
            warned: AtomicBool::new(false),
        });
        EventStream(receiver)
    }
}

impl EventSink {
    /// `None` for a sink from `event_channel`, which has one reader.
    pub fn subscribe(&self, label: impl Into<String>) -> Option<EventStream> {
        match &self.events {
            EventTarget::Hub(inner) => Some(inner.subscribe(label)),
            EventTarget::Direct(_) => None,
        }
    }
}

impl fmt::Debug for EventHub {
    /// Labels and depths. Never an event.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subscribers = self.inner.read();
        f.debug_struct("EventHub")
            .field(
                "subscribers",
                &subscribers
                    .iter()
                    .map(|subscription| (subscription.label.as_str(), subscription.events.len()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl fmt::Debug for HubInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HubInner")
            .field("subscribers", &self.read().len())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct BridgeBuilder {
    worker_threads: Option<usize>,
    max_blocking_threads: Option<usize>,
    shutdown_timeout: Duration,
}

impl Default for BridgeBuilder {
    fn default() -> Self {
        BridgeBuilder {
            worker_threads: None,
            max_blocking_threads: None,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }
}

impl BridgeBuilder {
    pub fn new() -> Self {
        BridgeBuilder::default()
    }

    pub fn worker_threads(mut self, threads: usize) -> Self {
        self.worker_threads = Some(threads.max(1));
        self
    }

    /// Caps the blocking pool: each thread is another malloc arena that lingers after a burst
    /// (#1502).
    pub fn max_blocking_threads(mut self, threads: usize) -> Self {
        self.max_blocking_threads = Some(threads.max(1));
        self
    }

    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Returns the first subscription to the bridge's own hub. Fails only if the OS refuses
    /// threads.
    pub fn build<H: CommandHandler>(self, handler: H) -> io::Result<(Bridge, EventStream)> {
        let hub = EventHub::new();
        let events = hub.subscribe("bridge");
        let bridge = self.build_with_events(handler, hub.sink())?;
        Ok((bridge, events))
    }

    /// For a composition root that owns the hub. Fails only if the OS refuses threads.
    pub fn build_with_events<H: CommandHandler>(
        self,
        handler: H,
        events: EventSink,
    ) -> io::Result<Bridge> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        // Both drivers explicitly: sockets opened on this runtime panic without `io`, and
        // `enable_all` could regress to timer-only if `net` left the dependencies.
        builder.enable_io().enable_time().thread_name("postio-core");
        if let Some(threads) = self.worker_threads {
            builder.worker_threads(threads);
        }
        if let Some(threads) = self.max_blocking_threads {
            builder.max_blocking_threads(threads);
        }
        let runtime = builder.build()?;

        // Unbounded in both directions: the UI must never block on a full
        // queue, and a burst of IDLE updates must never stall the sync engine.
        let (command_tx, command_rx) = async_channel::unbounded::<Queued>();

        let handler = Arc::new(handler);
        let sink = events;
        let pump = runtime.spawn(async move {
            // `recv` keeps yielding buffered commands after the sender closes,
            // so everything already queued at quit still gets handled.
            while let Ok(queued) = command_rx.recv().await {
                let sink = match queued.invocation {
                    Some(invocation) => sink.with_origin(invocation),
                    None => sink.clone(),
                };
                handler.handle(queued.command, sink).await;
            }
        });

        Ok(Bridge {
            runtime: Some(runtime),
            pump: Some(pump),
            commands: command_tx,
            shutdown_timeout: self.shutdown_timeout,
        })
    }
}

/// Dropping it shuts the runtime down.
#[derive(Debug)]
pub struct Bridge {
    runtime: Option<tokio::runtime::Runtime>,
    pump: Option<tokio::task::JoinHandle<()>>,
    commands: async_channel::Sender<Queued>,
    shutdown_timeout: Duration,
}

impl Bridge {
    pub fn new<H: CommandHandler>(handler: H) -> io::Result<(Bridge, EventStream)> {
        BridgeBuilder::new().build(handler)
    }

    pub fn builder() -> BridgeBuilder {
        BridgeBuilder::new()
    }

    pub fn commands(&self) -> CommandSender {
        CommandSender(self.commands.clone())
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime
            .as_ref()
            .expect("the runtime outlives every method on Bridge")
            .handle()
            .clone()
    }

    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle().spawn(future)
    }

    /// Blocks the caller (the UI thread, at quit) for at most the shutdown timeout; detached work
    /// does not extend it.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.commands.close();

        if let (Some(runtime), Some(pump)) = (self.runtime.as_ref(), self.pump.take()) {
            let timeout = self.shutdown_timeout;
            let drained = blocking(|| {
                runtime.block_on(async move { tokio::time::timeout(timeout, pump).await.is_ok() })
            });
            // A miss is not proof of a hang: a dropped `JoinHandle` detaches rather than cancels,
            // and `shutdown_timeout` gives a second window (#759, #817).
            if !drained {
                tracing::warn!(
                    ?timeout,
                    "the command pump did not drain within the shutdown timeout on the first wait"
                );
            }
        }

        if let Some(runtime) = self.runtime.take() {
            // Detached tasks get dropped rather than waited for; the timeout
            // only covers threads that are mid-blocking-call.
            let timeout = self.shutdown_timeout;
            blocking(move || runtime.shutdown_timeout(timeout));
        }
    }
}

/// Runs blocking `work` from any thread, including one already inside a runtime (tests).
/// `block_in_place` needs a multi-threaded runtime, so every runtime here is one.
fn blocking<T>(work: impl FnOnce() -> T) -> T {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        assert!(
            handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread,
            "a bridge shutdown was reached from inside a current-thread \
             runtime, which cannot stand aside for `block_in_place`. Whichever \
             runtime drives this has to be built \
             `new_multi_thread().worker_threads(1)`."
        );
        return tokio::task::block_in_place(work);
    }
    work()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forwarded_command_arrives_in_the_order_it_was_sent() {
        let (sender, receiver) = command_channel();
        sender.send(Command::Undo).expect("open");
        sender.send(Command::Refresh).expect("open");
        assert_eq!(receiver.try_recv(), Some(Command::Undo));
        assert_eq!(receiver.try_recv(), Some(Command::Refresh));
        assert_eq!(receiver.try_recv(), None);
        drop(receiver);
        assert!(sender.is_closed(), "a sender with nobody reading says so");
    }
}
