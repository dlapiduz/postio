//! The desktop's settings panels, answered for any frontend; a panel that reads several things asks
//! in one call.
//! Logs carry ids, counts and outcomes only.

use postio_client::protocol::{AccountField, AccountSettings, PrivacyLog};
use postio_model::ids::{AccountId, MailboxId, SignatureId};
use postio_model::listing::StoreError;
use postio_model::mailbox::Mailbox;
use postio_storage::Store;
use postio_storage::repository::{
    AccountRepository, EgressLogRepository, MailboxRepository, MailboxRoleRepository,
    MessageRepository, SettingsRepository, SignatureRepository, UnsubscribeRepository,
};

/// Global, never per account: a second account joining a running installation must not teach the
/// keyboard again (ADR 0012 Q6).
const SEEN_KEY: &str = "orientation_seen";

const ROLES: [postio_model::MailboxRole; 5] = [
    postio_model::MailboxRole::Sent,
    postio_model::MailboxRole::Archive,
    postio_model::MailboxRole::Drafts,
    postio_model::MailboxRole::Trash,
    postio_model::MailboxRole::Junk,
];

/// `list()` rather than the enabled ones: settings is where a disabled account is enabled again. A
/// row pending removal is on its way out and showing it would let it be removed twice. A frontend
/// asks for weights only while the panel is on screen (#871).
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
        // A role's resolution comes from `by_role`, the lookup the send path files a copy through,
        // so "Automatic" cannot disagree with where mail goes.
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

/// Read, change the one field, write the whole row back (#880). A row that has gone is nothing to
/// edit.
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
        AccountField::DefaultSignature(value) => row.default_signature_id = value,
    }
    repository.update(&mut row).await?;
    Ok(())
}

/// An edit leaves the rich variant as it was: the editor is text-only, and rewriting `html` to
/// `None` would discard a signature another tool wrote.
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

/// `idx_signatures_name` is unique on `(account_id, name)`; the constraint's own words are not an
/// answer anybody can act on.
fn explain_signature_failure(name: &str, error: &postio_storage::Error) -> String {
    let raw = error.to_string();
    // The only refusal this form can produce; anything else is a real fault and its own text is
    // more use than a guess.
    if raw.contains("UNIQUE") || raw.contains("constraint") {
        format!("This account already has a signature called “{name}”")
    } else {
        raw
    }
}

pub async fn delete_signature(database: &Store, signature: SignatureId) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    SignatureRepository::new(&connection)
        .delete(signature)
        .await
        .map_err(|error| StoreError::new(error.to_string()))?;
    Ok(())
}

pub async fn egress(
    database: &Store,
    limit: u32,
) -> Result<Vec<postio_model::egress::EgressEvent>, StoreError> {
    let connection = database.read().await?;
    Ok(EgressLogRepository::new(&connection).recent(limit).await?)
}

/// Not account-scoped on screen: the remote-image allow list the same pane shows is one file for
/// every account.
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
    shown
        .activations
        .sort_by_key(|activation| std::cmp::Reverse(activation.activated_at));
    Ok(shown)
}

/// A local column write: no sync pass is involved and nothing announces it. Answers the account's
/// folders as they now stand.
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

pub async fn orientation_seen(database: &Store) -> Result<bool, StoreError> {
    let reader = database.read().await?;
    let seen = SettingsRepository::new(&reader.checkout())
        .get(SEEN_KEY)
        .await?;
    Ok(seen.is_some())
}

pub async fn move_recent(database: &Store) -> Result<Vec<MailboxId>, StoreError> {
    let reader = database.read().await?;
    Ok(SettingsRepository::new(&reader.checkout())
        .move_recent()
        .await?)
}

pub async fn note_move(database: &Store, mailbox: MailboxId) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    SettingsRepository::new(&connection)
        .note_move(mailbox)
        .await?;
    Ok(())
}

/// The value is when, not `"true"`: a row that says only that something happened cannot be
/// debugged.
pub async fn retire_orientation(database: &Store) -> Result<(), StoreError> {
    let connection = database.connect().await?;
    SettingsRepository::new(&connection)
        .set(SEEN_KEY, &chrono::Utc::now().to_rfc3339())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use postio_storage::Store;
    use postio_storage::test_support::{self, counting};

    const SMALL: usize = 200;
    const LARGE: usize = 2_000;

    async fn open_the_panel(database: &Store) {
        let accounts = super::accounts(database, true).await.expect("the accounts");
        assert!(
            accounts.iter().all(|account| account.weight.is_some()),
            "every account is weighed while the panel is on screen"
        );
        super::privacy(database).await.expect("the privacy log");
    }

    async fn seeded(size: usize) -> Store {
        let database = test_support::memory().await;
        postio_storage::seed::seed_large(&database, 7, size).await;
        database
    }

    #[tokio::test]
    async fn opening_settings_reads_a_bounded_amount_however_big_the_mailbox_is() {
        let small = seeded(SMALL).await;
        let large = seeded(LARGE).await;

        let over_small = counting::counted_async(|| open_the_panel(&small)).await;
        let over_large = counting::counted_async(|| open_the_panel(&large)).await;
        // Compiles are left out: the second panel finds the first's in the thread's cache.
        assert_eq!(
            (over_small.statements, over_small.rows),
            (over_large.statements, over_large.rows),
            "opening settings over {LARGE} messages cost {over_large:?} where \
             {SMALL} cost {over_small:?}"
        );

        counting::record();
        open_the_panel(&large).await;
        let issued = counting::recorded();
        assert!(!issued.is_empty(), "nothing was recorded");
        let connection = large.connect().await.expect("a connection");
        let mut walks = Vec::new();
        for sql in issued.keys() {
            let steps = counting::unbounded(&connection, sql, counting::GROWING_TABLES).await;
            if !steps.is_empty() {
                walks.push(format!(
                    "{steps:?}: {}",
                    sql.split_whitespace().collect::<Vec<_>>().join(" ")
                ));
            }
        }
        assert!(
            walks.is_empty(),
            "opening settings reads all of a table that grows with the \
             mailbox: {walks:#?}"
        );
    }
}
