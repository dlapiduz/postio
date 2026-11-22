//! Skipping or resuming a folder's backfill (ADR 0016). Nothing emits `Event::MailboxesChanged`
//! here (no sync pass), so
//! the write's answer is the account's folders, handed to whatever drew them at once.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_model::ids::MailboxId;
use postio_model::listing::MailStore as _;
use postio_model::mailbox::Mailbox;

use crate::settings::SettingsPanel;

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

pub fn install(panel: &SettingsPanel, client: Client) {
    // Weak: the panel owns these handlers (#1072).
    let weak = panel.downgrade();
    panel.connect_backfill_exclusion_changed({
        let weak = weak.clone();
        let client = client.clone();
        move |mailbox, excluded| {
            let weak: glib::WeakRef<SettingsPanel> = weak.clone();
            let client = client.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: one client call, a oneshot receive.
                let answer = set_excluded(&client, mailbox, excluded).await;
                if let Some(all) = answer
                    && let Some(panel) = weak.upgrade()
                {
                    panel.set_account_folders(all);
                }
            });
        }
    });
    panel.connect_map(move |_| {
        let weak: glib::WeakRef<SettingsPanel> = weak.clone();
        let client = client.clone();
        glib::spawn_future_local(async move {
            if let Some(panel) = weak.upgrade() {
                // POSTIO-GLIB-SAFE: client calls, oneshot receives.
                refresh(&panel, &client).await;
            }
        });
    });
}

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
