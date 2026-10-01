//! The application: its id, its one window, and the order things start in.
//!
//! The store starts opening on a thread before GTK does anything, as the
//! desktop app's does (#1604): the keyring and the engine's open of an
//! encrypted file need nothing from the window. The window then comes up at
//! once and says what it waits for if the wait is long; the inbox fills from
//! the store; and sync starts after the first frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::startup::{self, Session};
use crate::window::FocusWindow;

/// Focus's application id: a name inside the desktop app's own namespace,
/// which a sandboxed app may own (research R3).
pub const APP_ID: &str = "dev.postio.Postio.Focus";

/// Focus's application, as `run` starts it.
pub fn application() -> adw::Application {
    // Tell the compositor which application this is: GNOME matches a
    // window to its desktop entry by the Wayland `app_id`, which GDK takes
    // from the program name -- the binary's, `postio-focus`, unless it is
    // set. Focus's entry is `dev.postio.Postio.Focus.desktop`, and its
    // `StartupWMClass` names the same id (postio-gtk's `app.rs` has the
    // history).
    glib::set_prgname(Some(APP_ID));
    adw::Application::builder()
        .application_id(APP_ID)
        // The desktop entry registers `x-scheme-handler/postio` with `%U`,
        // so a `postio://` link clicked elsewhere arrives as a file to
        // open; without this flag GApplication drops it (T159).
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build()
}

/// The whole program: open the store, show the inbox, run until closed.
pub fn run() -> glib::ExitCode {
    let config_path = postio_config::paths::config_path().ok();
    // Before anything else can have anything to say: a store that will not
    // open and a keyring that will not answer both happen before there is
    // any UI to report them in.
    let logging = postio_session::logging::init(
        &config_path
            .as_deref()
            .map(postio_session::logging::config_at)
            .unwrap_or_default(),
    );
    // Held for the life of the process: dropping it stops the watch, and
    // `[logging]` exists to retune a running Postio.
    let _log_watch = config_path.as_deref().and_then(|path| logging.watch(path));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio-focus starting");

    let config = Rc::new(
        config_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
            .unwrap_or_default(),
    );

    // One keyring for the installation, read on the opening thread.
    let secrets: Arc<dyn postio_account::secret::SecretStore> =
        Arc::new(postio_account::secret::KeyringSecretStore::default());
    let open_again: Rc<dyn Fn() -> async_channel::Receiver<startup::Progress>> = {
        let config_path = config_path.clone();
        let secrets = Arc::clone(&secrets);
        Rc::new(move || startup::open_on_a_thread(config_path.clone(), Arc::clone(&secrets)))
    };
    // Before GTK: the open overlaps the whole of GTK's own start.
    let early = RefCell::new(Some(open_again()));

    if adw::init().is_err() {
        // Name what GTK was given: an empty pair is a terminal outside the
        // desktop session (a tmux or ssh shell), which is not a broken build.
        let named = |key: &str| std::env::var(key).unwrap_or_default();
        tracing::error!(
            wayland_display = %named("WAYLAND_DISPLAY"),
            display = %named("DISPLAY"),
            "no display; Focus needs a Wayland or X11 session (set WAYLAND_DISPLAY or DISPLAY)"
        );
        return glib::ExitCode::FAILURE;
    }

    let session: Rc<RefCell<Option<Session>>> = Rc::default();
    let application = application();
    application.connect_activate({
        let session = Rc::clone(&session);
        move |application| {
            // A second launch raises the window it already has.
            if let Some(window) = application.active_window() {
                window.present();
                return;
            }
            let window = FocusWindow::new(Some(application));
            window.present();
            let progress = early.borrow_mut().take().unwrap_or_else(|| open_again());
            let opened: Rc<dyn Fn(Session)> = {
                let session = Rc::clone(&session);
                let window = window.downgrade();
                let config_path = config_path.clone();
                Rc::new(move |opened: Session| {
                    // `[keys]` and `[focus]` apply while Focus runs (T060).
                    if let (Some(path), Some(window)) = (config_path.as_deref(), window.upgrade()) {
                        opened.follow_config(&window, path);
                    }
                    let session = Rc::clone(&session);
                    session.replace(Some(opened));
                    if let Some(window) = window.upgrade() {
                        // The network after the frame the stored mail is
                        // drawn in, never before it.
                        let warm = window.downgrade();
                        startup::after_first_frame(&window, move || {
                            if let Some(session) = session.borrow().as_ref() {
                                session.start_syncing();
                            }
                            // The composer's web process, while nobody is
                            // waiting on it (#1216).
                            if let Some(window) = warm.upgrade() {
                                window.warm_composer();
                            }
                        });
                    }
                })
            };
            startup::open(
                &window,
                progress,
                Rc::clone(&config),
                config_path.clone(),
                Rc::clone(&open_again),
                opened,
            );
        }
    });

    // A `postio://` link: the window first, as a launch would bring it,
    // then the message the link names -- gone to, never acted on.
    application.connect_open(|application, files, _| {
        application.activate();
        let window = application.active_window().and_downcast::<FocusWindow>();
        if let Some(window) = window {
            for file in files {
                window.open_link(&file.uri());
            }
        }
    });

    // The command line's only arguments are links to open.
    let code = application.run();

    // The engines first, then the clean-shutdown mark, before the host's
    // runtime goes with it.
    if let Some(session) = session.borrow_mut().take() {
        session.stop();
    }
    code
}
