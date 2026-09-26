//! What a window opens on: the account to read, or the first-run screen.
//!
//! The store's owner's question, since only it reads the store and the
//! keyring (ADR 0041); moved here from the desktop app so every frontend is
//! routed by the same rule.

use postio_storage::Store;

/// Deletes every account "Remove" (in the settings panel, #464) has marked
/// but not yet actually removed, cascading to its mail.
///
/// Called once, at the top of [`route`], before anything decides
/// which account to open or starts an engine for one — the boundary ADR
/// 0005 Q6a chose specifically so a crash before the undo toast expires
/// leaves the row exactly as marked, not half deleted. Failing to read or
/// write is logged and otherwise ignored: a reap that cannot run this
/// launch gets another chance next launch, and the account stays out of
/// `list_enabled` either way.
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

/// What a window opens on, as the store's owner decides it.
///
/// An account is something to open only when the store holds a row **and**
/// the keyring gives up a password for it; otherwise the first-run screen,
/// prefilled from the row when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupRoute {
    /// Open it: there is a row, and a password to authenticate with.
    Ready(Box<postio_model::Account>),
    /// Show the first-run screen; `Some` is a repair of this account.
    Onboard(Option<Box<postio_model::Account>>),
}

/// Decide which of the two startup does.
///
/// Async because reading the keyring is: `KeyringSecretStore` reaches the
/// Secret Service over D-Bus and bounds the round trip with a timeout, so
/// this must be polled on the engine runtime and answered over a channel —
/// never awaited on a frontend's main loop.
///
/// A keyring that will not answer therefore costs the window a moment, not
/// the session: the timeout inside `retrieve` turns silence into an error,
/// and an error means onboarding rather than a window that never decides.
pub async fn route(
    database: &Store,
    secrets: &dyn postio_account::secret::SecretStore,
) -> StartupRoute {
    reap_pending_accounts(database).await;
    let Some(account) = postio_session::first_account(database).await else {
        return StartupRoute::Onboard(None);
    };
    let key = postio_account::secret::AccountKey::new(account.address.address.clone());
    // The account's domain, never the local part, for the same reason
    // the desktop app's `feed_the_window` logs only that.
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
