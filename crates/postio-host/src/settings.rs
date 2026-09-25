//! The desktop's settings panels, answered for any frontend.
//!
//! Moved from `postio-app`'s `settings_accounts`, `settings_privacy`,
//! `settings_egress`, `sidebar_backfill` and `orientation`
//! (`specs/005-tui-frontend` T018). Each read the store itself, a repository
//! at a time; each read is one request here, and a panel that read several
//! things at once -- every account's folders, roles and weight -- asks for
//! them in one call.
//!
//! Logs here carry ids, counts and outcomes only.

use postio_client::protocol::{AccountField, AccountSettings, PrivacyLog};
use postio_model::ids::{AccountId, MailboxId, SignatureId};
use postio_model::listing::StoreError;
use postio_model::mailbox::Mailbox;
use postio_storage::Store;
use postio_storage::repository::{
    AccountRepository, EgressLogRepository, MailboxRepository, MailboxRoleRepository,
    MessageRepository, SettingsRepository, SignatureRepository, UnsubscribeRepository,
};

/// The `settings` row the first-run keyboard orientation owns.
///
/// Global, never per account: ADR 0012 Q6 -- a second account joining a
/// running installation must not teach the keyboard again.
const SEEN_KEY: &str = "orientation_seen";

/// The roles the detail view names a folder for.
const ROLES: [postio_model::MailboxRole; 5] = [
    postio_model::MailboxRole::Sent,
    postio_model::MailboxRole::Archive,
    postio_model::MailboxRole::Drafts,
    postio_model::MailboxRole::Trash,
    postio_model::MailboxRole::Junk,
];

/// Every account the settings show, with its folders and role map, and --
/// when `weights` -- what its mail weighs.
///
/// `list()` rather than the enabled ones: settings is where a disabled
/// account is enabled again. A row pending removal is on its way out, and
/// showing it would let it be removed twice.
///
/// The weights are `count(*)` and `sum(size)` over every message an account
/// has -- 1.48s on a real store -- so a frontend asks for them only while
/// the panel is on screen (#871).
pub async fn accounts(database: &Store, weights: bool) -> Result<Vec<AccountSettings>, StoreError> {
    let connection = database.read().await?;
    let accounts = AccountRepository::new(&connection).list().await?;
    let messages = MessageRepository::new(&connection);
    let mailboxes = MailboxRepository::new(&connection);
    let roles = MailboxRoleRepository::new(&connection);
    let mut shown = Vec::with_capacity(accounts.len());
    for account in accounts
        .into_iter()
        .filter(|account| !account.pending_deletion)
    {
        let id = account.id;
        let weight = if weights {
            messages
                .footprint(id)
                .await
                .inspect_err(|error| tracing::warn!(%error, "could not measure an account's mail"))
                .ok()
                .map(|footprint| postio_core::event::MailFootprint {
                    total_bytes: footprint.total_bytes,
                    attachment_bytes: footprint.attachment_bytes,
                    local_bytes: footprint.local_bytes,
                    complete: footprint.complete,
                })
        } else {
            None
        };
        // Three questions, three reads: what there is to choose from, what
        // was chosen, and what each role resolves to as things stand -- the
        // last from `by_role`, the lookup the send path files a copy
        // through, so "Automatic" cannot disagree with where mail goes.
        let folders = mailboxes
            .list_for_account(id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|mailbox| mailbox.selectable)
            .map(|mailbox| mailbox.path)
            .collect();
        let chosen = roles.for_account(id).await.unwrap_or_default();
        let mut resolved = Vec::new();
        for role in ROLES {
            if let Ok(Some(mailbox)) = mailboxes.by_role(id, role).await {
                resolved.push((role, mailbox.path));
            }
        }
        let refused = roles.refusals(id).await.unwrap_or_default();
        shown.push(AccountSettings {
            account,
            folders,
            chosen,
            resolved,
            refused,
            weight,
        });
    }
    Ok(shown)
}

/// Apply one field's new value to `account`'s row: read it, change the one
/// field, write the whole row back (#880). A row that has gone is nothing
/// to edit.
pub async fn edit(
    database: &Store,
    account: AccountId,
    field: AccountField,
) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    let repository = AccountRepository::new(&connection);
    let Some(mut row) = repository.get(account).await? else {
        return Ok(());
    };
    match field {
        AccountField::DisplayName(value) => row.display_name = value,
        AccountField::ImapHost(value) => row.incoming.host = value,
        AccountField::ImapPort(value) => row.incoming.port = value,
        AccountField::SmtpHost(value) => row.outgoing.host = value,
        AccountField::SmtpPort(value) => row.outgoing.port = value,
        // #979: an account may have signatures and prefer none of them.
        AccountField::DefaultSignature(value) => row.default_signature_id = value,
    }
    repository.update(&mut row).await?;
    Ok(())
}

/// Write a signature, new or edited (#1086). The error is the sentence for
/// the person who typed it.
///
/// An edit leaves the rich variant exactly as it was: the settings' editor
/// is text-only, and rewriting `html` to `None` would quietly discard a
/// signature somebody else's tooling wrote.
pub async fn save_signature(
    database: &Store,
    account: AccountId,
    signature: Option<SignatureId>,
    name: &str,
    text: &str,
) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    let signatures = SignatureRepository::new(&connection);
    let written = match signature {
        Some(existing) => {
            let mut edited = postio_model::Signature::new(name, text);
            edited.id = existing;
            let previous = signatures
                .list_for_account(account)
                .await
                .ok()
                .and_then(|all| all.into_iter().find(|stored| stored.id == existing));
            if let Some(previous) = previous {
                edited.html = previous.html;
            }
            signatures.update(&edited).await
        }
        None => {
            let mut created = postio_model::Signature::new(name, text);
            signatures.create(account, &mut created).await.map(|_| ())
        }
    };
    written.map_err(|error| StoreError::new(explain_signature_failure(name, &error)))
}

/// A store refusal, as the person who typed the signature needs to read it.
///
/// `idx_signatures_name` is a unique index on `(account_id, name)`, so a
/// second "Work" fails, and the constraint's own words are not an answer
/// anybody can act on.
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

/// Remove a signature. An account whose default it was is left consistent by
/// the schema (`ON DELETE SET NULL`).
pub async fn delete_signature(database: &Store, signature: SignatureId) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    SignatureRepository::new(&connection)
        .delete(signature)
        .await
        .map_err(|error| StoreError::new(error.to_string()))?;
    Ok(())
}

/// The newest `limit` outbound connections (#151).
pub async fn egress(
    database: &Store,
    limit: u32,
) -> Result<Vec<postio_model::egress::EgressEvent>, StoreError> {
    let connection = database.read().await?;
    Ok(EgressLogRepository::new(&connection).recent(limit).await?)
}

/// Every account's unsubscribe activations, newest first (#971), and how
/// many messages asked for a read receipt (#970).
///
/// Not account-scoped on screen: the remote-image allow list the same pane
/// shows is one file for every account, and these follow its shape.
pub async fn privacy(database: &Store) -> Result<PrivacyLog, StoreError> {
    let connection = database.read().await?;
    let accounts = AccountRepository::new(&connection)
        .list()
        .await
        .unwrap_or_default();
    let log = UnsubscribeRepository::new(&connection);
    let messages = MessageRepository::new(&connection);
    let mut shown = PrivacyLog::default();
    for account in &accounts {
        match log.for_account(account.id).await {
            Ok(activations) => shown.activations.extend(activations),
            Err(error) => tracing::warn!(%error, "could not read the unsubscribe-activation log"),
        }
        match messages.read_receipt_requested_count(account.id).await {
            Ok(count) => shown.read_receipts += count,
            Err(error) => tracing::warn!(%error, "could not count read-receipt requests"),
        }
    }
    // Each account's rows come back newest first; more than one account
    // means sorting the combined list the same way.
    shown
        .activations
        .sort_by_key(|activation| std::cmp::Reverse(activation.activated_at));
    Ok(shown)
}

/// Skip or resume `mailbox`'s background backfill (ADR 0016, #350), and
/// answer its account's folders as they now stand, so the menu's wording is
/// right the moment it is reopened. A local column write: no sync pass is
/// involved, and nothing announces it.
pub async fn set_backfill_excluded(
    database: &Store,
    mailbox: MailboxId,
    excluded: bool,
) -> Result<Vec<Mailbox>, StoreError> {
    let connection = database.connect().await?;
    let mailboxes = MailboxRepository::new(&connection);
    mailboxes.set_backfill_excluded(mailbox, excluded).await?;
    let Some(folder) = mailboxes.get(mailbox).await? else {
        return Ok(Vec::new());
    };
    Ok(mailboxes.list_for_account(folder.account_id).await?)
}

/// Whether some earlier run already showed the keyboard orientation.
pub async fn orientation_seen(database: &Store) -> Result<bool, StoreError> {
    let reader = database.read().await?;
    let seen = SettingsRepository::new(&reader.checkout())
        .get(SEEN_KEY)
        .await?;
    Ok(seen.is_some())
}

/// Write down that this installation is done with the orientation.
///
/// The value is when, rather than `"true"`: a row that says only that
/// something happened is a row nobody can ever debug.
pub async fn retire_orientation(database: &Store) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    SettingsRepository::new(&connection)
        .set(SEEN_KEY, &chrono::Utc::now().to_rfc3339())
        .await?;
    Ok(())
}
