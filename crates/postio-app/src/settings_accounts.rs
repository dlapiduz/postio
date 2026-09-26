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
//! for exactly this case, wired straight to the store owner's
//! [`AccountOp::Restore`] rather than through the global stack. `u` *does*
//! reach it, as of #471: removal is a
//! command now, with `Recovery::Undo`, and `u` in `Context::Accounts`
//! activates the showing toast. Context-local state, context-local binding;
//! the global stack still never holds an account removal. Marking is instant; the actual delete only
//! runs at the next launch, before any engine exists
//! (`postio_app::reap_pending_accounts`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::{AccountField, AccountOp};
use postio_gtk::feed::Feeds;
use postio_gtk::settings::{
    AccountAction, AccountEdit, AccountMailboxes, ConnectionStatus, SignatureDraft,
};
use postio_gtk::window::Window;
use postio_model::ids::AccountId;

use crate::Wiring;
use crate::frontend::Frontend;

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
/// and the secret store `wiring` carries alongside the database),
/// rebuild-index, and each account's mailbox role map.
///
/// Every read and write is the store owner's, through `client` (ADR 0041);
/// `wiring` is left for what stays on this side -- the runtime, the keyring
/// the token-expiry line reads, and the connection test, which is the one
/// thing here that dials out and does so where the person pressed it.
pub async fn install(
    window: &Window,
    wiring: &Wiring,
    client: Client,
    reindexing: Reindexing,
    feeds: &Feeds,
) {
    install_for(
        window,
        &Frontend::over(wiring, client.clone()),
        client,
        reindexing,
        feeds,
    )
    .await;
}

/// [`install`], for a window whose store's owner may be another process.
pub async fn install_for(
    window: &Window,
    frontend: &Frontend,
    client: Client,
    reindexing: Reindexing,
    feeds: &Feeds,
) {
    // **Not on the startup path, and not merely deferred.**
    //
    // `refresh` reads every account's `footprint` -- `count(*)` and
    // `sum(size)` over every message it has -- to fill in what an account's
    // mail weighs. Measured on a real store that is 1.48s, and it was spent
    // before the first frame, for a panel that is not on screen and may never
    // be opened:
    //
    // ```text
    // TIMING settings_accounts::install 1.47984176s
    // TIMING feed_the_window            2.04421078s
    // startup 2626.1ms (... first frame 2457.7ms) budget 500.0ms — OVER
    // ```
    //
    // Moving it to `on_first_frame` was tried and is not enough: a tick
    // callback runs before the frame is marked, so the same 1.5s landed
    // inside the measurement and the total got *worse* (2474ms -> 2717ms,
    // three runs each on a settled machine). Work deferred is not work
    // removed, and a window that paints and then locks for a second and a
    // half is not faster, it is janky.
    //
    // So it is read when the panel is *shown*, which is the same trade
    // `Window::open_settings` already makes for the allow list and the
    // viewport height: "read fresh on every open rather than cached" (#871).
    // Nothing else needs these numbers -- they are drawn in this panel and
    // nowhere else.
    refresh(window, frontend, &client).await;

    {
        let weak = glib::object::ObjectExt::downgrade(window);
        let frontend = frontend.clone();
        let client = client.clone();
        let panel = window.settings();
        gtk::prelude::WidgetExt::connect_visible_notify(&panel, move |panel| {
            postio_session::blocking::now(async {
                if !gtk::prelude::WidgetExt::is_visible(panel) {
                    return;
                }
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &frontend, &client).await;
                }
            })
        });
    }

    let panel = window.settings();
    // Weak throughout: the window owns the settings panel that owns every
    // handler below, so a strong clone is a cycle and the window never frees
    // (#1072). A window that has gone has no panel to refresh and no screen
    // to open, so each upgrade failure is simply nothing to do.
    let weak = glib::object::ObjectExt::downgrade(window);
    panel.connect_account_enabled_changed({
        let weak = weak.clone();
        let frontend = frontend.clone();
        let client = client.clone();
        move |id, enabled| {
            postio_session::blocking::now(async {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime.
                let op = AccountOp::SetEnabled {
                    account: id,
                    enabled,
                };
                if let Err(error) = client.account(op).await {
                    tracing::warn!(%error, "could not change whether an account is enabled");
                }
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &frontend, &client).await;
                }
            })
        }
    });

    panel.connect_account_action({
        let weak = weak.clone();
        let frontend = frontend.clone();
        let client = client.clone();
        let reindexing = reindexing.clone();
        move |id, action| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            postio_session::blocking::now(async {
                match action {
                    AccountAction::Remove => remove(&window, &frontend, &client, id).await,
                    AccountAction::UpdateCredential => {
                        crate::settings_credential::install_for(&window, &frontend, id).await
                    }
                    AccountAction::RebuildIndex => {
                        rebuild_index(&window, &frontend, &client, &reindexing, id)
                    }
                    AccountAction::SetDefault => set_default(&window, &frontend, &client, id).await,
                }
            })
        }
    });

    // A role mapping, a discovery pass, a folder renamed on the server: any
    // of them changes what the Mailboxes group has to offer, and all of them
    // say so the same way.
    //
    // And a rebuild's progress (#981): the store's owner announces each
    // reading as `BackfillProgress` on the account's own id. While this
    // window has a rebuild of that account outstanding the reading is
    // drawn on its row; the row clears when the rebuild's own answer comes
    // back (`rebuild_index`), and a reading that lands after that is not
    // drawn, since the account has left `reindexing` by then.
    feeds.connect_event({
        let window = window.downgrade();
        let frontend = frontend.clone();
        let client = client.clone();
        let reindexing = reindexing.clone();
        move |event| match event {
            postio_core::Event::MailboxesChanged { .. } => postio_session::blocking::now(async {
                if let Some(window) = window.upgrade() {
                    refresh(&window, &frontend, &client).await;
                }
            }),
            postio_core::Event::BackfillProgress {
                account,
                done,
                total,
                ..
            } if reindexing.borrow().contains(account) => {
                if let Some(window) = window.upgrade() {
                    window
                        .settings()
                        .set_reindex_progress(*account, Some((*done, *total)));
                }
            }
            _ => {}
        }
    });

    panel.connect_account_edited({
        let weak = weak.clone();
        let frontend = frontend.clone();
        let client = client.clone();
        move |id, edit| {
            postio_session::blocking::now(async {
                match edit {
                    // A role mapping is not an account column: it re-roles the
                    // account's folders, is undoable, and has to announce itself
                    // so the sidebar relabels -- all of which the command owns
                    // (ADR 0035). Everything else here is a field on the row.
                    AccountEdit::MailboxRole(role, path) => {
                        if let Some(window) = weak.upgrade() {
                            window.act(postio_core::Command::MapMailboxRole {
                                account: Some(id),
                                role: Some(role),
                                path,
                            });
                        }
                        // No refresh here, deliberately. The command announces
                        // `MailboxesChanged` and the subscription below redraws
                        // from that -- which is both the local-first order (write,
                        // emit, repaint) and the only safe one: redrawing now
                        // would tear down the very dropdown whose signal is still
                        // being emitted.
                        return;
                    }
                    edit => edit_account(&client, id, edit).await,
                }
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &frontend, &client).await;
                }
            })
        }
    });

    panel.connect_test_connection({
        let weak = weak.clone();
        let frontend = frontend.clone();
        let client = client.clone();
        move |id| {
            postio_session::blocking::now(async {
                if let Some(window) = weak.upgrade() {
                    test_connection(&window, &frontend, &client, id).await;
                }
            })
        }
    });

    panel.connect_signature_saved({
        let weak = weak.clone();
        let frontend = frontend.clone();
        let client = client.clone();
        move |id, draft| {
            postio_session::blocking::now(async {
                if let Some(window) = weak.upgrade() {
                    save_signature(&window, &frontend, &client, id, draft).await;
                }
            })
        }
    });

    panel.connect_signature_deleted({
        let weak = weak.clone();
        let frontend = frontend.clone();
        move |id, signature| {
            postio_session::blocking::now(async {
                if let Some(window) = weak.upgrade() {
                    delete_signature(&window, &frontend, &client, id, signature).await;
                }
            })
        }
    });
}

/// Write a signature, new or edited, and show the account again (#1086).
///
/// Nothing in Postio created one before this: every layer under it -- the
/// model, the store, the composer's picker, #979's default row -- existed and
/// worked, and there was no way to become a user who had any.
///
/// A refused write goes back to the editor rather than to a log, in the
/// words the store's owner chose for it: `idx_signatures_name` is a unique
/// index on `(account_id, name)`, so a second "Work" fails, and the
/// constraint's own words are not an answer anybody can act on.
async fn save_signature(
    window: &Window,
    frontend: &Frontend,
    client: &Client,
    id: postio_model::ids::AccountId,
    draft: &SignatureDraft,
) {
    // One call: the host keeps the rich variant this text-only editor does
    // not show (#1086), rather than rewriting it to `None`.
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let written = client
        .save_signature(id, draft.id, draft.name.clone(), draft.text.clone())
        .await;
    match written {
        Ok(()) => {
            refresh(window, frontend, client).await;
            // Back to the account, with the list it now belongs to.
            window.settings().open_account_detail(id);
        }
        Err(error) => window
            .settings()
            .set_signature_error(Some(error.message().to_owned())),
    }
}

/// Remove a signature and show the account again.
///
/// `accounts.default_signature_id` is `ON DELETE SET NULL`, so an account
/// whose default this was is left consistent by the schema rather than by
/// anything here -- `storage_suite/accounts.rs` is what holds that to
/// account. The refresh below is what makes #979's row and the composer's
/// picker agree with it without a restart.
async fn delete_signature(
    window: &Window,
    frontend: &Frontend,
    client: &Client,
    id: postio_model::ids::AccountId,
    signature: postio_model::ids::SignatureId,
) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    match client.delete_signature(signature).await {
        Ok(()) => {
            refresh(window, frontend, client).await;
            window.settings().open_account_detail(id);
        }
        Err(error) => window
            .settings()
            .set_signature_error(Some(error.message().to_owned())),
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
async fn test_connection(
    window: &Window,
    frontend: &Frontend,
    client: &Client,
    id: postio_model::ids::AccountId,
) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let Ok(accounts) = client.accounts().await else {
        return;
    };
    let account = match accounts.into_iter().find(|account| account.id == id) {
        Some(account) => account,
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

    let secrets = frontend.secrets.clone();
    let (sender, receiver) = async_channel::bounded(1);
    frontend.runtime.spawn(async move {
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
/// Q6b), so the store's owner reads the current row, changes the one field
/// the detail view reported, and writes the whole thing back.
async fn edit_account(client: &Client, id: postio_model::ids::AccountId, edit: AccountEdit) {
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
pub(crate) async fn refresh(window: &Window, frontend: &Frontend, client: &Client) {
    // What each account's mail weighs, read here rather than waited for:
    // `Event::BackfillProgress` carries the same figure, but it only arrives
    // while a backfill is running and this panel is opened at a moment that
    // has nothing to do with one. The same trade `sidebar_backfill` makes --
    // re-read rather than wait for an event that may never come (#411).
    //
    // **Only when the panel is on screen.** `footprint` is `count(*)` and
    // `sum(size)` over every message an account has, and on a real store
    // that is 1.48s -- which at startup is spent before the first frame, for
    // a figure drawn in a panel that may never be opened. Measured: startup
    // 2474ms with it, ~1400ms without.
    //
    // The rows themselves stay unconditional: names, enabled state and token
    // expiry are cheap, and several wirings read them without opening
    // anything. It is the weights alone that cost, and `install` refreshes
    // again when the panel is shown -- the same trade `Window::open_settings`
    // already makes for the allow list and the viewport height, "read fresh
    // on every open rather than cached" (#871).
    let showing = gtk::prelude::WidgetExt::is_visible(&window.settings());
    // One call for the whole panel: every account the settings show (a
    // disabled one included -- this is where it is enabled again -- and one
    // pending removal left out, since it is on its way out), each with its
    // folders and role map (ADR 0035), and its weight when `showing`.
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let shown = match client.account_settings(showing).await {
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
    let mut accounts = Vec::with_capacity(shown.len());
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
        accounts.push(entry.account);
    }

    // The account row's own token-validity line (#878, on top of #870's
    // persistence): only an account that signed in through Postio's own
    // OAuth client has anything persisted to read -- a password account has
    // no such thing, and an account fed by an external broker never had this
    // module write one either (`OwnClientTokenSource::persist_expiry`'s own
    // doc explains why). The keyring read is async and this function is
    // not, so it crosses the runtime the same way `onboarding::submit`'s
    // credential test does, and lands back through the same panel
    // `set_accounts`/`set_mail_weights` already update.
    let oauth_accounts: Vec<_> = accounts
        .iter()
        .filter(|account| account.oauth.is_some())
        .map(|account| (account.id, account.address.address.clone()))
        .collect();
    if !oauth_accounts.is_empty() {
        let secrets = frontend.secrets.clone();
        let (sender, receiver) = async_channel::bounded(1);
        frontend.runtime.spawn(async move {
            let mut expiries = Vec::with_capacity(oauth_accounts.len());
            for (id, address) in oauth_accounts {
                let key = postio_account::secret::AccountKey::new(address);
                let expiry =
                    postio_account::oauth::token_source::stored_expiry(secrets.as_ref(), &key)
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
    panel.set_account_mailboxes(mailboxes);
    panel.set_mail_weights(&weights, frontend.attachments_eager);
}

/// Marks `id` for removal, refreshes the panel to reflect it immediately,
/// and offers a toast whose own button restores it — see the module doc for
/// why this is not the global undo stack.
async fn remove(
    window: &Window,
    frontend: &Frontend,
    client: &Client,
    id: postio_model::ids::AccountId,
) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    if let Err(error) = client.account(AccountOp::Remove(id)).await {
        tracing::warn!(%error, "could not mark an account for removal");
        return;
    }
    refresh(window, frontend, client).await;

    let restore_window = window.clone();
    let restore_frontend = frontend.clone();
    let restore_client = client.clone();
    window.show_removable_toast("Account removed", move || {
        postio_session::blocking::now(async {
            if let Err(error) = restore_client.account(AccountOp::Restore(id)).await {
                tracing::warn!(%error, "could not undo removing an account");
            }
            refresh(&restore_window, &restore_frontend, &restore_client).await;
        })
    });
}

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
async fn set_default(window: &Window, frontend: &Frontend, client: &Client, id: AccountId) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    if let Err(error) = client.account(AccountOp::SetDefault(id)).await {
        tracing::warn!(%error, "could not set the default account");
    }
    refresh(window, frontend, client).await;
}

/// Rebuilds `id`'s local search index (#981), reporting progress on its own
/// row as it runs and clearing the line the moment it is done.
///
/// The rebuild is the store owner's: one call, answered when it is over,
/// asked on the runtime and answered over a channel. Its readings arrive
/// meanwhile as [`postio_core::Event::BackfillProgress`] on the account's
/// own id -- the maintainer's own design for #981: one progress channel,
/// not two -- and [`install`] draws them on this row while `reindexing`
/// holds the account. A real backfill of the same account running at the
/// same moment reports on the same id, so its readings would be drawn here
/// too for as long as the rebuild lasts; before the move the row had a
/// channel of its own and could not confuse them.
///
/// `reindexing` gains `id` for as long as the rebuild runs and loses it the
/// moment the answer says it is over -- what [`crate::search`] reads to
/// raise a search outcome's own corpus caveat while this account's index is
/// mid-rebuild (#981's own "the search surface should say so too").
fn rebuild_index(
    window: &Window,
    frontend: &Frontend,
    client: &Client,
    reindexing: &Reindexing,
    id: AccountId,
) {
    reindexing.borrow_mut().insert(id);

    let (sender, receiver) = async_channel::bounded(1);
    let client = client.clone();
    frontend.runtime.spawn(async move {
        let _ = sender.send(client.rebuild_index(id).await).await;
    });

    glib::spawn_future_local({
        let window = window.clone();
        let reindexing = reindexing.clone();
        async move {
            if let Ok(Err(error)) = receiver.recv().await {
                tracing::warn!(%error, "could not rebuild an account's local search index");
            }
            reindexing.borrow_mut().remove(&id);
            window.settings().set_reindex_progress(id, None);
        }
    });
}
