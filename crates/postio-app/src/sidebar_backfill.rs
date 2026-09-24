//! Wires the sidebar's per-folder backfill toggle to the store (ADR 0016,
//! #350).
//!
//! `postio_gtk::sidebar::Sidebar` draws the context menu and calls back
//! through `connect_backfill_exclusion_changed` without knowing anything
//! persists it — the same shape `settings_accounts.rs` wires the settings
//! panel's enable switch through. This is the other half.
//!
//! # Reflected immediately, not on the next sync
//!
//! Unlike a server-driven change to the mailbox tree, nothing here ever
//! emits `Event::MailboxesChanged` — there is no sync pass involved, only a
//! local column write. So the write's answer is the account's mailboxes,
//! and this calls `Sidebar::set_mailboxes` itself, the same way
//! `settings_accounts::refresh` re-reads accounts after a write rather than
//! waiting for an event that will never come. Without it, right-clicking
//! the same folder again immediately after toggling would still show the
//! wording for the state it just left.

use gtk::glib;
use postio_client::Client;
use postio_gtk::window::Window;

/// Wires `window`'s sidebar to the store's owner: skipping or resuming a
/// folder's background backfill from its own context menu.
///
/// One call per toggle: the host writes the column and answers the
/// account's folders as they now stand, which the sidebar is handed again so
/// the context menu's wording is correct the moment it is reopened.
pub async fn install(window: &Window, client: Client) {
    // Weak: the window owns the sidebar that owns this handler (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.sidebar().connect_backfill_exclusion_changed({
        move |id, excluded| {
            postio_session::blocking::now(async {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime.
                match client.set_backfill_excluded(id, excluded).await {
                    Ok(all) if !all.is_empty() => window.sidebar().set_mailboxes(&all),
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%error, "could not change whether a folder backs up locally");
                    }
                }
            })
        }
    });
}
