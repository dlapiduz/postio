//! Opening the store behind a window. One Postio at a time has the store (ADR 0041); Focus opens it
//! on a thread while its window is already on screen,
//! then starts the host and shows the inbox. Sync starts after the first frame, since startup must
//! never wait on a server.
//! If another Postio has the store, the window says so and "Try again" opens it once it is closed.

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

/// So the first pages have the runtime to themselves.
const IDLE_PASSES_AFTER_FIRST_FRAME: std::time::Duration = std::time::Duration::from_millis(750);

/// Dropping it stops the host's runtime. Stop it with [`Session::stop`] first, so the engines
/// finish their writes and the clean-shutdown mark is left.
pub struct Session {
    /// Shared only with the config follower, which holds it weakly.
    host: std::rc::Rc<Host>,
    client: Client,
    state: SharedState,
    focus: FocusHandle,
    syncing: Cell<bool>,
    agents: std::cell::RefCell<Option<postio_mcp::socket::Listener>>,
}

impl Session {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn state(&self) -> &SharedState {
        &self.state
    }

    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn syncing(&self) -> bool {
        self.syncing.get()
    }

    pub fn start_syncing(&self) {
        if self.syncing.replace(true) {
            return;
        }
        self.host.start_syncing();
        self.host
            .start_idle_passes_after(IDLE_PASSES_AFTER_FIRST_FRAME);
    }

    /// A file that does not validate changes nothing, and the last good keys stay. `false` when the
    /// file cannot be watched: edits then wait for a restart.
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
                window.set_saved_searches(postio_ui::saved_search::pinned(service.config()));
            }
            if update.changed.sync {
                host.notify_with(service.config().sync.clone());
            }
            if update.changed.focus {
                let focus = service.config().focus.clone();
                host.enable_focus(app_setup(focus.clone(), Some(service.path())));
                window.set_focus_config(host.wiring().focus.overlaid(&focus));
            }
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
            // Whichever save this was, a file that loads without error is what Settings' "Revert
            // file" goes back to.
            if service.status().is_valid()
                && let Ok(text) = std::fs::read_to_string(service.path())
            {
                window.settings_note_known_good(&text);
            }
            std::ops::ControlFlow::Continue(())
        })
    }

    pub fn stop(&self) {
        // Agents first: nothing answers them once the host has stopped.
        self.agents.borrow_mut().take();
        self.host.stop();
    }

    /// Only the app that opened the store does: a test or a demo window is no one's to answer for.
    pub fn answer_agents(&self, store: &std::path::Path, config_path: Option<&std::path::Path>) {
        match postio_mcp::socket::listen(
            postio_mcp::socket::path_for(store),
            self.host.connector(),
            config_path.map(std::path::Path::to_path_buf),
            postio_mcp::Audit::to_file(store.with_file_name("mcp-access.log")),
        ) {
            Ok(listener) => *self.agents.borrow_mut() = Some(listener),
            Err(error) => {
                tracing::warn!(%error, "agents will not be answered by this window: {error}");
            }
        }
    }
}

fn app_setup(
    focus: postio_config::FocusConfig,
    config_path: Option<&std::path::Path>,
) -> FocusSetup {
    FocusSetup::from_config(focus, config_path)
        .with_remote_images_path(postio_ui::allowlist::RemoteImageAllowList::path())
}

fn keep_treatments_in_the_store(host: &Host, client: &postio_client::Client) {
    use postio_body::treatment::Treatment;
    // A choice for one address covers the person's others: after keeping it, the host's list is
    // read again and the table follows.
    let (refreshed, hear) = async_channel::unbounded::<Vec<(String, String)>>();
    let (saver, runtime) = (client.clone(), host.runtime());
    let table = std::rc::Rc::new(postio_ui::sender_treatments::Remembered::new(
        move |sender, treatment| {
            let (client, sender, refreshed) = (saver.clone(), sender.to_owned(), refreshed.clone());
            runtime.spawn(async move {
                let word = treatment.map(Treatment::attribute_value);
                if let Err(error) = client.set_treatment(&sender, word).await {
                    tracing::warn!(%error, "could not keep a sender's treatment");
                    return;
                }
                if let Ok(all) = client.treatments().await {
                    let _ = refreshed.send(all).await;
                }
            });
        },
    ));
    // No read of its own, which the window's opening budget would count.
    table.replace(host.wiring().focus.treatments());
    let following = std::rc::Rc::clone(&table);
    glib::spawn_future_local(async move {
        while let Ok(all) = hear.recv().await {
            following.replace(all);
        }
    });
    postio_widgets::reader::use_store_treatments(Some(table));
}

fn follow_what_people_decided(host: &Host, window: &FocusWindow) {
    let (told, heard) = async_channel::unbounded::<postio_config::FocusConfig>();
    host.wiring().focus.watch(std::sync::Arc::new(move |focus| {
        let _ = told.try_send(focus.clone());
    }));
    let window = window.downgrade();
    glib::spawn_future_local(async move {
        while let Ok(focus) = heard.recv().await {
            let Some(window) = window.upgrade() else {
                break;
            };
            window.set_focus_config(focus);
        }
    });
}

/// Focus mode first, before anything could sync: the filing pass has to be in every engine before
/// its first pass. Nothing here dials a server.
pub fn adopt(window: &FocusWindow, host: Host, config: &postio_config::Config) -> Session {
    adopt_at(window, host, config, None)
}

pub fn adopt_at(
    window: &FocusWindow,
    host: Host,
    config: &postio_config::Config,
    config_path: Option<&std::path::Path>,
) -> Session {
    adopt_from(window, host, config, config_path, None)
}

pub fn adopt_from(
    window: &FocusWindow,
    host: Host,
    config: &postio_config::Config,
    config_path: Option<&std::path::Path>,
    store: Option<&std::path::Path>,
) -> Session {
    let timeline = window.timeline();
    if let Some(timeline) = &timeline {
        timeline.mark(Phase::Account);
    }
    // The host has to know where the file is: Focus's corrections are written to it.
    let focus = host.enable_focus(app_setup(config.focus.clone(), config_path));
    let host = std::rc::Rc::new(host);
    let state = SharedState::default();
    let client = host.connect(ClientKind::Focus).with_state(state.clone());
    // What people decided about senders lives in the store (ADR 0048): the reader looks treatments
    // up there.
    keep_treatments_in_the_store(&host, &client);
    follow_what_people_decided(&host, window);
    window.set_focus_config(
        host.wiring()
            .focus
            .config()
            .unwrap_or_else(|| config.focus.clone()),
    );
    window.set_saved_searches(postio_ui::saved_search::pinned(config));
    window.set_compose_config(config.compose.clone());
    window.set_reader_config(&config.reader);
    window.set_remote_runtime(host.runtime());
    window.set_config_path(config_path.map(std::path::Path::to_path_buf));
    window.set_settings_seams(crate::settings::Seams {
        runtime: host.runtime(),
        secrets: host.wiring().secrets.clone(),
        attachments_eager: config.sync.attachment_fetch
            == postio_config::sync::AttachmentFetch::Eager,
        agent_activity: {
            let database = host.wiring().database.clone();
            let store = store.map(std::path::Path::to_path_buf);
            std::sync::Arc::new(move || {
                let database = database.clone();
                let store = store.clone();
                Box::pin(async move {
                    match store {
                        Some(store) => {
                            postio_session::agent_access::activity_beside(Some(&database), &store)
                                .await
                        }
                        // A store in memory has no log beside it.
                        None => {
                            postio_session::agent_access::activity(None, "", chrono::Utc::now())
                                .await
                        }
                    }
                })
            })
        },
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
        agents: std::cell::RefCell::new(None),
    }
}

/// One channel, so a stage can never arrive after the answer and put the plate back over mail.
pub enum Progress {
    Stage(Waiting),
    Done(Result<Host, Refusal>),
}

#[derive(Clone)]
pub struct Opener {
    config_path: Option<PathBuf>,
    store: PathBuf,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
}

impl Opener {
    pub fn new(
        config_path: Option<PathBuf>,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Opener::at(config_path, postio_session::paths::store_path(), secrets)
    }

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

    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// A thread, not the main loop: a keyring prompt can hold it for half a minute.
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

    /// On a thread for the same reason the open is.
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

pub fn open_on_a_thread(
    config_path: Option<PathBuf>,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
) -> async_channel::Receiver<Progress> {
    Opener::new(config_path, secrets).open_on_a_thread()
}

/// `progress` is an open already under way, started before GTK; `opener` starts another, for the
/// retry.
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
                let session = adopt_from(
                    &window,
                    host,
                    &config,
                    opener.config_path(),
                    Some(&opener.store),
                );
                session.answer_agents(&opener.store, opener.config_path());
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
                    // Trying again would meet the same file or the same key (#1598).
                    postio_session::Remedy::StartOver { because, .. } => {
                        let page = window.downgrade();
                        window.show_start_over(because, move || {
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

pub fn time(window: &FocusWindow, timeline: Timeline) {
    timeline.mark(Phase::Window);
    let shell = timeline.clone();
    postio_widgets::startup::on_first_frame(window, move || shell.mark(Phase::Shell));
    window.set_timeline(timeline);
}

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
