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
//!    Before it starts ticking it sorts what another app filed while Focus
//!    was closed ([`catch_up`]).

mod body;
mod catch_up;
mod due;
mod model;
pub(crate) mod rules;
mod summary;
mod surfaced;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

#[cfg(test)]
pub(crate) use catch_up::FILED_THROUGH;
pub(crate) use due::{deliver_due, fire_reminders, settle_answers};
pub(crate) use rules::like_this;
pub(crate) use summary::digest_summary;
pub(crate) use surfaced::surfaced;

use postio_config::FocusConfig;
use postio_sync::{FilingPass, FocusFiling};

use crate::{Host, Inner};

/// What Focus mode runs with: `[focus]` as `config.toml` says it, and the
/// filing pass built from it.
#[derive(Debug, Clone, Default)]
pub struct FocusSetup {
    config: FocusConfig,
    /// Where `config.toml` is, for Focus's verbs to write the person's
    /// corrections to: the path the host opened with, unless given.
    config_path: Option<std::path::PathBuf>,
    /// A pass to file with instead of Focus's own, for a test that watches
    /// what the engines hand over.
    filing: Option<Arc<dyn FilingPass>>,
    /// How the person's model is reached instead of this computer's own
    /// sockets: a test's fake runtime.
    model_transport: Option<Arc<dyn postio_ai::Transport>>,
}

impl FocusSetup {
    /// Focus mode as `config` -- the `[focus]` section -- says: whether mail
    /// is filed away, whom never to, and the digest rules.
    pub fn with_config(mut self, config: FocusConfig) -> Self {
        self.config = config;
        self
    }

    /// Write the person's corrections -- a sender restored from Filtered, a
    /// marker kind stopped, a digest rule -- to the `config.toml` at `path`
    /// (contracts/config.md). Without it, the path [`Host::open`] was given.
    pub fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Reach the person's model through `transport` rather than this
    /// computer's own sockets: what a test does, so that nothing it runs
    /// opens a connection. Nothing connects either way unless `[focus.model]`
    /// names a model (FR-166).
    pub fn with_model_transport(mut self, transport: Arc<dyn postio_ai::Transport>) -> Self {
        self.model_transport = Some(transport);
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
    /// Whether the catch-up has sorted what was filed while Focus was
    /// closed, so the mark may move: before then, moving it would forget
    /// what the catch-up had yet to sort.
    marking: Arc<AtomicBool>,
    /// `[focus]` as the tasks read it, replaced by each call to
    /// [`Host::enable_focus`].
    config: Arc<RwLock<FocusConfig>>,
    /// The person's model, as Focus's tasks and reads reach it.
    pub(crate) models: Arc<model::Models>,
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
    /// As Focus stops, after the engines have: move the mark that says how
    /// far it has accounted for the mail to where the store is now, rather
    /// than where its last tick left it (T164). Nothing when Focus mode is
    /// off, or when the catch-up had not finished and the mark still says
    /// where it has to start.
    pub(crate) fn keep_focus_mark(&self) {
        let marking = self
            .inner
            .focus
            .lock()
            .expect("never poisoned")
            .as_ref()
            .is_some_and(|focus| focus.marking.load(Ordering::Acquire));
        if !marking {
            return;
        }
        let database = self.inner.wiring.database.clone();
        if let Err(error) =
            postio_session::blocking::now(async move { catch_up::mark_newest(&database).await })
        {
            tracing::warn!(%error, "Focus could not keep its mark as it stopped: {error}");
        }
    }

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
        let handle = match &*focus {
            Some(running) => {
                *running.config.write().expect("never poisoned") = setup.config.clone();
                running.clone()
            }
            None => {
                let caught_up = Arc::new(AtomicBool::new(false));
                let marking = Arc::new(AtomicBool::new(false));
                let config = Arc::new(RwLock::new(setup.config.clone()));
                let models = Arc::new(model::Models::new(
                    setup.model_transport.clone(),
                    self.inner.wiring.egress.clone(),
                ));
                let handle = FocusHandle {
                    body_stage: body::spawn(
                        &self.inner,
                        Arc::clone(&config),
                        Arc::clone(&models),
                        Arc::clone(&caught_up),
                    ),
                    due_timer: due_timer(
                        &self.inner,
                        Arc::clone(&config),
                        Arc::clone(&models),
                        Arc::clone(&marking),
                    ),
                    caught_up,
                    marking,
                    config,
                    models,
                };
                *focus = Some(handle.clone());
                handle
            }
        };
        // Focus's verbs read `[focus]` and write the person's corrections
        // to the file. What they write reaches the filing pass and Focus's
        // tasks at once, as a reload from the file would.
        let path = setup.config_path.clone().or_else(|| {
            self.inner
                .config_path
                .lock()
                .expect("never poisoned")
                .clone()
        });
        let tasks = Arc::clone(&handle.config);
        let filing = self.inner.wiring.filing.clone();
        let chosen = setup.filing.clone();
        self.inner.wiring.focus.install(
            path,
            setup.config,
            Arc::new(move |written: &FocusConfig| {
                *tasks.write().expect("never poisoned") = written.clone();
                filing.set(Some(chosen.clone().unwrap_or_else(|| {
                    Arc::new(FocusFiling::from_config(written)) as Arc<dyn FilingPass>
                })));
            }),
        );
        handle
    }
}

/// The due timer: the engine's tick, kept by the host for the whole store.
///
/// Before its first tick it sorts what another app filed while Focus was
/// closed ([`catch_up`]), so a digest that comes due at once holds that mail
/// too. Then, each tick, it delivers the digests `config` says have come due
/// ([`due`]) -- the first tick is at once, so what came due while Focus was
/// closed is delivered as it opens -- hands what is delivered to the
/// summariser when the person's model writes summaries ([`summary`]), and
/// keeps the mark that says how far Focus has accounted for the mail.
fn due_timer(
    inner: &Arc<Inner>,
    config: Arc<RwLock<FocusConfig>>,
    models: Arc<model::Models>,
    marking: Arc<AtomicBool>,
) -> tokio::task::AbortHandle {
    let inner = Arc::clone(inner);
    let summarising = Arc::new(AtomicBool::new(false));
    let runtime = inner.runtime().clone();
    runtime
        .spawn(async move {
            match catch_up::catch_up(&inner).await {
                Ok(_) => marking.store(true, Ordering::Release),
                Err(error) => {
                    tracing::warn!(%error, "Focus could not sort what was filed while it was closed: {error}");
                }
            }
            let database = inner.wiring.database.clone();
            let mut marked = None;
            let mut standing: Option<Vec<postio_model::ReminderId>> = None;
            let mut tick = tokio::time::interval(postio_runtime::POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let config = config.read().expect("never poisoned").clone();
                let mut surfaced_changed = false;
                match deliver_due(&database, &config, &chrono::Local::now()).await {
                    Ok(0) => {}
                    Ok(delivered) => {
                        tracing::debug!(delivered, "Focus delivered digests");
                        surfaced_changed = true;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not deliver its digests: {error}");
                    }
                }
                match fire_reminders(&database, chrono::Utc::now()).await {
                    Ok(0) => {}
                    Ok(fired) => {
                        tracing::debug!(fired, "Focus surfaced reminders");
                        surfaced_changed = true;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not fire its reminders: {error}");
                    }
                }
                // A surfaced reminder can stop standing without this timer:
                // the filing pass settles one when its reply arrives. Asked
                // of the store each tick, one seek, and said when it moved.
                if let Ok(now_standing) = standing_reminders(&database).await {
                    if standing
                        .as_ref()
                        .is_some_and(|before| *before != now_standing)
                    {
                        surfaced_changed = true;
                    }
                    standing = Some(now_standing);
                }
                if surfaced_changed {
                    inner.hub.emit(postio_core::Event::SurfacedChanged);
                }
                // What is delivered gains its summary in the background,
                // when the person's model writes them: the row never waits
                // for it (FR-142, FR-175).
                summary::summarise_in_background(&inner, &models, &config, &summarising);
                match settle_answers(&database, chrono::Utc::now()).await {
                    Ok(settled) => {
                        // The rows repaint with the answer as it now stands.
                        for (account, message) in settled {
                            inner.hub.emit(postio_core::Event::MessagesChanged {
                                account,
                                messages: vec![message],
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not settle its answers: {error}");
                    }
                }
                if let Err(error) = catch_up::keep_mark(&database, &mut marked).await {
                    tracing::warn!(%error, "Focus could not keep its mark: {error}");
                }
            }
        })
        .abort_handle()
}

/// The surfaced reminders' ids, oldest first: what the due timer compares
/// tick to tick.
async fn standing_reminders(
    database: &postio_storage::Store,
) -> Result<Vec<postio_model::ReminderId>, postio_storage::Error> {
    let reader = database.read().await?;
    Ok(postio_storage::repository::ReminderRepository::new(&reader)
        .surfaced()
        .await?
        .into_iter()
        .map(|reminder| reminder.id)
        .collect())
}
