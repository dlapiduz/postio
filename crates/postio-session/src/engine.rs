//! Building the sync engine for an account.
//!
//! Everything here is a choice about *this* installation rather than about
//! how mail works: which TLS stack dials the server, where the password is
//! kept, how many connections to hold open. That is what a composition root
//! is for, and it is why `postio-runtime` takes these as parts rather than
//! constructing them — a test hands it a mock backend and an in-memory
//! keyring through the same door.
//!
//! # Nothing here dials anything
//!
//! `ImapBackend::new` builds a pool and opens no connection; the engine's
//! supervisor decides when to. So a Postio started with no network, or with
//! a password the keyring will not give up, still opens its window and says
//! so — which is the whole point of being local-first.
//!
//! [`NetworkSource::NetworkManager`] is asked for here rather than defaulted
//! to inside the engine, because a listener that opened a system-bus
//! connection on its own would make every test behave differently on a
//! desktop and on a CI runner. A machine without NetworkManager loses nothing
//! but the promptness of a reconnect.

use std::sync::Arc;

use postio_imap::backend::MailBackend;
use postio_imap::imap::{ConnectionSettings, ImapBackend, PoolConfig, RustlsConnector};
use postio_imap::secret::{AccountKey, SecretStore};
use postio_model::Account;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource};
use postio_storage::{BlobStore, Database};

use postio_core::bridge::EventSink;

/// Start the engine for `account`.
///
/// `None` when the transports cannot be built at all — a system with no
/// usable TLS stack, say. That costs the account its sync and nothing else:
/// the local store still opens and everything already synced still reads.
///
/// `secrets` is handed in rather than built here for the reason the module
/// docs give: it is the composition root's choice, and it is the same store
/// onboarding writes the password into and startup reads it back from.
pub fn start(
    account: &Account,
    database: &Database,
    blobs: BlobStore,
    events: EventSink,
    secrets: Arc<dyn SecretStore>,
    mailbox_roles: postio_model::RoleOverrides,
    backfill: postio_runtime::BackfillPolicy,
) -> Option<Engine> {
    let key = AccountKey::new(account.address.address.clone());

    let connector = match RustlsConnector::new() {
        Ok(connector) => Arc::new(connector),
        Err(error) => {
            tracing::error!(%error, "no IMAP transport, so no sync");
            return None;
        }
    };
    let smtp = match postio_smtp::transport::RustlsConnector::new() {
        Ok(connector) => Arc::new(connector),
        Err(error) => {
            tracing::error!(%error, "no SMTP transport, so nothing can be sent");
            return None;
        }
    };

    let backend: Arc<dyn MailBackend> = Arc::new(ImapBackend::new(
        settings(&account.incoming),
        key,
        secrets.clone(),
        connector,
        PoolConfig::default(),
    ));

    match Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
        smtp,
        secrets,
        events,
        retry: Default::default(),
        backfill,
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::NetworkManager,
        mailbox_roles,
    }) {
        Ok(engine) => Some(engine),
        Err(error) => {
            tracing::error!(%error, "the sync engine did not start");
            None
        }
    }
}

/// The account's IMAP server, as the connection pool wants it.
///
/// Field for field. The two types exist separately because `postio-model`
/// describes an account and `postio-imap` describes a connection, and neither
/// should have to change when the other does.
fn settings(server: &postio_model::account::ServerConfig) -> ConnectionSettings {
    ConnectionSettings::new(
        server.host.clone(),
        server.port,
        server.security,
        server.username.clone(),
    )
}
