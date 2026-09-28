//! Focus mode: what Postio Focus switches on in the host that holds the
//! store, and nothing else does (spec 007, `contracts/engine.md`).
//!
//! Focus's rules act only while Focus runs (FR-134). Mail another app files
//! lands in the inbox as it always has, and Focus sorts it when it next
//! opens. So the host has a mode, and only `postio-focus` calls
//! [`Host::enable_focus`]: the classic app and the terminal never do, and
//! nothing here runs while either of them holds the store. The mode
//! installs three things:
//!
//! 1. **A filing pass** in every sync engine, through the wiring's
//!    [`postio_runtime::FilingSlot`]: each incremental pass hands it what
//!    arrived, in the transaction that filed it ([`postio_sync::filing`]).
//!    It is Focus's own ([`postio_sync::FocusFiling`]), built from `[focus]`:
//!    spam and updates filed away with their reasons, guarded mail left
//!    alone.
//! 2. **A body-stage task**, on the body indexer's pattern: it hears every
//!    `BodyLoaded`, for invitations and the needs-action detector to read
//!    the bodies that land. It listens, and reads nothing yet.
//! 3. **A due timer** on the engine's tick
//!    ([`postio_runtime::POLL_INTERVAL`]), for digest deliveries, reminders
//!    and RSVP windows. One for the store rather than one per account, so it
//!    keeps time whether or not an engine is running. It ticks, and nothing
//!    is due yet.

use std::sync::Arc;

use postio_config::FocusConfig;
use postio_sync::{FilingPass, FocusFiling};

use crate::{Host, Inner};

/// What Focus mode runs with: `[focus]` as `config.toml` says it, and the
/// filing pass built from it.
#[derive(Debug, Clone, Default)]
pub struct FocusSetup {
    config: FocusConfig,
    /// A pass to file with instead of Focus's own, for a test that watches
    /// what the engines hand over.
    filing: Option<Arc<dyn FilingPass>>,
}

impl FocusSetup {
    /// Focus mode as `config` -- the `[focus]` section -- says: whether mail
    /// is filed away, whom never to, and the digest rules.
    pub fn with_config(mut self, config: FocusConfig) -> Self {
        self.config = config;
        self
    }

    /// File what arrives with `pass` rather than Focus's own.
    pub fn filing(mut self, pass: Arc<dyn FilingPass>) -> Self {
        self.filing = Some(pass);
        self
    }

    /// The pass the engines file with: the one given, or Focus's own as
    /// the config sets it.
    fn pass(&self) -> Arc<dyn FilingPass> {
        self.filing
            .clone()
            .unwrap_or_else(|| Arc::new(FocusFiling::from_config(&self.config)))
    }
}

/// Focus mode, on in a host: the tasks it runs.
///
/// Dropping it switches nothing off. Focus mode lasts as long as the host,
/// and its tasks stop with the host's runtime.
#[derive(Debug, Clone)]
pub struct FocusHandle {
    body_stage: tokio::task::AbortHandle,
    due_timer: tokio::task::AbortHandle,
}

impl FocusHandle {
    /// Whether the body stage and the due timer are both still running.
    pub fn running(&self) -> bool {
        !self.body_stage.is_finished() && !self.due_timer.is_finished()
    }
}

impl Host {
    /// Turn on Focus's pipeline in this process (spec 007).
    ///
    /// Called by `postio-focus` at startup, after the host starts and
    /// before [`Host::start_syncing`], and by nothing else: the classic app
    /// and the terminal never call it, so none of this runs while they hold
    /// the store. The filing pass reaches every engine, running or to come,
    /// before its next pass starts.
    ///
    /// A second call changes the filing pass and answers the tasks the
    /// first one started: it is how a changed `[focus]` reaches the pass.
    pub fn enable_focus(&self, setup: FocusSetup) -> FocusHandle {
        self.inner.wiring.filing.set(Some(setup.pass()));
        let mut focus = self.inner.focus.lock().expect("never poisoned");
        if let Some(running) = &*focus {
            return running.clone();
        }
        let handle = FocusHandle {
            body_stage: body_stage(&self.inner),
            due_timer: due_timer(&self.inner),
        };
        *focus = Some(handle.clone());
        handle
    }
}

/// The body-stage task: every `BodyLoaded` the host hears.
///
/// Over a wiring whose events go to a single reader rather than a hub,
/// there is nothing to hear, and the task ends at once.
fn body_stage(inner: &Inner) -> tokio::task::AbortHandle {
    let events = inner.hub.subscribe("focus:body-stage");
    inner
        .runtime()
        .spawn(async move {
            let Some(events) = events else {
                return;
            };
            // Drained, so the hub never queues for a reader that is not
            // reading; nothing reads a body for Focus yet.
            while events.next().await.is_some() {}
        })
        .abort_handle()
}

/// The due timer: the engine's tick, kept by the host for the whole store.
fn due_timer(inner: &Inner) -> tokio::task::AbortHandle {
    inner
        .runtime()
        .spawn(async {
            let mut tick = tokio::time::interval(postio_runtime::POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                // Nothing is due yet: digest deliveries, reminders and RSVP
                // windows will be.
                tick.tick().await;
            }
        })
        .abort_handle()
}
