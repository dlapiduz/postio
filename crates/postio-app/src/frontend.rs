//! What the window's surfaces hold: a client of the store's owner and the
//! few things beside it that are this process's own.
//!
//! In ordinary use the owner is `postio-daemon`, reached over its socket, and
//! nothing here can open the store (ADR 0041). The integration suites run
//! the owner in this process instead, over a [`Wiring`] they built; the
//! surfaces cannot tell the difference, because everything they ask goes
//! through [`Frontend::client`] either way.

use std::sync::Arc;

use postio_client::Client;
use postio_core::bridge::EventSink;

use crate::Wiring;

/// Everything a window's surfaces need that is not the store.
#[derive(Clone)]
pub struct Frontend {
    /// The window's client of the store's owner: every read and write.
    pub client: Client,
    /// Where this process's own work is polled: client calls that must
    /// not hold the main loop, a part written to disk, a probe.
    pub runtime: tokio::runtime::Handle,
    /// What the window hears: a surface's own sentence -- a part that could
    /// not be saved -- goes here, beside everything the owner says.
    pub events: EventSink,
    /// The keyring, for what this process asks of it itself: the
    /// connection test, and the token-expiry line in the settings.
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    /// Where the connections this process opens itself are recorded: a
    /// discovery probe, a connection test (#151).
    pub egress: Arc<dyn postio_model::egress::EgressSink>,
    /// Where the window's gestures go: to the owner, in the order made.
    pub commands: postio_core::bridge::CommandSender,
    /// `[sync] attachments = "eager"`, which the settings panel shows.
    pub attachments_eager: bool,
    /// The store's owner, when it is in this process: the integration
    /// suites' wiring. `None` over the daemon's socket.
    pub wiring: Option<Wiring>,
}

impl Frontend {
    /// A window over an owner in this process: `client` is a client of a
    /// host over `wiring`, and the rest is the wiring's own.
    pub fn over(wiring: &Wiring, client: Client) -> Frontend {
        Frontend {
            client,
            runtime: wiring.runtime.clone(),
            events: wiring.events.clone(),
            secrets: wiring.secrets.clone(),
            egress: wiring.egress.clone(),
            commands: wiring.commands.clone(),
            attachments_eager: wiring.backfill.attachments
                == postio_runtime::AttachmentPolicy::Eager,
            wiring: Some(wiring.clone()),
        }
    }

    /// A client of its own over the same owner, for a surface that needs one
    /// before the window is fed -- the first-run screen.
    pub fn in_process(wiring: &Wiring) -> Frontend {
        Frontend::over(
            wiring,
            postio_host::Host::over(wiring.clone())
                .connect(postio_client::protocol::ClientKind::Gtk),
        )
    }
}
