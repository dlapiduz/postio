//! A window whose store is another process's: `postio-daemon`'s.
//!
//! This is how the desktop app runs (ADR 0041). The daemon owns the store,
//! the keyring's store key, the engines and the upkeep; the window reaches
//! all of it through one [`Client`] over the daemon's socket, so the desktop
//! app and the terminal can be open on one store at once. Nothing here can
//! open the store, and nothing here waits on the network: every call is a
//! channel receive, answered by the daemon from what it has on disk.
//!
//! The integration suites drive the same surfaces over an owner in their
//! own process instead (`feed_the_window` and friends), and
//! `app_suite::daemon_window` drives this module over a socket.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::{ClientKind, StartupRoute};
use postio_client::socket::{ConnectError, Endpoint};
use postio_core::CommandId;
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_model::listing::MailStore;
use postio_ui::list_state::Waiting;

use crate::frontend::Frontend;

/// A window's connection to the store's owner, once it answers.
#[derive(Clone)]
pub struct Connected {
    /// What every surface holds.
    pub frontend: Frontend,
    /// The verbs the owner answers.
    pub wired: Vec<CommandId>,
    /// What the window hears: the owner's news and its own sentences.
    /// Taken by the one drain.
    pub events: Rc<RefCell<Option<EventStream>>>,
}

/// What reaching the owner has to say, in order.
pub enum Reaching {
    /// The owner is there and still opening the store.
    Waiting(Waiting),
    /// Connected, or the sentence saying why not.
    Done(Result<Client, String>),
}

/// Reach the owner at `endpoint` on a thread of its own, starting `daemon`
/// if nothing answers, and report as it goes.
///
/// A thread because connecting blocks -- on a socket, and on the backoff
/// while a daemon just started opens its store, which a keyring prompt has
/// held for half a minute -- and the main loop must be free to draw the
/// whole time. The window says "Opening your mailbox" if the owner says it
/// is still opening; the list pane's threshold keeps a quick start silent.
pub fn reach(endpoint: Endpoint, daemon: std::path::PathBuf) -> async_channel::Receiver<Reaching> {
    // Unbounded: a bounded send would block this thread on a main loop that
    // is busy drawing.
    let (sender, receiver) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("postio-connect".to_owned())
        .spawn(move || {
            let reached = postio_client::socket::connect_or_start(
                &endpoint,
                ClientKind::Gtk,
                &daemon,
                &mut |opening| {
                    let _ = sender.send_blocking(Reaching::Waiting(Waiting::from(opening)));
                },
            );
            let _ = sender.send_blocking(Reaching::Done(
                reached.map_err(|error: ConnectError| error.to_string()),
            ));
        })
        .map_or_else(
            |error| {
                let (sender, receiver) = async_channel::bounded(1);
                let _ = sender.try_send(Reaching::Done(Err(format!(
                    "Postio could not start the worker that reaches its background service: \
                     {error}"
                ))));
                receiver
            },
            |_| receiver,
        )
}

/// The owner's endpoint and the daemon to start, from this user's runtime
/// directory and this executable's.
pub fn reach_this_users_daemon() -> async_channel::Receiver<Reaching> {
    match Endpoint::from_env() {
        Ok(endpoint) => reach(endpoint, postio_client::socket::daemon_path()),
        Err(error) => {
            let (sender, receiver) = async_channel::bounded(1);
            let _ = sender.try_send(Reaching::Done(Err(error.to_string())));
            receiver
        }
    }
}

/// Everything a window holds over `client`, its calls awaited on `runtime`.
///
/// `state` is the window's own selection, sent with each command so the
/// owner aims it at what is on this screen. The window's gestures go to the
/// owner in the order they are made, and what the owner says reaches the
/// window's one event queue beside the window's own sentences.
pub async fn connected(
    client: Client,
    runtime: tokio::runtime::Handle,
    state: SharedState,
    attachments_eager: bool,
) -> Connected {
    let client = client.with_state(state);
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the owner answers
    // on its own runtime.
    let wired = client.wired().await.unwrap_or_else(|error| {
        tracing::error!(%error, "cannot ask which verbs the owner answers: {error}");
        Vec::new()
    });

    let (commands, queued) = postio_core::bridge::command_channel();
    runtime.spawn({
        let client = client.clone();
        async move {
            while let Some(command) = queued.recv().await {
                // A gesture made while the owner is gone is not taken; the
                // next one, after a reconnect, goes to the new owner.
                if let Err(error) = client.send(command).await {
                    tracing::warn!(%error, "a command was not taken: {error}");
                }
            }
        }
    });
    let (sink, stream) = postio_core::bridge::event_channel();
    forward_events(&client, &sink, &runtime);

    Connected {
        frontend: Frontend {
            egress: client.egress(),
            client,
            runtime,
            events: sink,
            // The account passwords this process reads itself -- the
            // connection test, the token-expiry line -- never the store key,
            // which only the owner reads.
            secrets: Arc::new(postio_account::secret::KeyringSecretStore::default()),
            commands,
            attachments_eager,
            wiring: None,
        },
        wired,
        events: Rc::new(RefCell::new(Some(stream))),
    }
}

/// Carry what the owner says to `client` into the window's event queue,
/// until the connection it has now ends.
fn forward_events(
    client: &Client,
    sink: &postio_core::bridge::EventSink,
    runtime: &tokio::runtime::Handle,
) {
    let arriving = client.events();
    let sink = sink.clone();
    runtime.spawn(async move {
        while let Ok(envelope) = arriving.recv().await {
            if !sink.emit(envelope.event) {
                return;
            }
        }
    });
}

/// What the window says when the owner goes away under it.
pub const STOPPED: &str = "Postio's background service stopped. Your mail is where it was; \
                           try again to reconnect.";

/// Put the unavailable screen up once the owner the window reaches has
/// gone: the daemon exited, or the connection broke. "Try again" reaches it
/// the way startup does, starting it if nothing answers -- only when asked:
/// nothing here reconnects on its own.
fn watch_for_the_owner_going(window: &Window, following: &Rc<Following>) {
    let Some(client) = following
        .reached
        .borrow()
        .as_ref()
        .map(|connected| connected.frontend.client.clone())
    else {
        return;
    };
    let closed = client.closed();
    let window = window.downgrade();
    let following = Rc::clone(following);
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: a channel receive; the connection's own thread
        // closes it.
        closed.await;
        let Some(window) = window.upgrade() else {
            return;
        };
        tracing::warn!("the background service went away");
        {
            let mut previous = following.previous.borrow_mut();
            if previous.is_none() {
                *previous = window.content();
            }
        }
        unreachable(&window, STOPPED, {
            let window = window.clone();
            move |screen| {
                screen.set_busy(true);
                follow(&window, (following.again)(), Rc::clone(&following));
            }
        });
    });
}

/// The window is over a new owner: take its connection into every clone of
/// the old client, and have the sidebar, the list and the reader read
/// again from it.
async fn reconnected(window: &Window, connected: &Connected, client: &Client, state: &SharedState) {
    let frontend = &connected.frontend;
    frontend.client.reconnect(client);
    forward_events(&frontend.client, &frontend.events, &frontend.runtime);
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the owner answers
    // on its own runtime.
    let accounts = frontend.client.accounts().await.unwrap_or_default();
    let (showing, focus) = state.read(|app| (app.mailbox(), app.focus()));
    for account in accounts {
        let account = account.id;
        frontend
            .events
            .emit(postio_core::Event::MailboxesChanged { account });
        // POSTIO-GLIB-SAFE: as above.
        let folders = frontend.client.mailboxes(account).await.unwrap_or_default();
        for folder in folders {
            frontend
                .events
                .emit(postio_core::Event::MessageListChanged {
                    account,
                    mailbox: folder.id,
                });
            if Some(folder.id) == showing
                && let Some(message) = focus
            {
                frontend
                    .events
                    .emit(postio_core::Event::BodyLoaded { account, message });
            }
        }
    }
    window.report_usable();
}

/// Open the window over the owner: its account, or the first-run screen.
///
/// The owner decides which (`StartupRoute`), by the rule the desktop app
/// used when it owned the store. `fed` makes a second `activate` -- a
/// second launch raising the window -- a no-op, as `open_or_onboard`'s does.
pub async fn open(
    window: &Window,
    connected: &Connected,
    state: SharedState,
    fed: &Rc<Cell<bool>>,
) {
    if fed.replace(true) {
        return;
    }
    // The mail is behind the window from here on, so every command that
    // reads or writes it becomes available (#1114).
    window.set_store_open(true);
    let frontend = connected.frontend.clone();
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the owner answers
    // on its own runtime.
    let route = frontend
        .client
        .startup_route()
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "the owner could not say where to start: {error}");
            StartupRoute::Onboard(None)
        });
    let notifier = crate::notifications::Notifier::from_the_owner();
    match route {
        StartupRoute::Ready(_) => {
            crate::open_account_for(
                window,
                &frontend,
                &state,
                &connected.wired,
                &connected.events,
                &notifier,
            )
            .await;
        }
        StartupRoute::Onboard(repairing) => {
            let open = {
                let window = window.clone();
                let frontend = frontend.clone();
                let wired = connected.wired.clone();
                let events = Rc::clone(&connected.events);
                move || {
                    let (window, frontend, state, wired, events, notifier) = (
                        window.clone(),
                        frontend.clone(),
                        state.clone(),
                        wired.clone(),
                        Rc::clone(&events),
                        notifier.clone(),
                    );
                    glib::spawn_future_local(async move {
                        let opening = crate::open_account_for(
                            &window, &frontend, &state, &wired, &events, &notifier,
                        );
                        // POSTIO-GLIB-SAFE: every await under this is a
                        // client call, a oneshot receive the owner answers
                        // on its own runtime.
                        opening.await;
                    });
                }
            };
            crate::onboarding::install_for(
                window,
                &frontend,
                repairing.map(|account| *account),
                // Discovery probes are outbound connections too (#151),
                // recorded in the owner's log.
                Arc::new(
                    postio_account::discovery::PimalayaTransport::new()
                        .with_egress(frontend.egress.clone()),
                ),
                Arc::new(postio_account::oauth::browser::SystemBrowserOpener),
                open,
            )
            .await;
        }
    }
    // Both branches are a usable UI: mail to read, or the screen that asks
    // for the account there is none of.
    window.report_usable();
}

/// Say that the owner could not be reached, and offer to try again.
///
/// The screen the desktop app showed when it could not open its store
/// (#404, ADR 0014 Q3): a hard stop with one action, under the window's
/// chrome so it can be closed. `retry` is what the button does.
pub fn unreachable(
    window: &Window,
    reason: &str,
    retry: impl Fn(&postio_gtk::unavailable::Unavailable) + 'static,
) {
    let screen = postio_gtk::unavailable::Unavailable::new();
    screen.set_reason(reason);
    window.set_content(Some(&postio_gtk::widgets::under_window_chrome(&screen)));
    screen.focus_retry();
    screen.connect_retry({
        let screen = screen.clone();
        move || retry(&screen)
    });
    window.report_usable();
}

/// What following a connection needs, however many retries it takes.
pub struct Following {
    /// Where the window's client calls are awaited.
    pub runtime: tokio::runtime::Handle,
    /// The window's own selection.
    pub state: SharedState,
    /// `[sync] attachments = "eager"`.
    pub attachments_eager: bool,
    /// The connection, once there is one; held for the life of the window.
    pub reached: Rc<RefCell<Option<Connected>>>,
    /// Whether the window has been opened over it.
    pub fed: Rc<Cell<bool>>,
    /// How to reach the owner again, for the retry.
    pub again: Box<dyn Fn() -> async_channel::Receiver<Reaching>>,
    /// Called once connected, before anything is opened.
    pub on_connected: Box<dyn Fn()>,
    /// The window's content before an unreachable screen replaced it, so a
    /// retry that succeeds hands the window back to its panes.
    pub previous: RefCell<Option<gtk::Widget>>,
}

/// Follow `reaching` to the end: say what is being waited on, then open the
/// window over the owner, or say why it could not be reached and offer to
/// try again.
pub fn follow(
    window: &Window,
    reaching: async_channel::Receiver<Reaching>,
    following: Rc<Following>,
) {
    let window = window.clone();
    glib::spawn_future_local(async move {
        let mut answer =
            Err("Postio stopped reaching its background service before it answered.".to_owned());
        while let Ok(reaching) = reaching.recv().await {
            match reaching {
                Reaching::Waiting(waiting) => window.set_waiting_on(waiting),
                Reaching::Done(done) => {
                    answer = done;
                    break;
                }
            }
        }
        match answer {
            Ok(client) => {
                (following.on_connected)();
                tracing::info!("connected to the background service");
                if let Some(previous) = following.previous.borrow_mut().take() {
                    window.set_content(Some(&previous));
                }
                // Once the window has been opened over an owner, a new one
                // is taken into the connection the panes already hold.
                let already = following.reached.borrow().clone();
                if let Some(already) = already {
                    // POSTIO-GLIB-SAFE: every await under this is a client
                    // call, a oneshot receive the owner answers on its own
                    // runtime.
                    reconnected(&window, &already, &client, &following.state).await;
                    watch_for_the_owner_going(&window, &following);
                    return;
                }
                let connecting = connected(
                    client,
                    following.runtime.clone(),
                    following.state.clone(),
                    following.attachments_eager,
                );
                // POSTIO-GLIB-SAFE: the one await under this is a client
                // call, a oneshot receive the owner answers on its own
                // runtime; the forwarding is spawned onto `runtime`.
                let ready = connecting.await;
                *following.reached.borrow_mut() = Some(ready.clone());
                // POSTIO-GLIB-SAFE: every await under this is a client call,
                // a oneshot receive the owner answers on its own runtime.
                open(&window, &ready, following.state.clone(), &following.fed).await;
                watch_for_the_owner_going(&window, &following);
            }
            Err(reason) => {
                // Safe verbatim: no connect error carries mail or a key.
                tracing::error!(reason, "the background service could not be reached");
                {
                    let mut previous = following.previous.borrow_mut();
                    if previous.is_none() {
                        *previous = window.content();
                    }
                }
                unreachable(&window, &reason, {
                    let window = window.clone();
                    move |screen| {
                        screen.set_busy(true);
                        follow(&window, (following.again)(), Rc::clone(&following));
                    }
                });
            }
        }
    });
}
