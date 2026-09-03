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
//!
//! # Saved searches
//!
//! `[filters]` is the same one-file-is-the-settings promise (issue #10): a
//! pinned entry reaches the sidebar through this same reload bridge, no
//! second store involved. `Ctrl+S` is the other direction — the one write
//! this module makes to the file rather than only reading it — and it takes
//! the plain, decoupled path: read `path` fresh, add the filter, save, and
//! repaint the sidebar directly, rather than routing through the `service`
//! this function already owns. The watcher reaches the same state a moment
//! later and repaints again, redundantly but harmlessly; that redundancy is
//! what keeps a hand-edited `[filters]` and the box's own `Ctrl+S` reaching
//! the sidebar through one path instead of two.

use std::path::Path;

use adw::prelude::*;
use gtk::glib;
use postio_config::Config;
use postio_config::filters::Reorder;
use postio_config::validate::Checked;
use postio_config::watch::ConfigWatcher;
use postio_core::{CommandId, ConfigService, Event};

use crate::finder::Mode;
use crate::sidebar::{SavedSearch, SavedSearchAction};
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
        tracing::warn!("no configuration directory; using the built-in defaults");
        return;
    };
    install_at(window, &path);
}

/// As [`install`], for a path the caller chose.
///
/// Separate so a test can point at a temporary directory rather than the
/// developer's own configuration.
/// Push `[compose]` into the composer: where a signature sits relative to a
/// quote, per draft kind (#12).
fn apply_compose(window: &Window, config: &Config) {
    window.composer().set_signature_placement(
        placement(config.compose.signature_on_reply),
        placement(config.compose.signature_on_forward),
    );
}

/// The schema's spelling, as the body crate's.
fn placement(setting: postio_config::SignaturePlacement) -> postio_body::Placement {
    match setting {
        postio_config::SignaturePlacement::AboveQuote => postio_body::Placement::AboveQuote,
        postio_config::SignaturePlacement::BelowQuote => postio_body::Placement::BelowQuote,
    }
}

pub fn install_at(window: &Window, path: &Path) {
    let mut service = ConfigService::load(path);
    report(service.status().errors());
    window.apply_keymap(service.keymap().clone());
    window.apply_ui(&service.config().ui);
    window.list().set_density(service.config().ui.density);
    window.list().set_keymap(service.keymap().clone());
    apply_compose(window, service.config());
    window.settings().load(path);
    window
        .sidebar()
        .set_saved_searches(&saved_searches(service.config()));
    window.sidebar().connect_search_selected({
        let window = window.downgrade();
        move |query| {
            if let Some(window) = window.upgrade() {
                window.run_search(&query);
            }
        }
    });
    window.sidebar().connect_saved_search_action({
        let path = path.to_path_buf();
        let window = window.downgrade();
        move |key, action| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match action {
                SavedSearchAction::Rename => request_rename(&window, &path, &key),
                SavedSearchAction::MoveUp => move_saved_search(&window, &path, &key, Reorder::Up),
                SavedSearchAction::MoveDown => {
                    move_saved_search(&window, &path, &key, Reorder::Down)
                }
                SavedSearchAction::Delete => request_delete(&window, &path, &key),
            }
        }
    });

    window.connect_command({
        let path = path.to_path_buf();
        let window = window.downgrade();
        move |id| {
            if id == CommandId::EditConfig {
                spawn_editor(&path);
            } else if id == CommandId::SaveSearch
                && let Some(window) = window.upgrade()
            {
                save_current_search(&window, &path);
            } else if let Some(action) = saved_search_action_for(id)
                && let Some(window) = window.upgrade()
                && let Some(key) = window.sidebar().focused_saved_search()
            {
                // The registry keeps these four to `Context::Sidebar` (#455),
                // so a stray invocation with no saved search focused -- the
                // palette, say, over a folder row -- is defended against
                // rather than relied on not to happen, the same guard
                // `Window::run`'s `ToggleThreadUnread` arm uses.
                match action {
                    SavedSearchAction::Rename => request_rename(&window, &path, &key),
                    SavedSearchAction::MoveUp => {
                        move_saved_search(&window, &path, &key, Reorder::Up)
                    }
                    SavedSearchAction::MoveDown => {
                        move_saved_search(&window, &path, &key, Reorder::Down)
                    }
                    SavedSearchAction::Delete => request_delete(&window, &path, &key),
                }
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
            tracing::warn!(path = %path.display(), %error, "config will not be watched; edits need a restart");
            return;
        }
    };

    let weak = window.downgrade();
    glib::spawn_future_local(async move {
        // The watcher is moved in so that it lives exactly as long as the task
        // reading from it. Dropping it stops the thread.
        let _watcher = watcher;
        while let Ok(checked) = receiver.recv().await {
            // One span per reload, so the problems a file produced are
            // attributable to *that* reload rather than to whichever of the
            // day's edits happened to be nearest in the log. Nothing here
            // awaits, so entering it for the body is sound.
            let reload = tracing::info_span!("config_reload", path = %service.path().display());
            let _entered = reload.enter();

            let update = service.apply(checked);
            for event in &update.events {
                if let Event::Error { message } = event {
                    tracing::warn!(message, "rejected");
                }
            }
            tracing::debug!(keys = update.changed.keys, "applied",);
            let Some(window) = weak.upgrade() else {
                break;
            };
            if update.changed.keys {
                window.apply_keymap(service.keymap().clone());
                window.list().set_keymap(service.keymap().clone());
            }
            if update.changed.ui {
                window.apply_ui(&service.config().ui);
                window.list().set_density(service.config().ui.density);
            }
            if update.changed.compose {
                apply_compose(&window, service.config());
            }
            if update.changed.filters {
                window
                    .sidebar()
                    .set_saved_searches(&saved_searches(service.config()));
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

/// The pinned entries of `[filters]`, as the sidebar widget wants them --
/// in [`Config::ordered_filter_keys`]'s order, which `Sidebar::
/// set_saved_searches` now draws exactly as given (#292).
fn saved_searches(config: &Config) -> Vec<SavedSearch> {
    config
        .ordered_filter_keys()
        .into_iter()
        .filter_map(|key| {
            let filter = config.filters.get(&key)?;
            let name = filter.name.clone().unwrap_or_else(|| key.clone());
            Some(SavedSearch {
                key,
                name,
                query: filter.query.clone(),
            })
        })
        .collect()
}

/// Which [`SavedSearchAction`] a registry command id asks for, when it asks
/// for one at all (#455).
///
/// The same four verbs [`Sidebar::connect_saved_search_action`] already
/// reports from the mouse's context menu -- a keystroke is a second way to
/// name one, not a second thing to act on, so both paths end at the exact
/// functions below.
///
/// [`Sidebar::connect_saved_search_action`]: crate::sidebar::Sidebar::connect_saved_search_action
fn saved_search_action_for(id: CommandId) -> Option<SavedSearchAction> {
    match id {
        CommandId::RenameSavedSearch => Some(SavedSearchAction::Rename),
        CommandId::MoveSavedSearchUp => Some(SavedSearchAction::MoveUp),
        CommandId::MoveSavedSearchDown => Some(SavedSearchAction::MoveDown),
        CommandId::DeleteSavedSearch => Some(SavedSearchAction::Delete),
        _ => None,
    }
}

/// `Ctrl+S`: save whatever the search box currently holds as a new pinned
/// filter, and show it in the sidebar right away.
///
/// Reads `path` fresh rather than through the `service` handle `install_at`
/// already owns -- see the module doc's "Saved searches" section for why
/// that decoupling, not a shared mutable `service`, is the simpler seam
/// here. A silent no-op with nothing typed: saving an empty query would
/// pin "everything", which is not a folder anyone meant to make.
fn save_current_search(window: &Window, path: &Path) {
    let finder = window.finder();
    if finder.mode() != Mode::Search {
        return;
    }
    let query = finder.query().text;
    if query.trim().is_empty() {
        return;
    }

    let original = std::fs::read_to_string(path).unwrap_or_default();
    let mut config = Config::from_toml_str(&original).unwrap_or_default();
    config.save_filter(&query);
    if let Err(error) = write_filters(&original, &config, path) {
        tracing::warn!(%error, "could not save the search");
        return;
    }
    window
        .sidebar()
        .set_saved_searches(&saved_searches(&config));
}

/// Writes `config.filters`' current state back to `path`, touching only
/// `[filters]` — every saved-search verb in this module (`Ctrl+S`, rename,
/// reorder, delete) writes through this rather than through
/// [`Config::to_toml_string`], which reserializes the whole file and would
/// silently drop a hand-written comment or reorder every other section on
/// someone's next search save (#885).
fn write_filters(original: &str, config: &Config, path: &Path) -> postio_config::Result<()> {
    let patched = postio_config::patch_filters(original, &config.filters)?;
    Config::write_text_to_path(&patched, path)
}

/// Move `key` up or down among the pinned filters, and repaint.
///
/// No confirmation: [`postio_core::Recovery`] has nothing to say about a
/// reorder because it destroys nothing -- moving it back is the same
/// action once more, the same as any other position swap.
fn move_saved_search(window: &Window, path: &Path, key: &str, direction: Reorder) {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let mut config = Config::from_toml_str(&original).unwrap_or_default();
    if !config.move_filter(key, direction) {
        return;
    }
    if let Err(error) = write_filters(&original, &config, path) {
        tracing::warn!(%error, "could not save the reordered searches");
        return;
    }
    window
        .sidebar()
        .set_saved_searches(&saved_searches(&config));
}

/// Ask before deleting -- the one saved-search verb the registry's
/// `discard_draft` precedent applies to: nothing here can be undone from a
/// toast (issue #292 weighed the undo stack directly and it does not fit a
/// config-file edit; see the issue for why), and re-creating a deleted
/// search costs retyping the query. `discard_draft` is the one other
/// [`Recovery::Confirm`][r] command in this application, and this reuses
/// its exact `adw::AlertDialog` shape rather than adding a second kind of
/// dialog for the same purpose.
///
/// [r]: postio_core::Recovery
fn request_delete(window: &Window, path: &Path, key: &str) {
    let dialog = adw::AlertDialog::new(
        Some("Delete this saved search?"),
        Some("It can be saved again from the same query, but the query itself is gone."),
    );
    dialog.add_responses(&[("keep", "Keep"), ("delete", "Delete")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("keep");
    dialog.connect_response(None, {
        let path = path.to_path_buf();
        let key = key.to_owned();
        let window = window.downgrade();
        move |_, response| {
            if response != "delete" {
                return;
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            delete_saved_search(&window, &path, &key);
        }
    });
    dialog.present(Some(window));
}

fn delete_saved_search(window: &Window, path: &Path, key: &str) {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let mut config = Config::from_toml_str(&original).unwrap_or_default();
    if !config.delete_filter(key) {
        return;
    }
    if let Err(error) = write_filters(&original, &config, path) {
        tracing::warn!(%error, "could not save after deleting the search");
        return;
    }
    window
        .sidebar()
        .set_saved_searches(&saved_searches(&config));
}

/// Ask for a new display name, pre-filled with the one showing now.
///
/// The same `adw::AlertDialog` shape [`request_delete`] uses, with an entry
/// as its extra child instead of a second response -- one dialog widget
/// reused for both of this feature's questions, rather than a second kind
/// for "type something" beside the one already in the app for "are you
/// sure".
fn request_rename(window: &Window, path: &Path, key: &str) {
    let config = Config::load_from_path(path).unwrap_or_default();
    let Some(filter) = config.filters.get(key) else {
        return;
    };
    let current = filter.name.clone().unwrap_or_else(|| key.to_owned());

    let entry = gtk::Entry::new();
    entry.set_text(&current);
    entry.set_activates_default(true);

    let dialog = adw::AlertDialog::new(Some("Rename this saved search?"), None);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, {
        let path = path.to_path_buf();
        let key = key.to_owned();
        let entry = entry.clone();
        let window = window.downgrade();
        move |_, response| {
            if response != "rename" {
                return;
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            rename_saved_search(&window, &path, &key, &entry.text());
        }
    });
    dialog.present(Some(window));
}

fn rename_saved_search(window: &Window, path: &Path, key: &str, name: &str) {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let mut config = Config::from_toml_str(&original).unwrap_or_default();
    if !config.rename_filter(key, name) {
        return;
    }
    if let Err(error) = write_filters(&original, &config, path) {
        tracing::warn!(%error, "could not save the renamed search");
        return;
    }
    window
        .sidebar()
        .set_saved_searches(&saved_searches(&config));
}

/// What the configuration file on disk got wrong.
///
/// `warn`: unlike a dropped key binding, these are the reason a setting the
/// user wrote is not in force, and there is nowhere else they surface at
/// startup — the settings panel only shows them once it is opened.
fn report(errors: &[postio_config::validate::ValidationError]) {
    for error in errors {
        tracing::warn!(%error, "config");
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
        tracing::warn!(
            path = %path.display(),
            "neither $VISUAL nor $EDITOR is set, so there is no editor to open"
        );
        return;
    };
    if let Err(error) = std::process::Command::new(&editor).arg(path).spawn() {
        tracing::warn!(
            editor = %editor.to_string_lossy(),
            path = %path.display(),
            %error,
            "cannot launch the editor"
        );
    }
}
