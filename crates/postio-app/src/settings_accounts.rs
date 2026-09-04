//! Wires the settings panel's account rows to storage (#464, ADR 0005 Q6a).
//!
//! `postio_gtk::settings::SettingsPanel` draws the rows and calls back
//! through `connect_account_enabled_changed`/`connect_account_action`
//! without knowing anything persists them — the same shape `compose.rs`
//! wires the composer's seams through. This is the other half.
//!
//! # Enable/disable takes effect on the next launch
//!
//! `postio-session::engine` only reads `accounts.enabled` at startup
//! (ADR 0005 Q6a); there is no live engine attach/detach yet, so flipping
//! the switch here writes the column and nothing more. The row itself does
//! not say so — see the follow-up filed alongside this for the panel-level
//! wording ADR 0005 Q6a's own text promises.
//!
//! # Remove is local to its own toast, not the undo stack
//!
//! [`postio_gtk::window::Window::show_removable_toast`] is a narrower undo
//! for exactly this case, wired straight to
//! [`postio_storage::repository::AccountRepository::restore`] rather than
//! through the global stack. `u` *does* reach it, as of #471: removal is a
//! command now, with `Recovery::Undo`, and `u` in `Context::Accounts`
//! activates the showing toast. Context-local state, context-local binding;
//! the global stack still never holds an account removal. Marking is instant; the actual delete only
//! runs at the next launch, before any engine exists
//! (`postio_app::reap_pending_accounts`).

use gtk::glib;
use postio_gtk::feed::Feeds;
use postio_gtk::settings::{AccountAction, AccountEdit, AccountMailboxes};
use postio_gtk::window::Window;
use postio_runtime::AttachmentPolicy;
use postio_storage::repository::{
    AccountRepository, MailboxRepository, MailboxRoleRepository, MessageRepository,
};

use crate::Wiring;

/// Wires `window`'s settings panel to `wiring`: the account list itself,
/// the enable/disable switch, remove-with-undo, and update-credential
/// (opened through [`crate::settings_credential::install`], which needs the
/// runtime and the secret store `wiring` carries alongside the database).
pub fn install(window: &Window, wiring: &Wiring, feeds: &Feeds) {
    refresh(window, wiring);

    let panel = window.settings();
    panel.connect_account_enabled_changed({
        let window = window.clone();
        let wiring = wiring.clone();
        move |id, enabled| {
            if let Ok(connection) = wiring.database.connection()
                && let Err(error) = AccountRepository::new(&connection).set_enabled(id, enabled)
            {
                tracing::warn!(%error, "could not change whether an account is enabled");
            }
            refresh(&window, &wiring);
        }
    });

    panel.connect_account_action({
        let window = window.clone();
        let wiring = wiring.clone();
        move |id, action| match action {
            AccountAction::Remove => remove(&window, &wiring, id),
            AccountAction::UpdateCredential => {
                crate::settings_credential::install(&window, &wiring, id)
            }
        }
    });

    // A role mapping, a discovery pass, a folder renamed on the server: any
    // of them changes what the Mailboxes group has to offer, and all of them
    // say so the same way.
    feeds.connect_event({
        let window = window.downgrade();
        let wiring = wiring.clone();
        move |event| {
            if !matches!(event, postio_core::Event::MailboxesChanged { .. }) {
                return;
            }
            if let Some(window) = window.upgrade() {
                refresh(&window, &wiring);
            }
        }
    });

    panel.connect_account_edited({
        let window = window.clone();
        let wiring = wiring.clone();
        move |id, edit| {
            match edit {
                // A role mapping is not an account column: it re-roles the
                // account's folders, is undoable, and has to announce itself
                // so the sidebar relabels -- all of which the command owns
                // (ADR 0025). Everything else here is a field on the row.
                AccountEdit::MailboxRole(role, path) => {
                    window.act(postio_core::Command::MapMailboxRole {
                        account: Some(id),
                        role: Some(role),
                        path,
                    });
                    // No refresh here, deliberately. The command announces
                    // `MailboxesChanged` and the subscription below redraws
                    // from that -- which is both the local-first order (write,
                    // emit, repaint) and the only safe one: redrawing now
                    // would tear down the very dropdown whose signal is still
                    // being emitted.
                    return;
                }
                edit => edit_account(&wiring, id, edit),
            }
            refresh(&window, &wiring);
        }
    });
}

/// Applies one field's new value to `id`'s stored account (#880).
///
/// An account is database state, not `config.toml` preference (ADR 0005
/// Q6b), so this reads the current row, changes the one field the detail
/// view reported, and writes the whole thing back through
/// [`AccountRepository::update`] — the same read-mutate-write shape
/// `remove`'s `mark_pending_deletion` skips only because it is a single
/// column with its own dedicated method.
fn edit_account(wiring: &Wiring, id: postio_model::ids::AccountId, edit: AccountEdit) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let repository = AccountRepository::new(&connection);
    let Ok(Some(mut account)) = repository.get(id) else {
        return;
    };
    match edit {
        AccountEdit::DisplayName(value) => account.display_name = value,
        AccountEdit::ImapHost(value) => account.incoming.host = value,
        AccountEdit::ImapPort(value) => account.incoming.port = value,
        AccountEdit::SmtpHost(value) => account.outgoing.host = value,
        AccountEdit::SmtpPort(value) => account.outgoing.port = value,
        // Handled as a command before this is reached; it writes no column.
        AccountEdit::MailboxRole(..) => return,
    }
    if let Err(error) = repository.update(&mut account) {
        tracing::warn!(%error, "could not save an account detail edit");
    }
}

/// Reads every account and redraws the panel's rows from it.
///
/// Not incremental: the settings panel is opened rarely and an account list
/// is never more than a handful of rows, so rebuilding it fresh is simpler
/// than reconciling a diff, the same trade [`crate::compose`]'s
/// `install_identities` makes for a much larger list.
///
/// `pub(crate)`: [`crate::settings_credential::install`] calls this too, once
/// a credential update closes, since a repaired account's own submission can
/// turn `enabled` back on (`onboarding::configure`) and the row should say
/// so without waiting for the next full refresh.
pub(crate) fn refresh(window: &Window, wiring: &Wiring) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    match AccountRepository::new(&connection).list() {
        // `list()` (unlike `list_enabled()`) has no reason to hide a
        // *disabled* account from settings -- that is exactly where you go
        // to re-enable one. A row pending removal is different: it is on
        // its way out, and showing it here would let it be removed twice
        // or re-enabled out from under the toast that is about to reap it.
        Ok(accounts) => {
            let accounts: Vec<_> = accounts
                .into_iter()
                .filter(|account| !account.pending_deletion)
                .collect();
            // What each account's mail weighs, read here rather than waited
            // for: `Event::BackfillProgress` carries the same figure, but it
            // only arrives while a backfill is running and this panel is
            // opened at a moment that has nothing to do with one. The same
            // trade `sidebar_backfill::refresh` makes -- re-read rather than
            // wait for an event that may never come (#411).
            let messages = MessageRepository::new(&connection);
            let weights: Vec<_> = accounts
                .iter()
                .filter_map(|account| {
                    let footprint = messages
                        .footprint(account.id)
                        .inspect_err(
                            |error| tracing::warn!(%error, "could not measure an account's mail"),
                        )
                        .ok()?;
                    Some((
                        account.id,
                        postio_core::event::MailFootprint {
                            total_bytes: footprint.total_bytes,
                            attachment_bytes: footprint.attachment_bytes,
                            local_bytes: footprint.local_bytes,
                            complete: footprint.complete,
                        },
                    ))
                })
                .collect();
            // The account row's own token-validity line (#878, on top of
            // #870's persistence): only an account that signed in through
            // Postio's own OAuth client has anything persisted to read --
            // a password account has no such thing, and an account fed by
            // an external broker never had this module write one either
            // (`OwnClientTokenSource::persist_expiry`'s own doc explains
            // why). The keyring read is async and this function is not, so
            // it crosses the runtime the same way `onboarding::submit`'s
            // credential test does, and lands back through the same panel
            // `set_accounts`/`set_mail_weights` already update.
            let oauth_accounts: Vec<_> = accounts
                .iter()
                .filter(|account| account.oauth.is_some())
                .map(|account| (account.id, account.address.address.clone()))
                .collect();
            if !oauth_accounts.is_empty() {
                let secrets = wiring.secrets.clone();
                let (sender, receiver) = async_channel::bounded(1);
                wiring.runtime.spawn(async move {
                    let mut expiries = Vec::with_capacity(oauth_accounts.len());
                    for (id, address) in oauth_accounts {
                        let key = postio_account::secret::AccountKey::new(address);
                        let expiry = postio_account::oauth::token_source::stored_expiry(
                            secrets.as_ref(),
                            &key,
                        )
                        .await;
                        expiries.push((id, expiry));
                    }
                    let _ = sender.send(expiries).await;
                });
                glib::spawn_future_local({
                    let window = window.clone();
                    async move {
                        if let Ok(expiries) = receiver.recv().await {
                            window.settings().set_token_expiries(&expiries);
                        }
                    }
                });
            }

            let panel = window.settings();
            let mailboxes = accounts
                .iter()
                .map(|account| (account.id, account_mailboxes(&connection, account.id)))
                .collect();
            panel.set_accounts(accounts);
            panel.set_account_mailboxes(mailboxes);
            panel.set_mail_weights(
                &weights,
                wiring.backfill.attachments == AttachmentPolicy::Eager,
            );
        }
        Err(error) => tracing::warn!(%error, "could not read the accounts to show"),
    }
}

/// One account's folders and role map, for the detail view's Mailboxes
/// group (ADR 0025).
///
/// Three reads rather than one, because the group answers three questions:
/// what folders there are to choose from, what the user has already chosen,
/// and what each role resolves to as things stand. The third is what lets
/// "Automatic" name the folder it picked, and it comes from `by_role` -- the
/// same lookup the send path files a copy through, so the label cannot
/// disagree with where mail actually goes.
fn account_mailboxes(
    connection: &rusqlite::Connection,
    account: postio_model::ids::AccountId,
) -> AccountMailboxes {
    let mailboxes = MailboxRepository::new(connection);
    let folders = mailboxes
        .list_for_account(account)
        .unwrap_or_default()
        .into_iter()
        .filter(|mailbox| mailbox.selectable)
        .map(|mailbox| mailbox.path)
        .collect();
    let chosen = MailboxRoleRepository::new(connection)
        .for_account(account)
        .unwrap_or_default();
    let resolved = [
        postio_model::MailboxRole::Sent,
        postio_model::MailboxRole::Archive,
        postio_model::MailboxRole::Drafts,
        postio_model::MailboxRole::Trash,
        postio_model::MailboxRole::Junk,
    ]
    .into_iter()
    .filter_map(|role| {
        mailboxes
            .by_role(account, role)
            .ok()
            .flatten()
            .map(|mailbox| (role, mailbox.path))
    })
    .collect();
    AccountMailboxes {
        folders,
        chosen,
        resolved,
    }
}

/// Marks `id` for removal, refreshes the panel to reflect it immediately,
/// and offers a toast whose own button restores it — see the module doc for
/// why this is not the global undo stack.
fn remove(window: &Window, wiring: &Wiring, id: postio_model::ids::AccountId) {
    let database = &wiring.database;
    let Ok(connection) = database.connection() else {
        return;
    };
    match AccountRepository::new(&connection).mark_pending_deletion(id) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(%error, "could not mark an account for removal");
            return;
        }
    }
    drop(connection);
    refresh(window, wiring);

    let restore_window = window.clone();
    let restore_wiring = wiring.clone();
    window.show_removable_toast("Account removed", move || {
        if let Ok(connection) = restore_wiring.database.connection()
            && let Err(error) = AccountRepository::new(&connection).restore(id)
        {
            tracing::warn!(%error, "could not undo removing an account");
        }
        refresh(&restore_window, &restore_wiring);
    });
}
