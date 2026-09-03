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

use postio_gtk::settings::AccountAction;
use postio_gtk::window::Window;
use postio_runtime::AttachmentPolicy;
use postio_storage::repository::{AccountRepository, MessageRepository};

use crate::Wiring;

/// Wires `window`'s settings panel to `wiring`: the account list itself,
/// the enable/disable switch, remove-with-undo, and update-credential
/// (opened through [`crate::settings_credential::install`], which needs the
/// runtime and the secret store `wiring` carries alongside the database).
pub fn install(window: &Window, wiring: &Wiring) {
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
            let panel = window.settings();
            panel.set_accounts(accounts);
            panel.set_mail_weights(
                &weights,
                wiring.backfill.attachments == AttachmentPolicy::Eager,
            );
        }
        Err(error) => tracing::warn!(%error, "could not read the accounts to show"),
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
