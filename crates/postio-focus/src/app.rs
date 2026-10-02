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

/// Postio's application id: the D-Bus name a second launch finds the first
/// by, the desktop entry's basename and the Wayland `app_id` a window is
/// matched to that entry by. Focus is Postio (spec 007, C27; ADR 0043), so it
/// is the id the desktop package has always had. Nothing on disk is keyed by
/// it outside a sandbox: config, state and the store live under `postio` in
/// the XDG directories, so the switch moved none of them.
pub const APP_ID: &str = "dev.postio.Postio";

/// The icon Postio is drawn with, which the Flatpak installs and the
/// desktop entry's `Icon=` names: the window's default icon, the desktop
/// entry and the binary's bundled theme all use this name.
pub const ICON_NAME: &str = "dev.postio.Postio";

/// Focus's application, as `run` starts it.
pub fn application() -> adw::Application {
    // Tell the compositor which application this is: GNOME matches a
    // window to its desktop entry by the Wayland `app_id`, which GDK takes
    // from the program name -- the binary's, `postio`, unless it is set.
    // The entry is `dev.postio.Postio.desktop`, and its `StartupWMClass`
    // names the same id. Reported against the 0.4.2 Flatpak: with the
    // binary's name instead, a session looked for `postio.desktop`, found
    // nothing, and drew a generic icon under a generic name.
    glib::set_prgname(Some(APP_ID));
    adw::Application::builder()
        .application_id(APP_ID)
        // The desktop entry registers `x-scheme-handler/mailto` and
        // `x-scheme-handler/postio` with `%U`, so a link clicked elsewhere
        // arrives as a file to open; without this flag GApplication drops
        // it (T159, T244).
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build()
}

/// The whole program: open the store, show the inbox, run until closed.
pub fn run() -> glib::ExitCode {
    // The budget is measured from process start (`postio_widgets::startup`).
    let timeline = postio_widgets::startup::Timeline::start();
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
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio starting");

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
    let opener = startup::Opener::new(config_path.clone(), secrets);
    // Before GTK: the open overlaps the whole of GTK's own start.
    let early = RefCell::new(Some(opener.open_on_a_thread()));

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
    timeline.mark(postio_widgets::startup::Phase::Init);

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
            startup::time(&window, timeline.clone());
            // Nothing unless `postio_widgets::jank` is enabled at debug.
            postio_widgets::jank::install(&window);
            window.present();
            let progress = early
                .borrow_mut()
                .take()
                .unwrap_or_else(|| opener.open_on_a_thread());
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
                opener.clone(),
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
