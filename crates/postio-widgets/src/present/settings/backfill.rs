//! Skipping or resuming a folder's background backfill (ADR 0016, #350):
//! the per-folder control in Settings' Sync & storage section, and the one
//! write the classic app's sidebar menu makes too.
//!
//! The panel draws a check per folder and calls back through
//! `connect_backfill_exclusion_changed` without knowing anything persists
//! it. This is the other half.
//!
//! # Reflected immediately, not on the next sync
//!
//! Unlike a server-driven change to the mailbox tree, nothing here ever
//! emits `Event::MailboxesChanged` -- there is no sync pass involved, only a
//! local column write. So the write's answer is the account's folders, and
//! whatever drew them is handed them again at once, rather than waiting for
//! an event that will never come. Without it, the control would still say
//! the state it just left.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_model::ids::MailboxId;
use postio_model::listing::MailStore as _;
use postio_model::mailbox::Mailbox;

use crate::settings::SettingsPanel;

/// Skip (`excluded`) or resume `mailbox`'s backfill. The answer is its
/// account's folders as they now stand; `None` when the write failed, which
/// is logged here.
pub async fn set_excluded(
    client: &Client,
    mailbox: MailboxId,
    excluded: bool,
) -> Option<Vec<Mailbox>> {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    match client.set_backfill_excluded(mailbox, excluded).await {
        Ok(all) => Some(all),
        Err(error) => {
            tracing::warn!(%error, "could not change whether a folder backs up locally");
            None
        }
    }
}

/// Wire `panel`'s per-folder backfill control to the store's owner: every
/// account's folders, read whenever the panel comes on screen, and each
/// check's change written and drawn again from the answer.
pub fn install(panel: &SettingsPanel, client: Client) {
    // Weak: the panel owns these handlers (#1072).
    let weak = panel.downgrade();
    panel.connect_backfill_exclusion_changed({
        let weak = weak.clone();
        let client = client.clone();
        move |mailbox, excluded| {
            let weak: glib::WeakRef<SettingsPanel> = weak.clone();
            let client = client.clone();
            postio_core::blocking::now(async move {
                if let Some(all) = set_excluded(&client, mailbox, excluded).await
                    && let Some(panel) = weak.upgrade()
                {
                    panel.set_account_folders(all);
                }
            })
        }
    });
    panel.connect_map(move |_| {
        let weak: glib::WeakRef<SettingsPanel> = weak.clone();
        let client = client.clone();
        postio_core::blocking::now(async move {
            if let Some(panel) = weak.upgrade() {
                refresh(&panel, &client).await;
            }
        })
    });
}

/// Every account's folders, read by the store's owner.
async fn refresh(panel: &SettingsPanel, client: &Client) {
    // POSTIO-GLIB-SAFE: client calls are oneshot receives; the host answers
    // on its own runtime.
    let accounts = match client.accounts().await {
        Ok(accounts) => accounts,
        Err(error) => {
            tracing::warn!(%error, "could not read the accounts whose folders back up");
            return;
        }
    };
    for account in accounts {
        match client.mailboxes(account.id).await {
            Ok(folders) => panel.set_account_folders(folders),
            Err(error) => tracing::warn!(%error, "could not read an account's folders"),
        }
    }
}
