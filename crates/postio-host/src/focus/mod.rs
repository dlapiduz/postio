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
//! 2. **A body stage**, on the body indexer's pattern ([`body`]): it
//!    catches up on recent inbox mail whose body is here, then hears every
//!    `BodyLoaded`, for invitations and the needs-action detector to read
//!    the bodies that land.
//! 3. **A due timer** on the engine's tick
//!    ([`postio_runtime::POLL_INTERVAL`]), for digest deliveries ([`due`]),
//!    and later reminders and RSVP windows. One for the store rather than one
//!    per account, so it keeps time whether or not an engine is running.

mod body;
mod due;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

pub(crate) use due::deliver_due;

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
    caught_up: Arc<AtomicBool>,
    /// `[focus]` as the tasks read it, replaced by each call to
    /// [`Host::enable_focus`].
    config: Arc<RwLock<FocusConfig>>,
}

impl FocusHandle {
    /// Whether the body stage and the due timer are both still running.
    pub fn running(&self) -> bool {
        !self.body_stage.is_finished() && !self.due_timer.is_finished()
    }

    /// Whether the body stage has caught up on the mail that was here when
    /// Focus started: every recent inbox body it found is classified
    /// (FR-141). From then on it classifies bodies as they land.
    pub fn caught_up(&self) -> bool {
        self.caught_up.load(Ordering::Acquire)
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
            *running.config.write().expect("never poisoned") = setup.config;
            return running.clone();
        }
        let caught_up = Arc::new(AtomicBool::new(false));
        let config = Arc::new(RwLock::new(setup.config));
        let handle = FocusHandle {
            body_stage: body::spawn(&self.inner, Arc::clone(&caught_up)),
            due_timer: due_timer(&self.inner, Arc::clone(&config)),
            caught_up,
            config,
        };
        *focus = Some(handle.clone());
        handle
    }
}

/// The due timer: the engine's tick, kept by the host for the whole store,
/// delivering the digests `config` says have come due ([`due`]). Its first
/// tick is at once, so what came due while Focus was closed is delivered as
/// it opens.
fn due_timer(inner: &Inner, config: Arc<RwLock<FocusConfig>>) -> tokio::task::AbortHandle {
    let database = inner.wiring.database.clone();
    inner
        .runtime()
        .spawn(async move {
            let mut tick = tokio::time::interval(postio_runtime::POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let config = config.read().expect("never poisoned").clone();
                match deliver_due(&database, &config, &chrono::Local::now()).await {
                    Ok(0) => {}
                    Ok(delivered) => tracing::debug!(delivered, "Focus delivered digests"),
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not deliver its digests: {error}");
                    }
                }
            }
        })
        .abort_handle()
}
