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
use postio_ui::list_state::Waiting;

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
    /// filing and its digests -- and what the empty inbox names. A file
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
                window.set_saved_searches(saved_searches(service.config()));
            }
            if update.changed.focus {
                let focus = service.config().focus.clone();
                host.enable_focus(setup(focus.clone(), Some(service.path())));
                window.set_focus_config(focus);
            }
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Stop the engines and mark a clean end, before the host is dropped.
    pub fn stop(&self) {
        self.host.stop();
    }
}

/// The pinned saved searches `config` holds, in the order `Alt+1`-`Alt+4`
/// take them: each name as the person called it, and its query.
fn saved_searches(config: &postio_config::Config) -> Vec<(String, String)> {
    config
        .ordered_filter_keys()
        .into_iter()
        .filter_map(|key| {
            let filter = config.filters.get(&key)?;
            Some((filter.name.clone().unwrap_or(key), filter.query.clone()))
        })
        .collect()
}

/// Focus mode as `focus` says, writing its corrections to `config_path`.
fn setup(focus: postio_config::FocusConfig, config_path: Option<&std::path::Path>) -> FocusSetup {
    let setup = FocusSetup::default().with_config(focus);
    match config_path {
        Some(path) => setup.with_config_path(path.to_path_buf()),
        None => setup,
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
    // Focus's corrections (stop markers, never-filter, digest rules) are
    // written to this file: the host has to know where it is.
    let focus = host.enable_focus(setup(config.focus.clone(), config_path));
    let state = SharedState::default();
    let client = host.connect(ClientKind::Focus).with_state(state.clone());
    window.set_focus_config(config.focus.clone());
    window.set_saved_searches(saved_searches(config));
    window.set_remote_runtime(host.runtime());
    window.show_inbox(
        client.clone(),
        state.clone(),
        postio_core::Keymap::resolve(&config.keys),
    );
    Session {
        host: std::rc::Rc::new(host),
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
    /// The host over the open store, or the sentence saying why there is
    /// none.
    Done(Result<Host, String>),
}

/// Read the keyring and open the store on a thread of its own, reporting as
/// it goes. A thread, not the main loop: a keyring prompt can hold it for
/// half a minute, and the window has to go on drawing meanwhile.
pub fn open_on_a_thread(
    config_path: Option<PathBuf>,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
) -> async_channel::Receiver<Progress> {
    // Unbounded: a bounded sender would block this thread on a main loop
    // that is busy drawing.
    let (sender, receiver) = async_channel::unbounded();
    std::thread::spawn(move || {
        let report = |waiting| {
            let _ = sender.send_blocking(Progress::Stage(waiting));
        };
        let opened = Host::open(config_path.as_deref(), secrets, &report);
        let _ = sender.send_blocking(Progress::Done(opened));
    });
    receiver
}

/// Open the store behind `window`, which is already on screen, and show the
/// inbox once it is open -- or the sentence for why not, with "Try again".
///
/// `progress` is an open already under way (`open_on_a_thread`), started
/// before GTK was; `open_again` starts another, for the retry. `opened` is
/// called with the session once there is one.
pub fn open(
    window: &FocusWindow,
    progress: async_channel::Receiver<Progress>,
    config: Rc<postio_config::Config>,
    config_path: Option<PathBuf>,
    open_again: Rc<dyn Fn() -> async_channel::Receiver<Progress>>,
    opened: Rc<dyn Fn(Session)>,
) {
    let window = window.clone();
    glib::spawn_future_local(async move {
        let mut answer = Err(
            // The thread went away without answering: a bug rather than a
            // condition, but the screen still says something a person can
            // act on.
            "Postio stopped opening its local store before it answered.".to_owned(),
        );
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
                let session = adopt_at(&window, host, &config, config_path.as_deref());
                opened(session);
            }
            Err(reason) => {
                tracing::error!(reason, "the store did not open");
                let retry = {
                    let window = window.downgrade();
                    let config = Rc::clone(&config);
                    let open_again = Rc::clone(&open_again);
                    let opened = Rc::clone(&opened);
                    move || {
                        if let Some(window) = window.upgrade() {
                            open(
                                &window,
                                open_again(),
                                Rc::clone(&config),
                                config_path.clone(),
                                Rc::clone(&open_again),
                                Rc::clone(&opened),
                            );
                        }
                    }
                };
                window.show_unavailable(&reason, retry);
            }
        }
    });
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
