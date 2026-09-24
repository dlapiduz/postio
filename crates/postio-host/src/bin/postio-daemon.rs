//! `postio-daemon`: the store's one owner, serving frontends over a socket.
//!
//! Started by the first frontend that finds nothing listening, never by hand
//! in ordinary use. It listens before the store is open and says "starting"
//! until it is; opens the store with the key from the keyring; starts sync;
//! and exits once no frontend has been connected for thirty seconds
//! (research R1b), stopping the engines first. A second one finds the lock
//! taken and exits at once, leaving the first alone.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use postio_client::socket::Endpoint;
use postio_host::Host;
use postio_host::serve::{BindError, bind};
use postio_session::logging;

/// How long the daemon outlives its last frontend. Long enough to quit one
/// frontend and open the other without the store closing between them.
const GRACE: Duration = Duration::from_secs(30);

/// How long after the store opens its upkeep starts: the desktop app's
/// delay after its first frame, so a frontend's first pages come first.
const IDLE_PASSES_AFTER_OPENING: Duration = Duration::from_millis(750);

const USAGE: &str = "usage: postio-daemon [--runtime-dir DIR] [--version]";

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let mut runtime_dir = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--version" => {
                println!(
                    "postio-daemon {}",
                    postio_client::protocol::BuildId::current()
                );
                return ExitCode::SUCCESS;
            }
            "--runtime-dir" => runtime_dir = arguments.next(),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }

    let config_path = postio_config::paths::config_path().ok();
    let logging = logging::init(
        &config_path
            .as_deref()
            .map(logging::config_at)
            .unwrap_or_default(),
    );
    let _log_watch = config_path.as_deref().and_then(|path| logging.watch(path));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "postio-daemon starting"
    );

    let endpoint = match runtime_dir {
        Some(dir) => Endpoint::at(dir),
        None => match Endpoint::from_env() {
            Ok(endpoint) => endpoint,
            Err(error) => {
                eprintln!("postio-daemon: {error}");
                return ExitCode::FAILURE;
            }
        },
    };
    let listener = match bind(&endpoint) {
        Ok(listener) => listener,
        Err(BindError::AlreadyRunning) => {
            tracing::info!("another daemon owns the store; leaving it be");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            tracing::error!(%error, "cannot listen: {error}");
            eprintln!("postio-daemon: {error}");
            return ExitCode::FAILURE;
        }
    };

    let ready = AtomicBool::new(false);
    let host = std::thread::scope(|scope| {
        scope.spawn(|| listener.answer_starting_until(&ready));
        let host = open(config_path.as_deref());
        // Whatever happened, the frontends waiting on "starting" stop waiting.
        ready.store(true, Ordering::Relaxed);
        host
    });
    let host = match host {
        Ok(host) => host,
        Err(reason) => {
            tracing::error!(reason, "the store did not open");
            eprintln!("postio-daemon: {reason}");
            return ExitCode::FAILURE;
        }
    };

    host.start_syncing();
    // The store's upkeep -- the body index, the header repair, the disk
    // reclaim -- which the desktop app ran after its first frame when it
    // owned the store; a moment after opening here, for the same reason.
    host.start_idle_passes_after(IDLE_PASSES_AFTER_OPENING);
    host.serve(listener, GRACE);
    host.stop();
    tracing::info!("postio-daemon stopped");
    ExitCode::SUCCESS
}

/// Read the key, open the store, and wire it as `config.toml` asks.
fn open(config_path: Option<&std::path::Path>) -> Result<Host, String> {
    let secrets: std::sync::Arc<dyn postio_account::secret::SecretStore> =
        std::sync::Arc::new(postio_account::secret::KeyringSecretStore::default());
    let key = postio_session::store_key_blocking(secrets.as_ref()).map_err(|e| e.to_string())?;
    let (database, blobs) = {
        // Its own runtime, dropped before the host's exists: opening the store
        // is async, and nothing else is running yet to host it.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| {
                format!("Postio could not start the worker that opens its store: {error}")
            })?;
        runtime.block_on(postio_session::open_store_reporting(&key, &|_| {}))?
    };

    let sync_config = config_path
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .map(|config| config.sync)
        .unwrap_or_default();
    let mailbox_roles = config_path
        .map(postio_session::mailbox_roles_at)
        .unwrap_or_default();
    let storage_ceiling = config_path.and_then(postio_session::storage_ceiling_at);

    let host = Host::start(database, blobs, |wiring| {
        wiring
            .with_mailbox_roles(mailbox_roles)
            .with_backfill(postio_session::backfill_policy(&sync_config))
            .with_watch(postio_session::watch_policy(&sync_config))
            .with_storage_ceiling(storage_ceiling)
            .with_secrets(secrets)
    })?;
    // Which folders' arrivals raise a notification, for whichever frontend
    // is elected to deliver them.
    host.notify_with(sync_config);
    Ok(host)
}
