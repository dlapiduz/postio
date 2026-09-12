//! Wires the sidebar's per-folder backfill toggle to storage (ADR 0016,
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
//! local column write. So this re-reads the account's mailboxes directly
//! and calls `Sidebar::set_mailboxes` itself, the same way
//! `settings_accounts::refresh` re-reads accounts after a write rather than
//! waiting for an event that will never come. Without it, right-clicking
//! the same folder again immediately after toggling would still show the
//! wording for the state it just left.

use gtk::glib;
use postio_gtk::window::Window;
use postio_storage::Store;
use postio_storage::repository::MailboxRepository;

use crate::Wiring;

/// Wires `window`'s sidebar to `wiring`: skipping or resuming a folder's
/// background backfill from its own context menu.
pub async fn install(window: &Window, wiring: &Wiring) {
    let database = wiring.database.clone();
    // Weak: the window owns the sidebar that owns this handler (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.sidebar().connect_backfill_exclusion_changed({
        move |id, excluded| {
            crate::blocking::now(async {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let Ok(connection) = database.connect().await else {
                    return;
                };
                let mailboxes = MailboxRepository::new(&connection);
                if let Err(error) = mailboxes.set_backfill_excluded(id, excluded).await {
                    tracing::warn!(%error, "could not change whether a folder backs up locally");
                    return;
                }
                refresh(&window, &database, id);
        
            })
        }
    });
}

/// Re-reads `id`'s account and hands the sidebar its mailboxes again, so the
/// context menu's wording is correct the moment it is reopened.
async fn refresh(window: &Window, database: &Store, id: postio_model::ids::MailboxId) {
    let Ok(connection) = database.connect().await else {
        return;
    };
    let mailboxes = MailboxRepository::new(&connection);
    let Ok(Some(mailbox)) = mailboxes.get(id).await else {
        return;
    };
    if let Ok(all) = mailboxes.list_for_account(mailbox.account_id).await {
        window.sidebar().set_mailboxes(&all);
    }
}
