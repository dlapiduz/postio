//! Where a tool call's store comes from.
//!
//! [`Installed`] opens this installation's store for each call and closes
//! it after, so the agent holds it only while it is asking (ADR 0041: one
//! app has the store at a time). [`Held`] serves a host something else
//! started and keeps: a demo store, or a suite's.

use std::path::PathBuf;
use std::sync::Arc;

use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_host::Host;

/// What a tool call reads and writes through.
pub trait Backend {
    /// The store, for one call. `Err` is a sentence for the agent to relay:
    /// the desktop app has the store, the keyring said no.
    fn lease(&mut self) -> Result<Lease, String>;
}

/// The store for one call: a client of a host, and the host itself when the
/// call opened it, stopped when the lease is dropped.
pub struct Lease {
    client: Option<Client>,
    host: Option<Host>,
}

impl Lease {
    /// The client a call reads and writes through.
    pub fn client(&self) -> &Client {
        self.client.as_ref().expect("held until the lease is dropped")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        // The client first: it is the host's, and nothing should be asking
        // when the host closes the store.
        self.client.take();
        if let Some(host) = self.host.take() {
            host.stop();
        }
    }
}

/// This installation's store, opened with the key in the OS keyring for
/// each call and closed after it.
pub struct Installed {
    config: Option<PathBuf>,
    store: PathBuf,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
}

impl Installed {
    /// This installation's store, wired as `config.toml` at `config` says,
    /// its key in `secrets`.
    pub fn new(
        config: Option<PathBuf>,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Installed::at(config, postio_session::paths::store_path(), secrets)
    }

    /// [`Installed::new`], over the store at `store`.
    pub fn at(
        config: Option<PathBuf>,
        store: PathBuf,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Installed {
            config,
            store,
            secrets,
        }
    }
}

impl Backend for Installed {
    fn lease(&mut self) -> Result<Lease, String> {
        let host = Host::open_at(
            self.config.as_deref(),
            &self.store,
            Arc::clone(&self.secrets),
            &|_| {},
        )
            .map_err(|refusal| refusal.to_string())?;
        let client = host.connect(ClientKind::Mcp);
        Ok(Lease {
            client: Some(client),
            host: Some(host),
        })
    }
}

/// A host started elsewhere and kept for the life of the server.
pub struct Held {
    host: Arc<Host>,
}

impl Held {
    /// Serve `host`.
    pub fn new(host: Arc<Host>) -> Self {
        Held { host }
    }
}

impl Backend for Held {
    fn lease(&mut self) -> Result<Lease, String> {
        Ok(Lease {
            client: Some(self.host.connect(ClientKind::Mcp)),
            host: None,
        })
    }
}
