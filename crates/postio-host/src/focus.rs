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
//! 2. **A body-stage task**, on the body indexer's pattern: it hears every
//!    `BodyLoaded`, for invitations and the needs-action detector to read
//!    the bodies that land. It listens, and reads nothing yet.
//! 3. **A due timer** on the engine's tick
//!    ([`postio_runtime::POLL_INTERVAL`]), for digest deliveries, reminders
//!    and RSVP windows. One for the store rather than one per account, so it
//!    keeps time whether or not an engine is running. It ticks, and nothing
//!    is due yet.

use std::sync::Arc;

use postio_sync::{FilingPass, NoFiling};

use crate::{Host, Inner};

/// What Focus mode runs with.
#[derive(Debug, Clone)]
pub struct FocusSetup {
    filing: Arc<dyn FilingPass>,
}

impl Default for FocusSetup {
    /// Focus mode as Focus runs it until its rules exist: a filing pass
    /// that files nothing.
    fn default() -> Self {
        FocusSetup {
            filing: Arc::new(NoFiling),
        }
    }
}

impl FocusSetup {
    /// File what arrives with `pass`.
    pub fn filing(mut self, pass: Arc<dyn FilingPass>) -> Self {
        self.filing = pass;
        self
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
    /// first one started.
    pub fn enable_focus(&self, setup: FocusSetup) -> FocusHandle {
        self.inner.wiring.filing.set(Some(setup.filing));
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
