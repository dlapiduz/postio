//! What the window's surfaces hold: a client of the store's host and the
//! few things beside it that are this process's own.
//!
//! The host is in this process, over the store this process opened (ADR
//! 0041); the integration suites build it over a [`Wiring`] of their own.
//! Everything a surface reads or writes goes through [`Frontend::client`].

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
    /// not be saved -- goes here, beside everything the host says.
    pub events: EventSink,
    /// The keyring, for what this process asks of it itself: the
    /// connection test, and the token-expiry line in the settings.
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    /// Where the connections this process opens itself are recorded: a
    /// discovery probe, a connection test (#151).
    pub egress: Arc<dyn postio_model::egress::EgressSink>,
    /// Where the window's gestures go: to the host, in the order made.
    pub commands: postio_core::bridge::CommandSender,
    /// `[sync] attachments = "eager"`, which the settings panel shows.
    pub attachments_eager: bool,
    /// The store's wiring: what starts an account's engine and the idle
    /// passes.
    pub wiring: Wiring,
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
            wiring: wiring.clone(),
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
