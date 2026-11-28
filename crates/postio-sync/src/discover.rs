//! Reconciles the server's folder list into local rows by `(account, path)`, updating and never
//! re-creating: mailbox ids are referenced by messages, sync state and queued operations.
//! A folder the server stops listing is marked unselectable, not deleted (`messages.mailbox_id`
//! cascades), and an empty listing is never read as "everything is gone".
//! Roles come from RFC 6154 attributes first, the name only as fallback.

use postio_account::backend::{MailBackend, MailboxFilter, MailboxSummary};
use postio_model::{AccountId, Mailbox, MailboxId, MailboxRole, RoleOverrides};
use postio_storage::Connection;
use postio_storage::repository::{MailboxRepository, MailboxRoleRepository};

use crate::drain::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub added: usize,
    pub updated: usize,
    pub vanished: usize,
}

impl DiscoveryReport {
    pub fn known(&self) -> usize {
        self.added + self.updated
    }

    pub fn changed(&self) -> bool {
        self.added > 0 || self.updated > 0 || self.vanished > 0
    }
}

/// One `LIST` and a reconciliation; opens no mailbox and touches no message. Safe on every
/// reconnection.
/// `overrides` is the `[mailboxes]` config tier; the account's own map (ADR 0035) is read from the
/// store on every pass and laid over it.
pub async fn discover(
    connection: &Connection,
    backend: &dyn MailBackend,
    account: AccountId,
    overrides: &RoleOverrides,
) -> Result<DiscoveryReport> {
    let listed = backend.list_mailboxes(&MailboxFilter::all()).await?;
    let mut report = reconcile(connection, account, &listed, overrides).await?;

    // Every reserved role must end this pass with a folder (spec 003, FR-026). Done after
    // reconciling because only the rows know the settled answer.
    // Not on an empty listing: nothing is not evidence of nothing, and five folders could be
    // created on an account that has them.
    if !listed.is_empty() && create_missing_roles(connection, backend, account).await? {
        // Re-listed, not assumed: the server assigns attributes and the separator.
        let listed = backend.list_mailboxes(&MailboxFilter::all()).await?;
        let second = reconcile(connection, account, &listed, overrides).await?;

        // Merged so the report covers the whole pass: the second reconcile's `updated` counts what
        // the first just added.
        // `vanished` comes from the first pass alone, the server's own listing.
        report = DiscoveryReport {
            added: report.added + second.added,
            updated: second.updated.saturating_sub(report.added),
            vanished: report.vanished,
        };
    }

    Ok(report)
}

/// Creates a folder for every reserved role that resolves to nothing; returns whether anything was
/// created.
/// Never the Inbox (FR-029). Never a role the server already refused (FR-031): a refusal is
/// recorded and honoured, not retried every pass.
/// Not a queue operation: this is discovery reconciling, with no user action to undo.
async fn create_missing_roles(
    connection: &Connection,
    backend: &dyn MailBackend,
    account: AccountId,
) -> Result<bool> {
    let known = MailboxRepository::new(connection)
        .list_for_account(account)
        .await?;
    let roles = MailboxRoleRepository::new(connection);
    let refused: Vec<MailboxRole> = roles
        .refusals(account)
        .await?
        .into_iter()
        .map(|(role, _)| role)
        .collect();

    let mut created = false;
    for role in MailboxRole::RESERVED {
        if role == MailboxRole::Inbox || refused.contains(&role) {
            continue;
        }
        let path = default_path_for(role);

        // Check both role and path: a retired folder is unroled to `Regular`, so asking only about
        // roles would create a folder at its path and the next listing would resurrect it.
        if known
            .iter()
            .any(|mailbox| mailbox.role == role || mailbox.path == path)
        {
            continue;
        }
        match backend.create_mailbox(path).await {
            Ok(()) => {
                tracing::info!(
                    account = account.get(),
                    role = role.as_str(),
                    "created a folder for a reserved role"
                );
                created = true;
            }
            Err(error) => {
                // Recorded, not fatal: other roles still resolve; the user sees the server's words
                // in settings.
                roles.refuse(account, role, &error.to_string()).await?;
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

fn default_path_for(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Archive => "Archive",
        MailboxRole::Sent => "Sent",
        MailboxRole::Drafts => "Drafts",
        MailboxRole::Trash => "Trash",
        MailboxRole::Junk => "Junk",
        // Named rather than `unreachable!()` so a sixth reserved role does not panic on a first
        // sync.
        MailboxRole::Inbox
        | MailboxRole::Flagged
        | MailboxRole::Snoozed
        | MailboxRole::Outbox
        | MailboxRole::Regular => "",
    }
}

/// The local half of `discover`, with the server's answer in hand, so it runs without a backend.
pub async fn reconcile(
    connection: &Connection,
    account: AccountId,
    listed: &[MailboxSummary],
    overrides: &RoleOverrides,
) -> std::result::Result<DiscoveryReport, postio_storage::Error> {
    let mailboxes = MailboxRepository::new(connection);
    let chosen = MailboxRoleRepository::new(connection)
        .for_account(account)
        .await?;
    let overrides = &overrides.over(chosen);
    let mut report = DiscoveryReport::default();

    for summary in listed {
        match mailboxes.by_path(account, &summary.path).await? {
            Some(existing) => {
                let mut updated = existing.clone();
                apply(&mut updated, summary, overrides);
                // Written only when something differs: a write per folder per reconnect is a write
                // nobody asked for.
                if updated != existing {
                    mailboxes.update(&updated).await?;
                }
                report.updated += 1;
            }
            None => {
                let mut mailbox = Mailbox::new(account, &summary.path, summary.delimiter);
                apply(&mut mailbox, summary, overrides);
                mailboxes.create(&mut mailbox).await?;
                report.added += 1;
            }
        }
    }

    // Parents in a second pass: a child can be listed before its parent exists.
    link_parents(&mailboxes, account, listed).await?;

    if !listed.is_empty() {
        report.vanished = retire_missing(&mailboxes, account, listed).await?;
    }

    Ok(report)
}

/// Copies what the server said onto a row; counts, sync state and the local id are untouched.
fn apply(mailbox: &mut Mailbox, summary: &MailboxSummary, overrides: &RoleOverrides) {
    let named = Mailbox::new(mailbox.account_id, &summary.path, summary.delimiter);
    mailbox.name = named.name;
    mailbox.path = summary.path.clone();
    mailbox.delimiter = summary.delimiter;
    // The user's mapping outranks both tiers the summary went through; applied here, not at the
    // IMAP edge, which has no business reading config.
    // Applied on top of `summary.role`, never re-derived from the name: the backend already settled
    // which look-alike holds a role.
    mailbox.role = overrides.settle(summary.role, &summary.path);
    mailbox.selectable = summary.selectable;
    mailbox.subscribed = summary.subscribed;
}

async fn link_parents(
    mailboxes: &MailboxRepository<'_>,
    account: AccountId,
    listed: &[MailboxSummary],
) -> std::result::Result<(), postio_storage::Error> {
    for summary in listed {
        let Some(parent_path) = parent_path(summary) else {
            continue;
        };
        let Some(parent) = mailboxes.by_path(account, parent_path).await? else {
            // The server does not list an intermediate level; the folder sits at the top.
            continue;
        };
        let Some(mut child) = mailboxes.by_path(account, &summary.path).await? else {
            continue;
        };
        if child.parent_id == Some(parent.id) || child.id == parent.id {
            continue;
        }
        child.parent_id = Some(parent.id);
        mailboxes.update(&child).await?;
    }
    Ok(())
}

fn parent_path(summary: &MailboxSummary) -> Option<&str> {
    let delimiter = summary.delimiter?;
    let (parent, _) = summary.path.rsplit_once(delimiter)?;
    (!parent.is_empty()).then_some(parent)
}

/// Marks every local folder the server did not list as unopenable; returns how many. Not a delete
/// (see module docs).
async fn retire_missing(
    mailboxes: &MailboxRepository<'_>,
    account: AccountId,
    listed: &[MailboxSummary],
) -> std::result::Result<usize, postio_storage::Error> {
    let mut retired = 0;
    for mut local in mailboxes.list_for_account(account).await? {
        if listed.iter().any(|summary| summary.path == local.path) {
            continue;
        }
        // A folder the server lacks cannot file a role's mail, or `by_role` answers with a row
        // APPEND refuses (#943). `INBOX` is exempt (RFC 3501).
        let unrole = local.role.is_special() && local.role != MailboxRole::Inbox;
        if !local.selectable && !unrole {
            continue;
        }
        local.selectable = false;
        if unrole {
            local.role = MailboxRole::Regular;
        }
        mailboxes.update(&local).await?;
        retired += 1;
    }
    Ok(retired)
}

pub async fn selectable(
    connection: &Connection,
    account: AccountId,
) -> std::result::Result<Vec<MailboxId>, postio_storage::Error> {
    Ok(MailboxRepository::new(connection)
        .list_for_account(account)
        .await?
        .into_iter()
        .filter(|mailbox| mailbox.selectable)
        .map(|mailbox| mailbox.id)
        .collect())
}
