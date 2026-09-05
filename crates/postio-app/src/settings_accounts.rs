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

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use postio_gtk::settings::{AccountAction, AccountEdit, ConnectionStatus, SignatureDraft};
use postio_gtk::window::Window;
use postio_model::ids::AccountId;
use postio_runtime::AttachmentPolicy;
use postio_storage::repository::{AccountRepository, MessageRepository, SignatureRepository};

use crate::Wiring;

/// Accounts whose local search index is being rebuilt right now (#981).
///
/// Shared between this module, which adds and removes membership as
/// [`rebuild_index`] runs, and [`crate::search`], which reads it when
/// composing a search outcome's corpus caveat -- view state about what is
/// on screen right now, not something [`Wiring`] carries for a second
/// window or a background task to see.
pub type Reindexing = Rc<RefCell<HashSet<AccountId>>>;

/// Wires `window`'s settings panel to `wiring`: the account list itself,
/// the enable/disable switch, remove-with-undo, update-credential (opened
/// through [`crate::settings_credential::install`], which needs the runtime
/// and the secret store `wiring` carries alongside the database), and
/// rebuild-index.
pub fn install(window: &Window, wiring: &Wiring, reindexing: Reindexing) {
    refresh(window, wiring);

    let panel = window.settings();
    // Weak throughout: the window owns the settings panel that owns every
    // handler below, so a strong clone is a cycle and the window never frees
    // (#1072). A window that has gone has no panel to refresh and no screen
    // to open, so each upgrade failure is simply nothing to do.
    let weak = glib::object::ObjectExt::downgrade(window);
    panel.connect_account_enabled_changed({
        let weak = weak.clone();
        let wiring = wiring.clone();
        move |id, enabled| {
            if let Ok(connection) = wiring.database.connection()
                && let Err(error) = AccountRepository::new(&connection).set_enabled(id, enabled)
            {
                tracing::warn!(%error, "could not change whether an account is enabled");
            }
            if let Some(window) = weak.upgrade() {
                refresh(&window, &wiring);
            }
        }
    });

    panel.connect_account_action({
        let weak = weak.clone();
        let wiring = wiring.clone();
        let reindexing = reindexing.clone();
        move |id, action| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            match action {
                AccountAction::Remove => remove(&window, &wiring, id),
                AccountAction::UpdateCredential => {
                    crate::settings_credential::install(&window, &wiring, id)
                }
                AccountAction::RebuildIndex => rebuild_index(&window, &wiring, &reindexing, id),
                AccountAction::SetDefault => set_default(&window, &wiring, id),
            }
        }
    });

    panel.connect_account_edited({
        let weak = weak.clone();
        let wiring = wiring.clone();
        move |id, edit| {
            edit_account(&wiring, id, edit);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &wiring);
            }
        }
    });

    panel.connect_test_connection({
        let weak = weak.clone();
        let wiring = wiring.clone();
        move |id| {
            if let Some(window) = weak.upgrade() {
                test_connection(&window, &wiring, id);
            }
        }
    });

    panel.connect_signature_saved({
        let weak = weak.clone();
        let wiring = wiring.clone();
        move |id, draft| {
            if let Some(window) = weak.upgrade() {
                save_signature(&window, &wiring, id, draft);
            }
        }
    });

    panel.connect_signature_deleted({
        let weak = weak.clone();
        let wiring = wiring.clone();
        move |id, signature| {
            if let Some(window) = weak.upgrade() {
                delete_signature(&window, &wiring, id, signature);
            }
        }
    });
}

/// Write a signature, new or edited, and show the account again (#1086).
///
/// Nothing in Postio created one before this: every layer under it -- the
/// model, the store, the composer's picker, #979's default row -- existed and
/// worked, and there was no way to become a user who had any.
///
/// A refused write goes back to the editor rather than to a log.
/// `idx_signatures_name` is a unique index on `(account_id, name)`, so a
/// second "Work" fails, and "UNIQUE constraint failed: signatures.account_id,
/// signatures.name" is not an answer anybody can act on.
fn save_signature(
    window: &Window,
    wiring: &Wiring,
    id: postio_model::ids::AccountId,
    draft: &SignatureDraft,
) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let signatures = SignatureRepository::new(&connection);
    let written = match draft.id {
        Some(existing) => {
            let mut signature = postio_model::Signature::new(&draft.name, &draft.text);
            signature.id = existing;
            // The rich variant is left exactly as it was: this editor is
            // text-only (#1086), and rewriting `html` to `None` here would
            // quietly discard a signature somebody else's tooling wrote.
            if let Some(previous) = existing_signature(&connection, id, existing) {
                signature.html = previous.html;
            }
            signatures.update(&signature)
        }
        None => {
            let mut signature = postio_model::Signature::new(&draft.name, &draft.text);
            signatures.create(id, &mut signature).map(|_| ())
        }
    };
    drop(connection);

    match written {
        Ok(()) => {
            refresh(window, wiring);
            // Back to the account, with the list it now belongs to.
            window.settings().open_account_detail(id);
        }
        Err(error) => window
            .settings()
            .set_signature_error(Some(explain_signature_failure(&draft.name, &error))),
    }
}

/// The stored signature `id` currently is, so an edit keeps the fields this
/// editor does not show.
fn existing_signature(
    connection: &postio_storage::PooledConnection,
    account: postio_model::ids::AccountId,
    id: postio_model::ids::SignatureId,
) -> Option<postio_model::Signature> {
    SignatureRepository::new(connection)
        .list_for_account(account)
        .ok()?
        .into_iter()
        .find(|signature| signature.id == id)
}

/// A store refusal, as the person who typed it needs to read it.
fn explain_signature_failure(name: &str, error: &postio_storage::Error) -> String {
    let raw = error.to_string();
    // The only refusal this form can produce today, and the only one worth
    // recognising: anything else is a real fault and its own text is more
    // use than a guess at what it meant.
    if raw.contains("UNIQUE") || raw.contains("constraint") {
        format!("This account already has a signature called “{name}”")
    } else {
        raw
    }
}

/// Remove a signature and show the account again.
///
/// `accounts.default_signature_id` is `ON DELETE SET NULL`, so an account
/// whose default this was is left consistent by the schema rather than by
/// anything here -- `storage_suite/accounts.rs` is what holds that to
/// account. The refresh below is what makes #979's row and the composer's
/// picker agree with it without a restart.
fn delete_signature(
    window: &Window,
    wiring: &Wiring,
    id: postio_model::ids::AccountId,
    signature: postio_model::ids::SignatureId,
) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let removed = SignatureRepository::new(&connection).delete(signature);
    drop(connection);
    match removed {
        Ok(_) => {
            refresh(window, wiring);
            window.settings().open_account_detail(id);
        }
        Err(error) => window
            .settings()
            .set_signature_error(Some(error.to_string())),
    }
}

/// Try `id`'s stored settings and put the answer on the detail view (#980).
///
/// The one thing in this module that leaves the machine, and it does so only
/// because somebody pressed a button -- `ARCHITECTURE.md` §11's test. Nothing
/// speculative: no probe on open, none on edit, none on a timer.
///
/// The read and the two connections happen off the GTK loop and the answer
/// comes back over an `async_channel`, the same crossing `search.rs` and
/// `feed.rs` make: `rusqlite` is blocking and a connect is a round trip, and
/// the main loop must be inside neither.
fn test_connection(window: &Window, wiring: &Wiring, id: postio_model::ids::AccountId) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let account = match AccountRepository::new(&connection).get(id) {
        Ok(Some(account)) => account,
        // The row went away between the press and here. The panel is already
        // showing "Testing…", so it has to be told something.
        _ => {
            window
                .settings()
                .set_connection_status(ConnectionStatus::Answered {
                    incoming: Err("this account is no longer in the store".to_owned()),
                    outgoing: Err("this account is no longer in the store".to_owned()),
                });
            return;
        }
    };
    drop(connection);

    let secrets = wiring.secrets.clone();
    let (sender, receiver) = async_channel::bounded(1);
    wiring.runtime.spawn(async move {
        // The real connectors: this is the one path that is supposed to
        // dial out. Every test of it hands scripted ones in instead.
        // A connector that will not build is a TLS stack problem, not a
        // server problem, and it has to say so rather than looking like the
        // account is misconfigured.
        let found = match (
            postio_account::imap::RustlsConnector::new(),
            postio_smtp::transport::RustlsConnector::new(),
        ) {
            (Ok(imap), Ok(smtp)) => {
                postio_session::reachability::test_connection(&account, &secrets, &imap, &smtp)
                    .await
            }
            (imap, smtp) => {
                let reason = imap
                    .err()
                    .map(|error| error.to_string())
                    .or_else(|| smtp.err().map(|error| error.to_string()))
                    .unwrap_or_else(|| "the TLS stack would not start".to_owned());
                postio_session::reachability::Reachabilities {
                    incoming: postio_session::reachability::Reachability::Refused {
                        reason: reason.clone(),
                    },
                    outgoing: postio_session::reachability::Reachability::Refused { reason },
                }
            }
        };
        let _ = sender.send(found).await;
    });

    glib::spawn_future_local({
        let window = window.clone();
        async move {
            let Ok(found) = receiver.recv().await else {
                // The task died. Saying so beats the spinner that stops.
                window
                    .settings()
                    .set_connection_status(ConnectionStatus::Answered {
                        incoming: Err("the test did not finish".to_owned()),
                        outgoing: Err("the test did not finish".to_owned()),
                    });
                return;
            };
            window
                .settings()
                .set_connection_status(ConnectionStatus::Answered {
                    incoming: as_result(found.incoming),
                    outgoing: as_result(found.outgoing),
                });
        }
    });
}

/// `Reachability` as the panel wants it: the widget layer may not depend on
/// `postio-session`, so the crossing happens here rather than by giving
/// `postio-gtk` a type it has no business linking.
fn as_result(reachability: postio_session::reachability::Reachability) -> Result<(), String> {
    match reachability {
        postio_session::reachability::Reachability::Reached => Ok(()),
        postio_session::reachability::Reachability::Refused { reason } => Err(reason),
    }
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
        // #979. `Option` all the way through: an account may have
        // signatures and prefer none of them, which is what the composer
        // reads as "use the identity's own".
        AccountEdit::DefaultSignature(value) => account.default_signature_id = value,
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

/// Rebuilds `id`'s local search index (#981), reporting progress on its own
/// row as it runs and clearing the line the moment it is done.
///
/// `postio_session::reindex_account` runs on the blocking pool -- it is
/// synchronous SQLite that decompresses a body or parses a block per
/// message, exactly like the two catch-up passes it wraps. Its own
/// `on_progress` callback therefore also runs there, where a GTK call would
/// be unsound; every reading it reports crosses to the main context over a
/// channel, the same shape [`refresh`]'s own token-expiry read already
/// uses for the same reason.
///
/// Also announced as an [`postio_core::Event::BackfillProgress`] on the
/// account's own account id -- the maintainer's own design for #981: one
/// progress channel, not two. The settings row is driven off the direct
/// channel rather than that event, though, because nothing here can tell a
/// rebuild's report apart from a real backfill's inside the event itself,
/// and "Rebuilding search index" would be the wrong words for the other one.
///
/// `reindexing` gains `id` for as long as the rebuild runs and loses it the
/// moment the channel says it is over -- what [`crate::search`] reads to
/// raise a search outcome's own corpus caveat while this account's index is
/// mid-rebuild (#981's own "the search surface should say so too").
/// Make `id` the account new messages come from (#960).
///
/// A single local write and then a redraw, the shape
/// `connect_account_enabled_changed` already uses — not the async shape
/// `rebuild_index` needs, because there is no long-running work here and
/// nothing to report progress about.
///
/// The repository clears the previous holder in the same transaction, so
/// there is no "unset the other one" step for this to get wrong, and no
/// window in which two rows both claim the marker. Nothing here reaches the
/// network: which account a new message comes from is local state, before
/// and after.
fn set_default(window: &Window, wiring: &Wiring, id: AccountId) {
    if let Ok(connection) = wiring.database.connection()
        && let Err(error) = AccountRepository::new(&connection).set_default(id)
    {
        tracing::warn!(%error, "could not set the default account");
    }
    refresh(window, wiring);
}

fn rebuild_index(window: &Window, wiring: &Wiring, reindexing: &Reindexing, id: AccountId) {
    reindexing.borrow_mut().insert(id);

    let database = wiring.database.clone();
    let events = wiring.events.clone();
    let (sender, receiver) = async_channel::unbounded::<Option<(u32, u32)>>();
    wiring.runtime.spawn_blocking(move || {
        let result = postio_session::reindex_account(&database, id, |done, total| {
            events.emit(postio_core::Event::BackfillProgress {
                account: id,
                done,
                total,
                footprint: None,
            });
            let _ = sender.send_blocking(Some((done, total)));
        });
        if let Err(error) = result {
            tracing::warn!(%error, "could not rebuild an account's local search index");
        }
        let _ = sender.send_blocking(None);
    });

    glib::spawn_future_local({
        let window = window.clone();
        let reindexing = reindexing.clone();
        async move {
            while let Ok(progress) = receiver.recv().await {
                let over = progress.is_none();
                window.settings().set_reindex_progress(id, progress);
                if over {
                    reindexing.borrow_mut().remove(&id);
                    break;
                }
            }
        }
    });
}
