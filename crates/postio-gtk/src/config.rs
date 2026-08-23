//! `config.toml`, applied live.
//!
//! The design promises *applied live · nothing to save*. `postio-config` has
//! the watcher and `postio-core` has the resolution onto the command registry;
//! what was missing was the last hop, because they meet across a thread
//! boundary that neither of them can cross:
//!
//! * The watcher reparses and validates on **its own thread**, so a broken file
//!   never costs the UI a frame. It hands back a [`Checked`], which is `Send`.
//! * Every GTK widget is **main-thread only** and not `Send`, so the watcher's
//!   callback cannot touch the window.
//!
//! [`install`] is the bridge: an `async_channel` whose `Sender` goes to the
//! watcher thread and whose `Receiver` is awaited by a task on the main
//! context, where the window is. That task is the only place a reload becomes
//! a repaint.
//!
//! # A broken file is not a broken application
//!
//! A reload that fails validation leaves the last good configuration — and so
//! the last good keymap — exactly as it was, and reports the problem. The user
//! keeps a working keyboard to fix the file with, `Ctrl+E` included. That
//! behaviour is `postio-core`'s; this module only has to not undo it.

use std::path::Path;

use adw::prelude::*;
use gtk::glib;
use postio_config::validate::Checked;
use postio_config::watch::ConfigWatcher;
use postio_core::{ConfigService, Event};

use crate::window::Window;

/// Load `config.toml`, apply it to `window`, and keep applying it.
///
/// Best effort throughout. A configuration directory that cannot be resolved,
/// or a watcher that cannot be started, costs the *live* half and nothing
/// else — the file that was on disk at startup is still in force. An
/// application that refused to open because its settings could not be watched
/// would be a worse answer than one whose settings need a restart.
pub fn install(window: &Window) {
    let Ok(path) = postio_config::paths::config_path() else {
        eprintln!("postio: no configuration directory; using the built-in defaults");
        return;
    };
    install_at(window, &path);
}

/// As [`install`], for a path the caller chose.
///
/// Separate so a test can point at a temporary directory rather than the
/// developer's own configuration.
pub fn install_at(window: &Window, path: &Path) {
    let mut service = ConfigService::load(path);
    report(service.status().errors());
    window.apply_keymap(service.keymap().clone());
    window.settings().load(path);

    // Unbounded because the sender is a file watcher that has already debounced
    // a burst of save events down to one message, and because blocking that
    // thread would be worse than queueing.
    let (sender, receiver) = async_channel::unbounded::<Checked>();
    let watcher = match ConfigWatcher::new(path, move |checked| {
        // `send_blocking` on the watcher's own thread, which is allowed to
        // block and has nothing else to do.
        let _ = sender.send_blocking(checked);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("postio: {path:?} will not be watched: {error}");
            return;
        }
    };

    let weak = window.downgrade();
    glib::spawn_future_local(async move {
        // The watcher is moved in so that it lives exactly as long as the task
        // reading from it. Dropping it stops the thread.
        let _watcher = watcher;
        while let Ok(checked) = receiver.recv().await {
            let update = service.apply(checked);
            for event in &update.events {
                if let Event::Error { message } = event {
                    eprintln!("postio: {message}");
                }
            }
            let Some(window) = weak.upgrade() else {
                break;
            };
            if update.changed.keys {
                window.apply_keymap(service.keymap().clone());
            }
        }
    });
}

fn report(errors: &[postio_config::validate::ValidationError]) {
    for error in errors {
        eprintln!("postio: {error}");
    }
}
