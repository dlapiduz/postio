//! What a window opens on: the account to read, or the first-run screen, by the same rule for every
//! frontend (ADR 0041).

use postio_storage::Store;

/// Called once at the top of [`route`], before anything picks an account or starts an engine (ADR
/// 0005 Q6a), so a crash
/// before the undo toast expires leaves the row as marked. A failed reap is logged and retried next
/// launch.
async fn reap_pending_accounts(database: &Store) {
    let Ok(connection) = database.connect().await else {
        return;
    };
    if let Err(error) = postio_storage::repository::AccountRepository::new(&connection)
        .reap_pending_deletions()
        .await
    {
        tracing::error!(%error, "could not reap an account marked for removal: {error}");
    }
}

/// An account opens only when the store has a row and the keyring gives up its password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupRoute {
    Ready(Box<postio_model::Account>),
    Onboard(Option<Box<postio_model::Account>>),
}

/// Async because the keyring read reaches the Secret Service over D-Bus: poll it on the engine
/// runtime, never on a frontend's
/// main loop. The timeout inside `retrieve` turns silence into an error, which means onboarding.
pub async fn route(
    database: &Store,
    secrets: &dyn postio_account::secret::SecretStore,
) -> StartupRoute {
    reap_pending_accounts(database).await;
    let Some(account) = postio_session::first_account(database).await else {
        return StartupRoute::Onboard(None);
    };
    let key = postio_account::secret::AccountKey::new(account.address.address.clone());
    // The account's domain, never the local part.
    let domain = account.address.domain().unwrap_or("unknown").to_owned();
    match secrets.retrieve(&key).await {
        Ok(password) if !password.is_empty() => StartupRoute::Ready(Box::new(account)),
        Ok(_) => {
            tracing::warn!(%domain, "the keyring holds an empty password; asking for it again");
            StartupRoute::Onboard(Some(Box::new(account)))
        }
        Err(error) => {
            // Safe to log verbatim: no `SecretError` carries a password.
            tracing::warn!(%domain, %error, "no usable password for the account; asking for it again");
            StartupRoute::Onboard(Some(Box::new(account)))
        }
    }
}
