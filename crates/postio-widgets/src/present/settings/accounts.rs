//! Wires the settings panel's account rows to storage (#464, ADR 0005 Q6a).
//!
//! [`SettingsPanel`] draws the rows and calls back through
//! `connect_account_enabled_changed`/`connect_account_action` without
//! knowing anything persists them -- the same shape the composer's seams
//! are wired through. This is the other half, for both desktop apps.
//!
//! # Enable/disable takes effect on the next launch
//!
//! `postio-session::engine` only reads `accounts.enabled` at startup
//! (ADR 0005 Q6a); there is no live engine attach/detach yet, so flipping
//! the switch here writes the column and nothing more.
//!
//! # Remove is local to its own toast, not the undo stack
//!
//! A removal's undo is the app's toast ([`Outside::offer_undo`]), wired
//! straight to the store owner's [`AccountOp::Restore`] rather than through
//! the global stack. Context-local state, context-local binding; the global
//! stack never holds an account removal. Marking is instant; the actual
//! delete only runs at the next launch, before any engine exists.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::{AccountField, AccountOp};
use postio_model::ids::AccountId;

use super::{Outside, Reindexing};
use crate::settings::{
    AccountAction, AccountEdit, AccountMailboxes, ConnectionStatus, SettingsPanel, SignatureDraft,
};

/// The account rows of one settings panel, joined to the store's host.
///
/// Cheap to clone: every handler the panel holds keeps one, and each holds
/// the panel weakly -- the panel owns the handlers, so a strong reference
/// would be a cycle and the panel would never free (#1072).
#[derive(Clone)]
pub struct Accounts {
    panel: glib::WeakRef<SettingsPanel>,
    client: Client,
    outside: Rc<dyn Outside>,
    reindexing: Reindexing,
}

impl Accounts {
    /// The presenter over `panel`, wiring nothing: what an app that only
    /// wants the rows read again holds for a moment.
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

    /// Wire `panel`'s account rows to `client`: the list itself, the
    /// enable/disable switch, remove-with-undo, update-credential,
    /// rebuild-index, signatures, the connection test, and each account's
    /// mailbox role map. Reads the rows once before it returns.
    ///
    /// The app hands [`Accounts::hear`] what its event stream says: a role
    /// mapping redraws the Mailboxes group, and a rebuild reports its
    /// progress on its row.
    pub async fn install(
        panel: &SettingsPanel,
        client: Client,
        outside: Rc<dyn Outside>,
        reindexing: Reindexing,
    ) -> Accounts {
        let accounts = Accounts::over(panel, client, outside, reindexing);
        // **Not on the startup path, and not merely deferred.**
        //
        // `refresh` reads every account's `footprint` -- `count(*)` and
        // `sum(size)` over every message it has -- to fill in what an
        // account's mail weighs. Measured on a real store that is 1.48s, and
        // it was spent before the first frame, for a panel that is not on
        // screen and may never be opened. Work deferred is not work removed,
        // so it is read when the panel is *shown*: "read fresh on every open
        // rather than cached" (#871). Nothing else needs these numbers.
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
                        // A role mapping is not an account column: it re-roles
                        // the account's folders, is undoable, and has to
                        // announce itself so the folder lists relabel -- all
                        // of which the command owns (ADR 0035). Everything
                        // else here is a field on the row.
                        AccountEdit::MailboxRole(role, path) => {
                            accounts.outside.run(postio_core::Command::MapMailboxRole {
                                account: Some(id),
                                role: Some(role),
                                path,
                            });
                            // No refresh here, deliberately. The command
                            // announces `MailboxesChanged` and `hear` redraws
                            // from that -- which is both the local-first order
                            // (write, emit, repaint) and the only safe one:
                            // redrawing now would tear down the very dropdown
                            // whose signal is still being emitted.
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

    /// What the store just said, as far as the account rows care.
    ///
    /// A role mapping, a discovery pass, a folder renamed on the server: any
    /// of them changes what the Mailboxes group has to offer, and all of them
    /// say so the same way. And a rebuild's progress (#981): the store's
    /// owner announces each reading as `BackfillProgress` on the account's
    /// own id. While a rebuild of that account is outstanding the reading is
    /// drawn on its row; the row clears when the rebuild's own answer comes
    /// back ([`Accounts::rebuild_index`]), and a reading that lands after
    /// that is not drawn, since the account has left `reindexing` by then.
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

    /// Reads every account and redraws the panel's rows from it.
    ///
    /// Not incremental: the settings panel is opened rarely and an account
    /// list is never more than a handful of rows, so rebuilding it fresh is
    /// simpler than reconciling a diff.
    pub async fn refresh(&self) {
        let Some(panel) = self.panel.upgrade() else {
            return;
        };
        // What each account's mail weighs, read here rather than waited for:
        // `Event::BackfillProgress` carries the same figure, but it only
        // arrives while a backfill is running and this panel is opened at a
        // moment that has nothing to do with one (#411).
        //
        // **Only when the panel is on screen.** `footprint` is `count(*)` and
        // `sum(size)` over every message an account has, and on a real store
        // that is 1.48s. The rows themselves stay unconditional: names,
        // enabled state and token expiry are cheap. It is the weights alone
        // that cost, and `install` refreshes again when the panel is shown.
        let showing = panel.is_visible();
        // One call for the whole panel: every account the settings show (a
        // disabled one included -- this is where it is enabled again -- and
        // one pending removal left out, since it is on its way out), each
        // with its folders and role map (ADR 0035), and its weight when
        // `showing`.
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

        // The account row's own token-validity line (#878): only an account
        // that signed in through Postio's own OAuth client has anything
        // persisted to read. The keyring is the app's to read, and the
        // answer lands back through the same panel `set_accounts` updates.
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

    /// Write a signature, new or edited, and show the account again (#1086).
    ///
    /// A refused write goes back to the editor rather than to a log, in the
    /// words the store's owner chose for it: `idx_signatures_name` is a
    /// unique index on `(account_id, name)`, so a second "Work" fails, and
    /// the constraint's own words are not an answer anybody can act on.
    async fn save_signature(&self, id: AccountId, draft: &SignatureDraft) {
        // One call: the host keeps the rich variant this text-only editor
        // does not show (#1086), rather than rewriting it to `None`.
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let written = self
            .client
            .save_signature(id, draft.id, draft.name.clone(), draft.text.clone())
            .await;
        self.after_signature(id, written).await;
    }

    /// Remove a signature and show the account again.
    ///
    /// `accounts.default_signature_id` is `ON DELETE SET NULL`, so an account
    /// whose default this was is left consistent by the schema rather than by
    /// anything here. The refresh below is what makes #979's row and the
    /// composer's picker agree with it without a restart.
    async fn delete_signature(&self, id: AccountId, signature: postio_model::ids::SignatureId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let removed = self.client.delete_signature(signature).await;
        self.after_signature(id, removed).await;
    }

    /// Back to the account with the list a signature now belongs to, or the
    /// refusal in the editor.
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

    /// Try `id`'s stored settings and put the answer on the detail view
    /// (#980).
    ///
    /// The one thing here that leaves the machine, and it does so only
    /// because somebody pressed a button. Nothing speculative: no probe on
    /// open, none on edit, none on a timer.
    async fn test_connection(&self, id: AccountId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let Ok(rows) = self.client.accounts().await else {
            return;
        };
        let Some(account) = rows.into_iter().find(|account| account.id == id) else {
            // The row went away between the press and here. The panel is
            // already showing "Testing…", so it has to be told something.
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
            let (incoming, outgoing) = answer.await;
            if let Some(panel) = panel.upgrade() {
                panel.set_connection_status(ConnectionStatus::Answered { incoming, outgoing });
            }
        });
    }

    /// The credential dialog for `id`, and the rows again once it is saved:
    /// a repaired account's own submission can turn `enabled` back on.
    fn update_credential(&self, id: AccountId) {
        let accounts = self.clone();
        self.outside.update_credential(
            id,
            Box::new(move || {
                let accounts = accounts.clone();
                postio_core::blocking::now(async move { accounts.refresh().await })
            }),
        );
    }

    /// Marks `id` for removal, refreshes the panel to reflect it immediately,
    /// and offers a toast whose own button restores it -- see the module doc
    /// for why this is not the global undo stack.
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

    /// Make `id` the account new messages come from (#960).
    ///
    /// The repository clears the previous holder in the same transaction, so
    /// there is no "unset the other one" step for this to get wrong. Nothing
    /// here reaches the network: which account a new message comes from is
    /// local state, before and after.
    async fn set_default(&self, id: AccountId) {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        if let Err(error) = self.client.account(AccountOp::SetDefault(id)).await {
            tracing::warn!(%error, "could not set the default account");
        }
        self.refresh().await;
    }

    /// Rebuilds `id`'s local search index (#981), reporting progress on its
    /// own row as it runs and clearing the line the moment it is done.
    ///
    /// The rebuild is the store owner's: one call, answered when it is over.
    /// Its readings arrive meanwhile as [`postio_core::Event::BackfillProgress`]
    /// on the account's own id, which [`Accounts::hear`] draws on this row
    /// while `reindexing` holds the account.
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

/// Applies one field's new value to `id`'s stored account (#880).
///
/// An account is database state, not `config.toml` preference (ADR 0005
/// Q6b), so the store's owner reads the current row, changes the one field
/// the detail view reported, and writes the whole thing back.
async fn edit_account(client: &Client, id: AccountId, edit: AccountEdit) {
    let field = match edit {
        AccountEdit::DisplayName(value) => AccountField::DisplayName(value),
        AccountEdit::ImapHost(value) => AccountField::ImapHost(value),
        AccountEdit::ImapPort(value) => AccountField::ImapPort(value),
        AccountEdit::SmtpHost(value) => AccountField::SmtpHost(value),
        AccountEdit::SmtpPort(value) => AccountField::SmtpPort(value),
        // #979. `Option` all the way through: an account may have
        // signatures and prefer none of them, which is what the composer
        // reads as "use the identity's own".
        AccountEdit::DefaultSignature(value) => AccountField::DefaultSignature(value),
        // Handled as a command before this is reached; it writes no column.
        AccountEdit::MailboxRole(..) => return,
    };
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    if let Err(error) = client.edit_account(id, field).await {
        tracing::warn!(%error, "could not save an account detail edit");
    }
}
