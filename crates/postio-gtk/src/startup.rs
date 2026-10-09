//! Opening the store behind a window, and what Focus holds once it is open.
//!
//! One Postio at a time has the store (ADR 0041). Focus opens it itself, on a
//! thread, while its window is already on screen and says what it waits for;
//! then it starts the store's host in this process, turns Focus mode on in
//! it, connects to it as a client, and shows the inbox. Sync starts after the
//! first frame, because the mail is already on disk and startup must never
//! wait on a server (US1 scenario 1).
//!
//! If another Postio has the store, the window says so in the sentence every
//! app uses, and "Try again" opens it once it has been closed (FR-003).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_core::SharedState;
use postio_host::{FocusHandle, FocusSetup, Host};
use postio_session::Refusal;
use postio_ui::list_state::Waiting;
use postio_widgets::startup::{Phase, Timeline};

use crate::window::FocusWindow;

/// How long after the first frame the store's upkeep starts -- the body
/// index, the header repair, the disk reclaim -- so the first pages have the
/// runtime to themselves. The desktop app's and the terminal's delay.
const IDLE_PASSES_AFTER_FIRST_FRAME: std::time::Duration = std::time::Duration::from_millis(750);

/// The store's host and Focus's hold on it, for as long as the window lives.
///
/// Dropping it stops the host's runtime. Stop it with [`Session::stop`] first,
/// as the app does on its way out, so the engines finish their writes and the
/// clean-shutdown mark is left.
pub struct Session {
    /// Shared only with the config follower, which holds it weakly: the
    /// session is still what keeps the host, and dropping it stops it.
    host: std::rc::Rc<Host>,
    client: Client,
    state: SharedState,
    focus: FocusHandle,
    syncing: Cell<bool>,
}

impl Session {
    /// The client the window reads and writes through.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// What the host aims this window's commands with: its selection.
    pub fn state(&self) -> &SharedState {
        &self.state
    }

    /// Focus mode in the host: whether its tasks are running.
    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// The host itself.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// Whether sync has been started.
    pub fn syncing(&self) -> bool {
        self.syncing.get()
    }

    /// Bring every account's connection up, and the store's upkeep a moment
    /// later. Once: a second call does nothing.
    pub fn start_syncing(&self) {
        if self.syncing.replace(true) {
            return;
        }
        self.host.start_syncing();
        self.host
            .start_idle_passes_after(IDLE_PASSES_AFTER_FIRST_FRAME);
    }

    /// Follow `config.toml` at `path` while the window lives (US7 scenario
    /// 1): a saved `[keys]` reaches the keyboard, every keycap and the key
    /// map at once; a saved `[focus]` reaches the host's Focus mode -- its
    /// filing and its digests -- and what the empty inbox names; a saved
    /// `[compose]` places the next signature, `[reader]` zooms the open
    /// message, and `[storage]` brings the store under its new ceiling
    /// (T235). A file
    /// that does not validate changes nothing, and the last good keys stay
    /// (`postio_widgets::present::config`). `false` when the file cannot be
    /// watched: edits then wait for a restart.
    pub fn follow_config(&self, window: &FocusWindow, path: &std::path::Path) -> bool {
        let service = postio_core::config::ConfigService::load(path);
        let window = window.downgrade();
        let host = std::rc::Rc::downgrade(&self.host);
        postio_widgets::present::config::follow(service, move |service, update| {
            let (Some(window), Some(host)) = (window.upgrade(), host.upgrade()) else {
                return std::ops::ControlFlow::Break(());
            };
            if update.changed.keys {
                window.set_keymap(service.keymap().clone());
            }
            if update.changed.filters {
                window.set_saved_searches(postio_session::focus::saved_searches(service.config()));
            }
            if update.changed.sync {
                host.notify_with(service.config().sync.clone());
            }
            if update.changed.focus {
                let focus = service.config().focus.clone();
                host.enable_focus(FocusSetup::from_config(focus.clone(), Some(service.path())));
                window.set_focus_config(focus);
            }
            // What used to wait for a restart (T235).
            if update.changed.compose {
                window.set_compose_config(service.config().compose.clone());
            }
            if update.changed.reader {
                window.set_reader_config(&service.config().reader);
            }
            if update.changed.storage {
                postio_host::maintenance::enforce_ceiling(
                    host.wiring(),
                    service.config().storage.max_bytes,
                );
            }
            // Whichever save this was -- Settings' own, or the editor's --
            // a file that loads without error is what Settings' "Revert
            // file" goes back to.
            if service.status().is_valid()
                && let Ok(text) = std::fs::read_to_string(service.path())
            {
                window.settings_note_known_good(&text);
            }
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Stop the engines and mark a clean end, before the host is dropped.
    pub fn stop(&self) {
        self.host.stop();
    }
}

/// Take a host over an open store and show its inbox in `window`.
///
/// Focus mode first, before anything could sync: the filing pass has to be
/// in every engine before its first pass (contracts/engine.md). Then Focus
/// connects as the client it is, and the window reads through it. Nothing
/// here dials a server; [`Session::start_syncing`] does, after the first
/// frame.
pub fn adopt(window: &FocusWindow, host: Host, config: &postio_config::Config) -> Session {
    adopt_at(window, host, config, None)
}

/// [`adopt`], for a store opened under the `config.toml` at `config_path`.
pub fn adopt_at(
    window: &FocusWindow,
    host: Host,
    config: &postio_config::Config,
    config_path: Option<&std::path::Path>,
) -> Session {
    let timeline = window.timeline();
    if let Some(timeline) = &timeline {
        timeline.mark(Phase::Account);
    }
    // Focus's corrections (stop markers, never-filter, digest rules) are
    // written to this file: the host has to know where it is.
    let focus = host.enable_focus(FocusSetup::from_config(config.focus.clone(), config_path));
    let host = std::rc::Rc::new(host);
    let state = SharedState::default();
    let client = host.connect(ClientKind::Focus).with_state(state.clone());
    window.set_focus_config(config.focus.clone());
    window.set_saved_searches(postio_session::focus::saved_searches(config));
    window.set_compose_config(config.compose.clone());
    window.set_reader_config(&config.reader);
    window.set_remote_runtime(host.runtime());
    window.set_config_path(config_path.map(std::path::Path::to_path_buf));
    // What Settings' connection test and token-expiry line read, from this
    // process (T234).
    window.set_settings_seams(crate::settings::Seams {
        runtime: host.runtime(),
        secrets: host.wiring().secrets.clone(),
        attachments_eager: config.sync.attachment_fetch
            == postio_config::sync::AttachmentFetch::Eager,
    });
    // Focus's notifications follow the `[sync]` settings,
    // and are only ever about mail that stayed in its inbox (FR-153).
    host.notify_with(config.sync.clone());
    window.set_notifier({
        // Weakly: the session owns the host, and a notification decided
        // after it has stopped is none.
        let deciding = std::rc::Rc::downgrade(&host);
        std::rc::Rc::new(
            move |mailbox, messages, attention| match deciding.upgrade() {
                Some(host) => Box::pin(host.focus_notification(mailbox, messages, attention)),
                None => Box::pin(std::future::ready(None)),
            },
        )
    });
    // What a first run with no account brings up once one is saved (T171):
    // the same `start_syncing` `Session::start_syncing` calls after the
    // first frame, reachable from the window that just gained an account
    // rather than through the session it does not hold.
    window.set_start_syncing({
        let host = std::rc::Rc::downgrade(&host);
        std::rc::Rc::new(move || {
            if let Some(host) = host.upgrade() {
                host.start_syncing();
            }
        })
    });
    window.show_inbox(
        client.clone(),
        state.clone(),
        postio_core::Keymap::resolve(&config.keys),
    );
    // The inbox is fed; the frame after this is the one with mail in it.
    if let Some(timeline) = &timeline {
        timeline.mark(Phase::Feeds);
        postio_widgets::startup::report_usable(window, timeline);
    }
    Session {
        host,
        client,
        state,
        focus,
        syncing: Cell::new(false),
    }
}

/// What the opening thread says, in the order it says it: one channel, so a
/// stage can never arrive after the answer and put the plate back over mail.
pub enum Progress {
    /// What the store is being waited on for now.
    Stage(Waiting),
    /// The host over the open store, or why there is none and what gets
    /// past it.
    Done(Result<Host, Refusal>),
}

/// How Focus opens its store: under which `config.toml`, where the store
/// is, and the keyring its key is in. Opening again and starting over are
/// both done through it, so neither can reach a different store.
#[derive(Clone)]
pub struct Opener {
    config_path: Option<PathBuf>,
    store: PathBuf,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
}

impl Opener {
    /// This installation's store, under the `config.toml` at `config_path`.
    pub fn new(
        config_path: Option<PathBuf>,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Opener::at(config_path, postio_session::paths::store_path(), secrets)
    }

    /// The store at `store`: a suite's, one per case.
    pub fn at(
        config_path: Option<PathBuf>,
        store: PathBuf,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Opener {
            config_path,
            store,
            secrets,
        }
    }

    /// The `config.toml` this store is opened under.
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// Read the keyring and open the store on a thread of its own,
    /// reporting as it goes. A thread, not the main loop: a keyring prompt
    /// can hold it for half a minute, and the window has to go on drawing
    /// meanwhile.
    pub fn open_on_a_thread(&self) -> async_channel::Receiver<Progress> {
        // Unbounded: a bounded sender would block this thread on a main loop
        // that is busy drawing.
        let (sender, receiver) = async_channel::unbounded();
        let opener = self.clone();
        std::thread::spawn(move || {
            let report = |waiting| {
                let _ = sender.send_blocking(Progress::Stage(waiting));
            };
            let opened = Host::open_at(
                opener.config_path.as_deref(),
                &opener.store,
                opener.secrets,
                &report,
            );
            let _ = sender.send_blocking(Progress::Done(opened));
        });
        receiver
    }

    /// Set the store aside and start a fresh one in its place, carrying the
    /// accounts across (`postio_session::start_over`), on a thread of its
    /// own for the same reason the open is. Answers where the old one went,
    /// or the sentence saying why it could not be done.
    pub fn start_over_on_a_thread(
        &self,
    ) -> async_channel::Receiver<Result<postio_session::start_over::StartedOver, String>> {
        let (sender, receiver) = async_channel::bounded(1);
        let opener = self.clone();
        std::thread::spawn(move || {
            let started = postio_session::store_key_blocking(opener.secrets.as_ref())
                .map_err(|error| error.to_string())
                .and_then(|key| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("Postio could not start the worker that starts the store over: {error}")
                        })?
                        .block_on(postio_session::start_over::start_over_at(&opener.store, &key))
                });
            let _ = sender.send_blocking(started);
        });
        receiver
    }
}

/// [`Opener::open_on_a_thread`] for this installation's store.
pub fn open_on_a_thread(
    config_path: Option<PathBuf>,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
) -> async_channel::Receiver<Progress> {
    Opener::new(config_path, secrets).open_on_a_thread()
}

/// Open the store behind `window`, which is already on screen, and show the
/// inbox once it is open -- or the sentence for why not, with the way past
/// it: "Try again" for what can pass, starting over for a store no
/// migration reaches.
///
/// `progress` is an open already under way (`Opener::open_on_a_thread`),
/// started before GTK was; `opener` starts another, for the retry, and
/// starts the store over. `opened` is called with the session once there
/// is one.
pub fn open(
    window: &FocusWindow,
    progress: async_channel::Receiver<Progress>,
    config: Rc<postio_config::Config>,
    opener: Opener,
    opened: Rc<dyn Fn(Session)>,
) {
    let window = window.clone();
    glib::spawn_future_local(async move {
        let mut answer = Err(Refusal::try_again(
            // The thread went away without answering: a bug rather than a
            // condition, but the screen still says something a person can
            // act on.
            "Postio stopped opening its local store before it answered.",
        ));
        while let Ok(said) = progress.recv().await {
            match said {
                Progress::Stage(waiting) => window.set_waiting_on(waiting),
                Progress::Done(done) => {
                    answer = done;
                    break;
                }
            }
        }
        match answer {
            Ok(host) => {
                if let Some(timeline) = window.timeline() {
                    timeline.mark(Phase::Store);
                }
                let session = adopt_at(&window, host, &config, opener.config_path());
                opened(session);
            }
            Err(refusal) => {
                tracing::error!(reason = %refusal, "the store did not open");
                let retry = {
                    let window = window.downgrade();
                    let config = Rc::clone(&config);
                    let opener = opener.clone();
                    let opened = Rc::clone(&opened);
                    move || {
                        if let Some(window) = window.upgrade() {
                            open(
                                &window,
                                opener.open_on_a_thread(),
                                Rc::clone(&config),
                                opener.clone(),
                                Rc::clone(&opened),
                            );
                        }
                    }
                };
                match refusal.remedy {
                    postio_session::Remedy::TryAgain => {
                        window.show_unavailable(&refusal.sentence, retry);
                    }
                    // Trying again would meet the same file (T215).
                    postio_session::Remedy::StartOver { .. } => {
                        let page = window.downgrade();
                        window.show_start_over(move || {
                            if let Some(window) = page.upgrade() {
                                start_over(&window, &opener, Rc::new(retry.clone()));
                            }
                        });
                    }
                }
            }
        }
    });
}

/// Set the store aside and start a fresh one (`Opener::start_over_on_a_thread`),
/// then open it as `open_fresh` does: what "Start a fresh store" runs. A start
/// over that could not be done says why, with "Try again" -- which opens the
/// store again and, if it is still the old one, offers this again.
fn start_over(window: &FocusWindow, opener: &Opener, open_fresh: Rc<dyn Fn()>) {
    window.show_starting_over();
    let started = opener.start_over_on_a_thread();
    let window = window.downgrade();
    glib::spawn_future_local(async move {
        let answer = started.recv().await.unwrap_or_else(|_| {
            Err("Postio stopped starting a fresh store before it answered.".to_owned())
        });
        let Some(window) = window.upgrade() else {
            return;
        };
        match answer {
            Ok(started) => {
                tracing::info!(accounts = started.accounts, "the store was started over");
                open_fresh();
                window.say(&postio_ui::focus_state::started_over(
                    &started.set_aside.display().to_string(),
                ));
            }
            Err(sentence) => {
                tracing::error!(reason = %sentence, "the store could not be started over");
                window.show_unavailable(&sentence, move || open_fresh());
            }
        }
    });
}

/// Measure `window`'s start on `timeline`, against the 500 ms budget: the
/// window exists now, and its first frame is the shell. [`open`] marks the
/// store, [`adopt_at`] the inbox it feeds, and the frame after that closes
/// the timeline (`postio_widgets::startup::report_usable`, which also reads
/// `POSTIO_STARTUP_TRACE` and `POSTIO_STARTUP_EXIT`).
pub fn time(window: &FocusWindow, timeline: Timeline) {
    timeline.mark(Phase::Window);
    let shell = timeline.clone();
    postio_widgets::startup::on_first_frame(window, move || shell.mark(Phase::Shell));
    window.set_timeline(timeline);
}

/// Run `then` once, after `widget`'s first frame has been drawn: "once the
/// person can see their mail", not "when the loop is idle".
pub fn after_first_frame(widget: &impl IsA<gtk::Widget>, then: impl FnOnce() + 'static) {
    let then = std::cell::RefCell::new(Some(then));
    let ticks = Cell::new(0u8);
    widget.add_tick_callback(move |_, _| {
        // The first tick is before the first frame is drawn; the second is
        // after it.
        ticks.set(ticks.get() + 1);
        if ticks.get() < 2 {
            return glib::ControlFlow::Continue;
        }
        if let Some(then) = then.borrow_mut().take() {
            then();
        }
        glib::ControlFlow::Break
    });
}
