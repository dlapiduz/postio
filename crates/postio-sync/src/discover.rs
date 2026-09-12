//! Turning the server's folder list into local rows.
//!
//! # The step that was missing
//!
//! Every folder-enumerating path in Postio reads the local `mailboxes` table —
//! the sidebar, the sync loop, the watcher, the backfill seed — and until this
//! module nothing ever wrote it outside tests and the seed. On a real account
//! that meant: connect, authenticate, read zero folders, sync zero folders,
//! fetch zero messages, report success. Every layer behaved correctly and the
//! user got an empty mailbox (`postio-755`).
//!
//! # Reconciled by path, never re-created
//!
//! A mailbox id is pointed at by sync state, by every message row, and by every
//! queued operation, so discovery matches on `(account, path)` and *updates*
//! the row it finds. A pass that reinserted would orphan all three, and the
//! symptom — mail that vanishes from a folder that is still there — would look
//! nothing like its cause.
//!
//! For the same reason a folder the server no longer lists is **not deleted**.
//! `messages.mailbox_id` cascades, so one incomplete `LIST` would take the
//! user's mail with it. It is marked unselectable instead, which is both true
//! (a folder that is not there cannot be opened) and exactly what the engine
//! already reads to decide what to sync and watch. It becomes selectable again
//! the moment the server lists it again.
//!
//! And an *empty* listing is never read as "everything is gone". A server that
//! answers `LIST` with nothing, or a listing that failed part way, would
//! otherwise empty the sidebar. Nothing is not evidence of nothing.
//!
//! # Roles come from the server
//!
//! `MailboxRole::resolve` reads the RFC 6154 `\Sent` / `\Drafts` / `\Trash`
//! attributes first and falls back to the name only when the server offers
//! none. That is what makes an account whose Sent folder is called
//! `Sent Messages` — or anything at all in any language — file its mail in the
//! right place.

use postio_account::backend::{MailBackend, MailboxFilter, MailboxSummary};
use postio_model::{AccountId, Mailbox, MailboxId, MailboxRole, RoleOverrides};
use postio_storage::repository::{MailboxRepository, MailboxRoleRepository};
use rusqlite::Connection;

use crate::drain::Result;

/// What one discovery pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryReport {
    /// Folders that were not known locally before this pass.
    pub added: usize,
    /// Folders already known whose description this pass rewrote.
    pub updated: usize,
    /// Folders the server no longer lists, now marked unselectable. Their rows
    /// and their mail are kept.
    pub vanished: usize,
}

impl DiscoveryReport {
    /// How many folders the account has, as far as this pass could tell.
    pub fn known(&self) -> usize {
        self.added + self.updated
    }

    /// Whether the local folder tree moved at all.
    pub fn changed(&self) -> bool {
        self.added > 0 || self.updated > 0 || self.vanished > 0
    }
}

/// Bring `account`'s local folder table in line with the server.
///
/// One `LIST` and a reconciliation; no mailbox is opened and no message is
/// touched. Safe to run on every reconnection, which is what the engine does.
///
/// `overrides` is the configuration tier -- `[mailboxes]`, one table for
/// every account. The account's own map (ADR 0035) is read from the store
/// here, on every pass, and laid over it: nothing about which folder plays
/// which part is frozen at startup, so a choice made in settings is honoured
/// by the next pass with the engine untouched.
pub async fn discover(
    connection: &Connection,
    backend: &dyn MailBackend,
    account: AccountId,
    overrides: &RoleOverrides,
) -> Result<DiscoveryReport> {
    let listed = backend.list_mailboxes(&MailboxFilter::all()).await?;
    let mut report = reconcile(connection, account, &listed, overrides)?;

    // Every reserved role must end this pass with a folder (spec 003, FR-026).
    // Done after reconciling rather than from the listing, because whether a
    // role resolves is the *settled* question -- account map, `[mailboxes]`,
    // SPECIAL-USE, name guess -- and only the rows know the answer.
    //
    // Not on an empty listing, for the reason `retire_missing` is not either:
    // nothing is not evidence of nothing. A server that answers `LIST` with
    // nothing, or a listing that failed part way, would otherwise have five
    // folders created on it -- on an account that may already have all five.
    if !listed.is_empty() && create_missing_roles(connection, backend, account).await? {
        // Re-listed rather than assumed: the server assigns the attributes and
        // the path separator, and a folder Postio believes in but never saw
        // listed is exactly the kind of row that later turns out not to exist.
        let listed = backend.list_mailboxes(&MailboxFilter::all()).await?;
        let second = reconcile(connection, account, &listed, overrides)?;

        // Merged, not replaced, so the report describes the whole pass.
        //
        // The second reconcile sees every folder again, so its `updated` counts
        // the ones the first pass had just added, and its `added` counts only
        // what was created here. Left as-is, a first discovery of an account
        // that needed two folders would report "added 2, updated 4" — which
        // reads as though four folders were already known, on an account that
        // had none a moment ago.
        //
        // `vanished` comes from the first pass alone: it is what the *server's
        // own* listing no longer had, and the second listing is one this
        // function has just changed.
        report = DiscoveryReport {
            added: report.added + second.added,
            updated: second.updated.saturating_sub(report.added),
            vanished: report.vanished,
        };
    }

    Ok(report)
}

/// Creates a folder for every reserved role that resolves to nothing.
///
/// Returns whether anything was created, so the caller knows to re-list.
///
/// # What this will not do
///
/// **Never the Inbox** (FR-029). RFC 3501 names that folder and every server
/// has one; a server that does not is broken in a way this does not paper
/// over, and creating it would be Postio disagreeing with the protocol about
/// where mail arrives.
///
/// **Never a role the server has already refused** (FR-031). A refusal is
/// recorded and honoured, because a server that will never allow a folder must
/// not be asked on every pass forever — which, against a real account, is the
/// expensive version of this bug and a silent one.
///
/// **Never twice.** A role that resolves is not a role that is missing, so the
/// second pass over the same account asks for nothing (SC-007).
///
/// # Why this is not an operation on the queue
///
/// Creating a role's folder is discovery reconciling an account against its
/// server, not a user's mutating action needing an undo. A queue route would
/// need a new `Operation`, an inverse nobody wants (deleting a folder), and a
/// queue row that no keystroke asked for.
async fn create_missing_roles(
    connection: &Connection,
    backend: &dyn MailBackend,
    account: AccountId,
) -> Result<bool> {
    let known = MailboxRepository::new(connection).list_for_account(account)?;
    let roles = MailboxRoleRepository::new(connection);
    let refused: Vec<MailboxRole> = roles
        .refusals(account)?
        .into_iter()
        .map(|(role, _)| role)
        .collect();

    let mut created = false;
    for role in MailboxRole::RESERVED {
        if role == MailboxRole::Inbox || refused.contains(&role) {
            continue;
        }
        // The role's own name. Never a constant naming one provider, and never
        // a branch on the host: providers are data (Principle VII), and a
        // preset that carries a folder name is where a provider-specific
        // answer belongs.
        let path = default_path_for(role);

        // Two ways a role is already answered, and both have to be checked.
        //
        // A folder wearing the role is the obvious one. The second is subtler:
        // a folder the server has stopped listing is *unroled* to `Regular` and
        // marked unselectable rather than deleted, because it may come back
        // (`retire_missing`, and this module's own doc). So a retired Archive
        // no longer *looks* like an archive, and asking only about roles would
        // create a folder at the path that row already occupies — which the
        // next listing would then resurrect, quietly undoing the retirement
        // this module exists to do carefully.
        if known
            .iter()
            .any(|mailbox| mailbox.role == role || mailbox.path == path)
        {
            continue;
        }
        match backend.create_mailbox(path).await {
            Ok(()) => {
                // Ids and outcomes only. The folder name is the role's, so it
                // carries nothing of the user's, but the habit is the point.
                tracing::info!(
                    account = account.get(),
                    role = role.as_str(),
                    "created a folder for a reserved role"
                );
                created = true;
            }
            Err(error) => {
                // Recorded, not fatal. The other roles still resolve and the
                // account stays usable; the user is told in settings, where
                // the server's own words are worth reading, rather than in a
                // log where they would outlive the screen they belong on.
                roles.refuse(account, role, &error.to_string())?;
                tracing::warn!(
                    account = account.get(),
                    role = role.as_str(),
                    "the server refused to create a folder for a reserved role"
                );
            }
        }
    }
    Ok(created)
}

/// What to call a folder created for `role`, absent a preset saying otherwise.
///
/// The role's own name, capitalised the way every provider spells it. `Inbox`
/// never reaches here — it is filtered out above — and the view roles cannot:
/// [`MailboxRole::RESERVED`] holds folders only.
fn default_path_for(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Archive => "Archive",
        MailboxRole::Sent => "Sent",
        MailboxRole::Drafts => "Drafts",
        MailboxRole::Trash => "Trash",
        MailboxRole::Junk => "Junk",
        // Unreachable by construction: `RESERVED` minus `Inbox` is the five
        // above. Named rather than `unreachable!()` so a sixth reserved role
        // gets a name instead of a panic on a user's first sync.
        MailboxRole::Inbox
        | MailboxRole::Flagged
        | MailboxRole::Snoozed
        | MailboxRole::Outbox
        | MailboxRole::Regular => "",
    }
}

/// The local half of [`discover`], with the server's answer already in hand.
///
/// Split out because everything interesting here is a decision about rows
/// rather than about the protocol, and because it lets the whole reconciliation
/// be exercised without a backend at all.
pub fn reconcile(
    connection: &Connection,
    account: AccountId,
    listed: &[MailboxSummary],
    overrides: &RoleOverrides,
) -> std::result::Result<DiscoveryReport, postio_storage::Error> {
    let mailboxes = MailboxRepository::new(connection);
    let chosen = MailboxRoleRepository::new(connection).for_account(account)?;
    let overrides = &overrides.over(chosen);
    let mut report = DiscoveryReport::default();

    for summary in listed {
        match mailboxes.by_path(account, &summary.path)? {
            Some(existing) => {
                let mut updated = existing.clone();
                apply(&mut updated, summary, overrides);
                // Written only when something actually differs: an unchanged
                // folder tree is the common case on every reconnection, and a
                // write per folder per reconnect is a write nobody asked for.
                if updated != existing {
                    mailboxes.update(&updated)?;
                }
                report.updated += 1;
            }
            None => {
                let mut mailbox = Mailbox::new(account, &summary.path, summary.delimiter);
                apply(&mut mailbox, summary, overrides);
                mailboxes.create(&mut mailbox)?;
                report.added += 1;
            }
        }
    }

    // Parents in a second pass: a child can be listed before its parent, and a
    // parent that does not exist yet has no id to point at.
    link_parents(&mailboxes, account, listed)?;

    if !listed.is_empty() {
        report.vanished = retire_missing(&mailboxes, account, listed)?;
    }

    Ok(report)
}

/// Copy what the server said onto a row, leaving everything that is ours.
///
/// Counts, sync state and the local id are deliberately untouched: the server's
/// `LIST` says what folders exist and what they are for, and nothing about what
/// has been synced out of them.
fn apply(mailbox: &mut Mailbox, summary: &MailboxSummary, overrides: &RoleOverrides) {
    let named = Mailbox::new(mailbox.account_id, &summary.path, summary.delimiter);
    mailbox.name = named.name;
    mailbox.path = summary.path.clone();
    mailbox.delimiter = summary.delimiter;
    // The user's own mapping outranks both tiers the summary already went
    // through -- see `RoleOverrides`. Applied here rather than at the IMAP
    // edge, because that layer parses what the *server* said and has no
    // business reading a config file. Written on every reconcile, so a
    // mapping edited between runs takes effect on the next discovery without
    // anything having to notice it changed.
    //
    // On top of `summary.role`, never re-derived from the name: the backend
    // has already settled which of two look-alikes holds a role, and that
    // verdict is the whole reason only one row per role comes out of here.
    mailbox.role = overrides.settle(summary.role, &summary.path);
    mailbox.selectable = summary.selectable;
    mailbox.subscribed = summary.subscribed;
}

/// Point every listed folder at the row for its parent path.
fn link_parents(
    mailboxes: &MailboxRepository<'_>,
    account: AccountId,
    listed: &[MailboxSummary],
) -> std::result::Result<(), postio_storage::Error> {
    for summary in listed {
        let Some(parent_path) = parent_path(summary) else {
            continue;
        };
        let Some(parent) = mailboxes.by_path(account, parent_path)? else {
            // A hierarchy whose intermediate level the server does not list.
            // The folder is still perfectly usable; it just sits at the top.
            continue;
        };
        let Some(mut child) = mailboxes.by_path(account, &summary.path)? else {
            continue;
        };
        if child.parent_id == Some(parent.id) || child.id == parent.id {
            continue;
        }
        child.parent_id = Some(parent.id);
        mailboxes.update(&child)?;
    }
    Ok(())
}

/// The path of a folder's parent, when it has one.
fn parent_path(summary: &MailboxSummary) -> Option<&str> {
    let delimiter = summary.delimiter?;
    let (parent, _) = summary.path.rsplit_once(delimiter)?;
    (!parent.is_empty()).then_some(parent)
}

/// Mark every local folder the server did not list as unopenable.
///
/// Returns how many were retired. See the module docs for why this is not a
/// delete.
fn retire_missing(
    mailboxes: &MailboxRepository<'_>,
    account: AccountId,
    listed: &[MailboxSummary],
) -> std::result::Result<usize, postio_storage::Error> {
    let mut retired = 0;
    for mut local in mailboxes.list_for_account(account)? {
        if listed.iter().any(|summary| summary.path == local.path) {
            continue;
        }
        // A folder the server does not have cannot be where a role files
        // its mail: the role goes with the folder, or `by_role` answers with
        // a row that APPEND will refuse (#943). `INBOX` is exempt -- RFC 3501
        // reserves it, and a listing without it is a broken listing, not a
        // renamed inbox.
        let unrole = local.role.is_special() && local.role != MailboxRole::Inbox;
        if !local.selectable && !unrole {
            // Already retired, or a `\Noselect` level in the hierarchy. Either
            // way there is nothing to write.
            continue;
        }
        local.selectable = false;
        if unrole {
            local.role = MailboxRole::Regular;
        }
        mailboxes.update(&local)?;
        retired += 1;
    }
    Ok(retired)
}

/// The ids of the folders an account can actually open, newest listing first.
///
/// A convenience for the engine, which asks this question after every discovery
/// pass to decide what to sync and what to watch.
pub fn selectable(
    connection: &Connection,
    account: AccountId,
) -> std::result::Result<Vec<MailboxId>, postio_storage::Error> {
    Ok(MailboxRepository::new(connection)
        .list_for_account(account)?
        .into_iter()
        .filter(|mailbox| mailbox.selectable)
        .map(|mailbox| mailbox.id)
        .collect())
}
