//! The store starts opening on a thread before GTK does anything (#1604): the keyring and the
//! engine's open need nothing from the window.
//! Sync starts after the first frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::startup::{self, Session};
use crate::window::FocusWindow;

/// The D-Bus name, desktop entry basename and Wayland `app_id`. Nothing on disk is keyed by it
/// outside a sandbox.
pub const APP_ID: &str = "dev.postio.Postio";

pub const NAME: &str = "Postio";

pub const ICON_NAME: &str = "dev.postio.Postio";

/// Unset in everyday builds, so a new commit does not rebuild this crate in every worktree sharing
/// the cache.
pub const COMMIT: Option<&str> = option_env!("POSTIO_COMMIT");

pub fn version_line(version: &str, commit: Option<&str>) -> String {
    match commit.filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("{version} ({commit})"),
        None => version.to_owned(),
    }
}

pub fn application() -> adw::Application {
    // GNOME matches a window to its desktop entry by the Wayland `app_id`, which GDK takes from the
    // program name (the binary's, `postio`, unless set); without this the 0.4.2 Flatpak drew a
    // generic icon.
    glib::set_prgname(Some(APP_ID));
    adw::Application::builder()
        .application_id(APP_ID)
        // mailto: and postio: links arrive as files to open; without this flag GApplication drops
        // them.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build()
}

pub fn run() -> glib::ExitCode {
    let timeline = postio_widgets::startup::Timeline::start();
    let config_path = postio_config::paths::config_path().ok();
    // A store that will not open and a keyring that will not answer happen before there is any UI
    // to report them in.
    let logging = postio_session::logging::init(
        &config_path
            .as_deref()
            .map(postio_session::logging::config_at)
            .unwrap_or_default(),
    );
    // Held for the life of the process: dropping it stops the watch.
    let _log_watch = config_path.as_deref().and_then(|path| logging.watch(path));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio starting");

    let config = Rc::new(
        config_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
            .unwrap_or_default(),
    );

    let secrets: Arc<dyn postio_account::secret::SecretStore> =
        Arc::new(postio_account::secret::KeyringSecretStore::default());
    let opener = startup::Opener::new(config_path.clone(), secrets);
    // Before GTK, so the open overlaps GTK's own start.
    let early = RefCell::new(Some(opener.open_on_a_thread()));

    if adw::init().is_err() {
        // An empty pair is a terminal outside the desktop session (tmux, ssh), not a broken build.
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
            if let Some(window) = application.active_window() {
                window.present();
                return;
            }
            let window = FocusWindow::new(Some(application));
            startup::time(&window, timeline.clone());
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
                    if let (Some(path), Some(window)) = (config_path.as_deref(), window.upgrade()) {
                        opened.follow_config(&window, path);
                    }
                    let session = Rc::clone(&session);
                    session.replace(Some(opened));
                    if let Some(window) = window.upgrade() {
                        // The network after the frame the stored mail is drawn in, never before it.
                        let warm = window.downgrade();
                        startup::after_first_frame(&window, move || {
                            if let Some(session) = session.borrow().as_ref() {
                                session.start_syncing();
                            }
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

    application.connect_open(|application, files, _| {
        application.activate();
        let window = application.active_window().and_downcast::<FocusWindow>();
        if let Some(window) = window {
            for file in files {
                window.open_link(&file.uri());
            }
        }
    });

    let code = application.run();

    // Engines first, then the clean-shutdown mark, before the host's runtime goes.
    if let Some(session) = session.borrow_mut().take() {
        session.stop();
    }
    code
}

#[cfg(test)]
mod about_tests {
    use super::version_line;

    #[test]
    fn a_release_build_says_its_commit_beside_its_version() {
        assert_eq!(
            version_line("0.6.1", Some("1eeec0e8d")),
            "0.6.1 (1eeec0e8d)"
        );
    }

    #[test]
    fn a_build_with_no_commit_says_its_version_alone() {
        assert_eq!(version_line("0.6.1", None), "0.6.1");
        assert_eq!(version_line("0.6.1", Some("")), "0.6.1");
    }
}
