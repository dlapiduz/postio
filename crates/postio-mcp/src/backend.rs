//! [`Installed`] opens the store per call and closes it after (ADR 0041: one
//! app has the store at a time). [`Held`] serves a host something else keeps.

use std::path::PathBuf;
use std::sync::Arc;

use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_host::{Connector, Host};

pub trait Backend {
    /// `Err` is a sentence for the agent to relay.
    fn lease(&mut self) -> Result<Lease, String>;
}

pub struct Lease {
    client: Option<Client>,
    host: Option<Host>,
    beside_window: bool,
}

impl Lease {
    pub fn client(&self) -> &Client {
        self.client
            .as_ref()
            .expect("held until the lease is dropped")
    }

    pub fn beside_window(&self) -> bool {
        self.beside_window
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

pub struct Installed {
    config: Option<PathBuf>,
    store: PathBuf,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
}

impl Installed {
    pub fn new(
        config: Option<PathBuf>,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
    ) -> Self {
        Installed::at(config, postio_session::paths::store_path(), secrets)
    }

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
            beside_window: false,
        })
    }
}

pub struct Held {
    connector: Connector,
}

impl Held {
    pub fn new(connector: Connector) -> Self {
        Held { connector }
    }
}

impl Backend for Held {
    fn lease(&mut self) -> Result<Lease, String> {
        // Beside the app's window when there is one, so the agent's actions are on its undo (ADR
        // 0041).
        let (client, beside_window) = self.connector.connect_agent();
        Ok(Lease {
            client: Some(client),
            host: None,
            beside_window,
        })
    }
}
