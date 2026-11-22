//! Wires the settings panel's account rows to storage (ADR 0005 Q6a). Enable/disable takes effect
//! on the next launch:
//! `postio-session::engine` reads `accounts.enabled` only at startup. A removal's undo is the app's
//! toast, wired to
//! [`AccountOp::Restore`], not the global stack; the delete runs at the next launch, before any
//! engine exists.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::{AccountField, AccountOp};
use postio_model::ids::AccountId;

use super::{Outside, Reached, Reindexing};
use crate::settings::{
    AccountAction, AccountEdit, AccountMailboxes, ConnectionStatus, SettingsPanel, SignatureDraft,
};

/// Cheap to clone; each handler holds the panel weakly, since the panel owns the handlers and a
/// strong reference would be a cycle (#1072).
#[derive(Clone)]
pub struct Accounts {
    panel: glib::WeakRef<SettingsPanel>,
    client: Client,
    outside: Rc<dyn Outside>,
    reindexing: Reindexing,
}

impl Accounts {
    pub fn over(
        panel: &SettingsPanel,
        client: Client,
        outside: Rc<dyn Outside>,
        reindexing: Reindexing,
    ) -> Accounts {
        Accounts {
            panel: panel.downgrade(),
            client,
            outside,
            reindexing,
        }
    }

    /// Reads the rows once before it returns. The app hands [`Accounts::hear`] its event stream.
    pub async fn install(
        panel: &SettingsPanel,
        client: Client,
        outside: Rc<dyn Outside>,
        reindexing: Reindexing,
    ) -> Accounts {
        let accounts = Accounts::over(panel, client, outside, reindexing);
        // Not on the startup path: `refresh` reads every account's `footprint` (a count and sum
        // over every message), 1.48s on a
        // real store, for a panel that may never open. Read fresh on every open instead (#871).
        accounts.refresh().await;
        panel.connect_visible_notify({
            let accounts = accounts.clone();
            move |panel| {
                if !panel.is_visible() {
                    return;
                }
                let accounts = accounts.clone();
                postio_core::blocking::now(async move { accounts.refresh().await })
            }
        });

        panel.connect_account_enabled_changed({
            let accounts = accounts.clone();
            move |id, enabled| {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive;
                    // the host answers on its own runtime.
                    let op = AccountOp::SetEnabled {
                        account: id,
                        enabled,
                    };
                    if let Err(error) = accounts.client.account(op).await {
                        tracing::warn!(%error, "could not change whether an account is enabled");
                    }
                    accounts.refresh().await;
                })
            }
        });

        panel.connect_account_action({
            let accounts = accounts.clone();
            move |id, action| {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move {
                    match action {
                        AccountAction::Remove => accounts.remove(id).await,
                        AccountAction::UpdateCredential => accounts.update_credential(id),
                        AccountAction::RebuildIndex => accounts.rebuild_index(id),
                        AccountAction::SetDefault => accounts.set_default(id).await,
                    }
                })
            }
        });

        panel.connect_account_edited({
            let accounts = accounts.clone();
            move |id, edit| {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move {
                    match edit {
                        // A role mapping re-roles the account's folders, is undoable, and announces
                        // itself so folder lists
                        // relabel: the command owns all of that (ADR 0035).
                        AccountEdit::MailboxRole(role, path) => {
                            accounts.outside.run(postio_core::Command::MapMailboxRole {
                                account: Some(id),
                                role: Some(role),
                                path,
                            });
                            // No refresh here: the command announces `MailboxesChanged` and `hear`
                            // redraws from that (write, emit,
                            // repaint); redrawing now would tear down the dropdown whose signal is
                            // still being emitted.
                            return;
                        }
                        edit => edit_account(&accounts.client, id, edit).await,
                    }
                    accounts.refresh().await;
                })
            }
        });

        panel.connect_test_connection({
            let accounts = accounts.clone();
            move |id| {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move { accounts.test_connection(id).await })
            }
        });

        panel.connect_signature_saved({
            let accounts = accounts.clone();
            move |id, draft| {
                let accounts = accounts.clone();
                let draft = draft.clone();
                postio_core::blocking::now(async move { accounts.save_signature(id, &draft).await })
            }
        });

        panel.connect_signature_deleted({
            let accounts = accounts.clone();
            move |id, signature| {
                let accounts = accounts.clone();
                postio_core::blocking::now(
                    async move { accounts.delete_signature(id, signature).await },
                )
            }
        });

        accounts
    }

    /// A rebuild's progress arrives as `BackfillProgress` on the account's own id and is drawn only
    /// while a rebuild of that
    /// account is outstanding (#981).
    pub fn hear(&self, event: &postio_core::Event) {
        match event {
            postio_core::Event::MailboxesChanged { .. } => {
                let accounts = self.clone();
                postio_core::blocking::now(async move { accounts.refresh().await })
            }
            postio_core::Event::BackfillProgress {
                account,
                done,
                total,
                ..
            } if self.reindexing.borrow().contains(account) => {
                if let Some(panel) = self.panel.upgrade() {
                    panel.set_reindex_progress(*account, Some((*done, *total)));
                }
            }
            _ => {}
        }
    }

    pub async fn refresh(&self) {
        let Some(panel) = self.panel.upgrade() else {
            return;
        };
        // Weights are read only when the panel is on screen: `footprint` is `count(*)` and
        // `sum(size)` over every message, 1.48s
        // on a real store; `install` refreshes again when it is shown. `Event::BackfillProgress`
        // carries the figure only during a backfill (#411).
        let showing = panel.is_visible();
        // One call for the whole panel: every account shown (a disabled one included, one pending
        // removal left out), with its
        // folders and role map (ADR 0035), and its weight when `showing`.
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let shown = match self.client.account_settings(showing).await {
            Ok(shown) => shown,
            Err(error) => {
                tracing::warn!(%error, "could not read the accounts to show");
                return;
            }
        };
        let weights: Vec<_> = shown
            .iter()
            .filter_map(|entry| entry.weight.map(|weight| (entry.account.id, weight)))
            .collect();
        let mut rows = Vec::with_capacity(shown.len());
        let mut mailboxes = Vec::with_capacity(shown.len());
        for entry in shown {
            mailboxes.push((
                entry.account.id,
                AccountMailboxes {
                    folders: entry.folders,
                    chosen: entry.chosen,
                    resolved: entry.resolved,
                    refused: entry.refused,
                },
            ));
            rows.push(entry.account);
        }

        // Only an account that signed in through Postio's own OAuth client has anything persisted
        // to read; the keyring is the app's.
        let oauth: Vec<_> = rows
            .iter()
            .filter(|account| account.oauth.is_some())
            .map(|account| (account.id, account.address.address.clone()))
            .collect();
        if !oauth.is_empty() {
            let expiries = self.outside.token_expiries(oauth);
            let panel = self.panel.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: the app reads the keyring on its own
                // runtime and answers over a channel (`Outside`).
                let expiries = expiries.await;
                if let Some(panel) = panel.upgrade() {
                    panel.set_token_expiries(&expiries);
                }
            });
        }

        panel.set_accounts(rows);
        panel.set_account_mailboxes(mailboxes);
        panel.set_mail_weights(&weights, self.outside.attachments_eager());
    }

    /// A refused write goes back to the editor in the store's words: `idx_signatures_name` is
    /// unique on `(account_id, name)`,
    /// whose own words are no answer anybody can act on.
    async fn save_signature(&self, id: AccountId, draft: &SignatureDraft) {
        // One call: the host keeps the rich variant this text-only editor does not show (#1086).
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let written = self
            .client
            .save_signature(id, draft.id, draft.name.clone(), draft.text.clone())
            .await;
        self.after_signature(id, written).await;
    }

    /// `accounts.default_signature_id` is `ON DELETE SET NULL`, so the schema keeps an account
    /// whose default this was consistent.
    async fn delete_signature(&self, id: AccountId, signature: postio_model::ids::SignatureId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let removed = self.client.delete_signature(signature).await;
        self.after_signature(id, removed).await;
    }

    async fn after_signature(
        &self,
        id: AccountId,
        written: Result<(), postio_model::listing::StoreError>,
    ) {
        match written {
            Ok(()) => {
                self.refresh().await;
                if let Some(panel) = self.panel.upgrade() {
                    panel.open_account_detail(id);
                }
            }
            Err(error) => {
                if let Some(panel) = self.panel.upgrade() {
                    panel.set_signature_error(Some(error.message().to_owned()));
                }
            }
        }
    }

    /// The one thing here that leaves the machine, and only because somebody pressed a button: no
    /// probe on open, edit or timer.
    async fn test_connection(&self, id: AccountId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let Ok(rows) = self.client.accounts().await else {
            return;
        };
        let Some(account) = rows.into_iter().find(|account| account.id == id) else {
            if let Some(panel) = self.panel.upgrade() {
                panel.set_connection_status(ConnectionStatus::Answered {
                    incoming: Err("this account is no longer in the store".to_owned()),
                    outgoing: Err("this account is no longer in the store".to_owned()),
                });
            }
            return;
        };
        let answer = self.outside.test_connection(account);
        let panel = self.panel.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: the app dials on its own runtime and answers
            // over a channel (`Outside`).
            let reached: Reached = answer.await;
            let Reached {
                incoming,
                outgoing,
                trouble,
            } = reached;
            if let Some(panel) = panel.upgrade() {
                panel.set_credential_trouble(id, trouble);
                panel.set_connection_status(ConnectionStatus::Answered { incoming, outgoing });
            }
        });
    }

    fn update_credential(&self, id: AccountId) {
        let accounts = self.clone();
        self.outside.update_credential(
            id,
            Box::new(move || {
                if let Some(panel) = accounts.panel.upgrade() {
                    panel.set_credential_trouble(id, None);
                }
                let accounts = accounts.clone();
                postio_core::blocking::now(async move { accounts.refresh().await })
            }),
        );
    }

    async fn remove(&self, id: AccountId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        if let Err(error) = self.client.account(AccountOp::Remove(id)).await {
            tracing::warn!(%error, "could not mark an account for removal");
            return;
        }
        self.refresh().await;
        let accounts = self.clone();
        self.outside.offer_undo(
            "Account removed",
            Box::new(move || {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move {
                    if let Err(error) = accounts.client.account(AccountOp::Restore(id)).await {
                        tracing::warn!(%error, "could not undo removing an account");
                    }
                    accounts.refresh().await;
                })
            }),
        );
    }

    /// The repository clears the previous holder in the same transaction. Nothing here reaches the
    /// network.
    async fn set_default(&self, id: AccountId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        if let Err(error) = self.client.account(AccountOp::SetDefault(id)).await {
            tracing::warn!(%error, "could not set the default account");
        }
        self.refresh().await;
    }

    fn rebuild_index(&self, id: AccountId) {
        self.reindexing.borrow_mut().insert(id);
        let accounts = self.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime, however long the rebuild takes.
            if let Err(error) = accounts.client.rebuild_index(id).await {
                tracing::warn!(%error, "could not rebuild an account's local search index");
            }
            accounts.reindexing.borrow_mut().remove(&id);
            if let Some(panel) = accounts.panel.upgrade() {
                panel.set_reindex_progress(id, None);
            }
        });
    }
}

/// An account is database state, not `config.toml` preference (ADR 0005 Q6b), so the store's owner
/// reads the row, changes
/// the one field and writes the whole thing back.
async fn edit_account(client: &Client, id: AccountId, edit: AccountEdit) {
    let field = match edit {
        AccountEdit::DisplayName(value) => AccountField::DisplayName(value),
        AccountEdit::ImapHost(value) => AccountField::ImapHost(value),
        AccountEdit::ImapPort(value) => AccountField::ImapPort(value),
        AccountEdit::SmtpHost(value) => AccountField::SmtpHost(value),
        AccountEdit::SmtpPort(value) => AccountField::SmtpPort(value),
        AccountEdit::DefaultSignature(value) => AccountField::DefaultSignature(value),
        AccountEdit::MailboxRole(..) => return,
    };
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    if let Err(error) = client.edit_account(id, field).await {
        tracing::warn!(%error, "could not save an account detail edit");
    }
}
