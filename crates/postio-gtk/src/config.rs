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
//! The bridge -- an `async_channel` whose `Sender` goes to the watcher thread
//! and whose `Receiver` is awaited by a task on the main context -- is
//! `postio_widgets::present::config::follow`, which both desktop apps use.
//! [`install`] hands it this window's answer to a reload: that callback is
//! the only place a reload becomes a repaint.
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
//! window's command stream and the path being watched. The launcher is
//! `postio_widgets::editor`, which Focus answers the same command with; its
//! doc says why it prefers `$VISUAL` and opens no terminal.
//!
//! Every successful reload this bridge sees — whichever save caused it — is
//! also handed to `SettingsPanel::note_known_good`, which
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
//!
//! The edit itself is [`postio_ui::saved_search`], not this module: reading
//! the file, patching only `[filters]` and writing it back has no widget in
//! it, and the frontend that could not reach it drew a *Save search as
//! folder* button that was enabled and did nothing (#1574). What stays here
//! is the half that needs a window — the finder's current query, the two
//! dialogs, and the repaint.
//!
//! # `[storage]` (#929)
//!
//! This crate has no store to enforce a disk ceiling against, so
//! `changed.storage` only reaches [`Window::notify_storage_changed`] here —
//! the composition root, which owns the `Database`/`BlobStore` pair, is what
//! subscribes through [`Window::connect_storage_changed`] and re-runs the
//! eviction pass off the main thread.

use std::path::Path;

use adw::prelude::*;
use postio_config::Config;
use postio_core::{CommandId, ConfigService};
use postio_ui::saved_search::{Reorder, Verb};

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
    let service = ConfigService::load(path);
    report(service.status().errors());
    window.apply_keymap(service.keymap().clone());
    window.apply_ui(&service.config().ui);
    window.list().set_density(service.config().ui.density);
    window.list().set_keymap(service.keymap().clone());
    // The settings footer's `Open in $EDITOR` cap reads its key from here,
    // like every other keycap in the application (#1179).
    window.settings().set_keymap(service.keymap());
    apply_compose(window, service.config());
    window.apply_reader(&service.config().reader);
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

    // A zoom a person chose is the one the next reader starts at (spec 006
    // FR-021): written to `[reader]` alone, so nothing else in the file moves.
    window.connect_zoom_changed({
        let path = path.to_path_buf();
        move |percent| save_zoom(&path, percent)
    });

    window.connect_command({
        let path = path.to_path_buf();
        let window = window.downgrade();
        move |id| {
            if id == CommandId::EditConfig {
                postio_widgets::editor::open(&path);
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

    // The watcher's thread to the main loop, one validated reload at a time:
    // the bridge both desktop apps share (`postio_widgets::present::config`).
    // What a reload *means* to this window stays here.
    let weak = window.downgrade();
    postio_widgets::present::config::follow(service, move |service, update| {
        let Some(window) = weak.upgrade() else {
            return std::ops::ControlFlow::Break(());
        };
        if update.changed.keys {
            window.apply_keymap(service.keymap().clone());
            window.list().set_keymap(service.keymap().clone());
            window.settings().set_keymap(service.keymap());
        }
        if update.changed.ui {
            window.apply_ui(&service.config().ui);
            window.list().set_density(service.config().ui.density);
        }
        if update.changed.compose {
            apply_compose(&window, service.config());
        }
        if update.changed.reader {
            window.apply_reader(&service.config().reader);
        }
        if update.changed.filters {
            window
                .sidebar()
                .set_saved_searches(&saved_searches(service.config()));
        }
        if update.changed.storage {
            window.notify_storage_changed(service.config().storage.max_bytes);
        }
        // Whichever save this was — the panel's own debounced write, or
        // `$EDITOR`'s — a file that loads without error is what "Revert
        // file" should be able to go back to.
        if service.status().is_valid()
            && let Ok(text) = std::fs::read_to_string(service.path())
        {
            window.settings().note_known_good(&text);
        }
        std::ops::ControlFlow::Continue(())
    });
}

/// The pinned entries of `[filters]`, as the sidebar widget wants them --
/// in [`Config::ordered_filter_keys`]'s order, which `Sidebar::
/// set_saved_searches` now draws exactly as given (#292).
///
/// The reading moved to `postio-ui` with the four verbs below (#1574); this
/// is the one-line wrapper the call sites in this module already had.
fn saved_searches(config: &Config) -> Vec<SavedSearch> {
    postio_ui::saved_search::pinned(config)
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
    // A blank query is `postio_ui::saved_search`'s no-op rather than an early
    // return here, so that the macOS field, which has no finder to ask,
    // declines for the same reason and with the same silence.
    let query = finder.query().text;
    edit_searches(
        window,
        path,
        Verb::Save { query: &query },
        "save the search",
    );
}

/// Write `[reader] zoom = percent` to `path`, touching only `[reader]`.
fn save_zoom(path: &Path, percent: u16) {
    if let Err(error) = postio_config::save_zoom(path, percent) {
        tracing::warn!(%error, "could not save the zoom");
    }
}

/// Run one saved-search verb against `path` and repaint the sidebar with
/// whatever it left behind.
///
/// The whole of the verb is `postio_ui::saved_search::apply` -- read the file
/// fresh, patch only `[filters]`, write it back -- so that the Mac runs the
/// same four edits rather than its own four (#1574). What is left here is the
/// half that needs a window: which list to draw, and what to say when the
/// file could not be written.
///
/// `what` completes "could not ...", so it reads as a sentence in the log.
fn edit_searches(window: &Window, path: &Path, verb: Verb<'_>, what: &str) {
    match postio_ui::saved_search::apply(path, verb) {
        Ok(edit) => window.sidebar().set_saved_searches(&edit.searches),
        Err(error) => tracing::warn!(%error, "could not {what}"),
    }
}

/// Move `key` up or down among the pinned filters, and repaint.
///
/// No confirmation: [`postio_core::Recovery`] has nothing to say about a
/// reorder because it destroys nothing -- moving it back is the same
/// action once more, the same as any other position swap.
fn move_saved_search(window: &Window, path: &Path, key: &str, direction: Reorder) {
    edit_searches(
        window,
        path,
        Verb::Move { key, direction },
        "save the reordered searches",
    );
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
    // The words are `postio-ui`'s, not this module's: a confirmation written
    // once on each platform is two confirmations, and the one nobody is
    // looking at is the one that misdescribes what is lost (#1574).
    let prompt = postio_ui::saved_search::DELETE_PROMPT;
    let dialog = adw::AlertDialog::new(Some(prompt.title), prompt.body);
    dialog.add_responses(&[("keep", prompt.cancel), ("delete", prompt.confirm)]);
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
    edit_searches(
        window,
        path,
        Verb::Delete { key },
        "save after deleting the search",
    );
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

    let prompt = postio_ui::saved_search::RENAME_PROMPT;
    let dialog = adw::AlertDialog::new(Some(prompt.title), prompt.body);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", prompt.cancel), ("rename", prompt.confirm)]);
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
    edit_searches(
        window,
        path,
        Verb::Rename { key, name },
        "save the renamed search",
    );
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
