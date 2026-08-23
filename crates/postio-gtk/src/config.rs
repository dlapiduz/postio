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
//!
//! # `Ctrl+E`
//!
//! [`install_at`] is also where `CommandId::EditConfig` becomes an actual
//! process: it is the one place in `postio-gtk` that already owns both the
//! window's command stream and the path being watched. [`spawn_editor`]
//! prefers `$VISUAL` over `$EDITOR`, the POSIX order, and does not open a
//! terminal to run either in — see its own doc comment for why that is a
//! documented limitation rather than a bug, on the host and doubly so under
//! Flatpak.
//!
//! Every successful reload this bridge sees — whichever save caused it — is
//! also handed to [`crate::settings::SettingsPanel::note_known_good`], which
//! is what lets "Revert file" undo a bad `$EDITOR` save as readily as a bad
//! one typed in the panel.

use std::path::Path;

use adw::prelude::*;
use gtk::glib;
use postio_config::validate::Checked;
use postio_config::watch::ConfigWatcher;
use postio_core::{CommandId, ConfigService, Event};

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
    window.apply_ui(&service.config().ui);
    window.list().set_density(service.config().ui.density);
    window.settings().load(path);

    window.connect_command({
        let path = path.to_path_buf();
        move |id| {
            if id == CommandId::EditConfig {
                spawn_editor(&path);
            }
        }
    });

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
            if update.changed.ui {
                window.apply_ui(&service.config().ui);
                window.list().set_density(service.config().ui.density);
            }
            // Whichever save this was — the panel's own debounced write, or
            // `$EDITOR`'s — a file that loads without error is what "Revert
            // file" should be able to go back to.
            if service.status().is_valid()
                && let Ok(text) = std::fs::read_to_string(service.path())
            {
                window.settings().note_known_good(&text);
            }
        }
    });
}

fn report(errors: &[postio_config::validate::ValidationError]) {
    for error in errors {
        eprintln!("postio: {error}");
    }
}

/// Launches the user's editor on `path`.
///
/// `$VISUAL` wins over `$EDITOR`, the precedence every POSIX tool gives them.
/// Neither is run inside a terminal: many desktop users already point
/// `$EDITOR` at a GUI editor for exactly this reason, and guessing at a
/// terminal emulator to wrap a text-mode one in is not a guess this module
/// has any way to make well. A terminal-only `$EDITOR` — vim, nano, `emacs
/// -nw` — starts with nothing to attach to; that is a real, documented
/// limitation of opening an editor from a GUI application, not a bug to
/// paper over with a heuristic that would be wrong as often as it was right.
///
/// # Flatpak
///
/// This does not work sandboxed, and cannot without more than this function:
/// the sandbox has neither the host's editor binary nor a path to launch one.
/// The one relevant portal, `org.freedesktop.portal.OpenURI`, opens the
/// desktop's default handler for a file — never an arbitrary command, so
/// never literally `$EDITOR` — and there is no terminal portal in the
/// freedesktop spec at all, so a text-mode editor has no portal answer
/// regardless. Reaching the host's own binary would need the app to talk to
/// `org.freedesktop.Flatpak` (the spawn portal) and the manifest to grant it,
/// neither of which this bead adds: that is a sandbox-permission decision for
/// whoever ships the Flatpak build to make deliberately, not a default to
/// slip in here. Until then, the settings panel itself is the sandboxed
/// fallback — it already edits the same file.
fn spawn_editor(path: &Path) {
    let Some(editor) = std::env::var_os("VISUAL").or_else(|| std::env::var_os("EDITOR")) else {
        eprintln!(
            "postio: neither $VISUAL nor $EDITOR is set; cannot open {}",
            path.display()
        );
        return;
    };
    if let Err(error) = std::process::Command::new(&editor).arg(path).spawn() {
        eprintln!(
            "postio: cannot launch {} on {}: {error}",
            editor.to_string_lossy(),
            path.display()
        );
    }
}
