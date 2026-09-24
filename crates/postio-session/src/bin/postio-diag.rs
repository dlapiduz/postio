//! `postio-diag`: what a store holds, what it costs, and what the background
//! lanes still owe -- counts, sizes, ids and header tokens, never mail.
//!
//! Every diagnosis this project has needed against a real store began with a
//! one-off example: a census of the tables, a census of the bodies carrying
//! the decode caveat, the shape of the messages table, a search timed on a
//! warm connection. Each answered its question and then sat in an
//! `examples/` directory waiting for the next person to rediscover it. This
//! is those examples as one command, built with the application, so the
//! next question starts from `postio-diag` rather than from a blank file.
//!
//! ```text
//! postio-diag census                      # asks the daemon, or opens the store
//! postio-diag --store /path/to/copy.db pending
//! POSTIO_STORE_KEY=<hex> postio-diag --store copy.db encoding
//! ```
//!
//! **With Postio running, the daemon answers.** It owns the store while it
//! runs (ADR 0041), so the reports are asked of it over its socket and run
//! on its own connection: nothing else opens the file. With nothing running,
//! the store is opened here.
//!
//! **Read-only by promise, not by mode.** The engine has no read-only open
//! (ADR 0038), so every subcommand issues `SELECT` and `PRAGMA` and nothing
//! else; what a copy protects against is the engine's own recovery on open.
//!
//! The key comes from the keyring, the way the application and
//! `postio-provision` read it, or from `POSTIO_STORE_KEY` for a copy whose
//! keyring is somewhere else. The key is never printed.

use std::process::ExitCode;

use postio_account::secret::platform_keyring;
use postio_storage::Store;
use postio_storage::key::{Purpose, StoreKey};

const USAGE: &str = "\
usage: postio-diag [--store <postio.db>] <command>

  census    what the store holds and what it costs: tables, bytes, folders
  encoding  the bodies carrying the decode caveat, and what their parts declared
  shape     rows per page, with and without the body columns
  pending   what the background lanes still owe: backfill, index, queue

Reads only. With Postio running, its daemon answers; otherwise the store is
opened, or the copy --store names. The key comes from the keyring, or
POSTIO_STORE_KEY.";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let mut path: Option<std::path::PathBuf> = None;
    let mut command: Option<String> = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--store" => path = arguments.next().map(Into::into),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if command.is_none() => command = Some(other.to_owned()),
            other => {
                eprintln!("postio-diag: unexpected argument {other:?}\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(command) = command else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    // The live store belongs to the daemon while it runs: ask it.
    if path.is_none()
        && let Ok(endpoint) = postio_client::socket::Endpoint::from_env()
        && let Ok(client) =
            postio_client::socket::connect(&endpoint, postio_client::protocol::ClientKind::Test)
    {
        return match client.diagnose(command.clone()).await {
            Ok(text) => {
                eprintln!("postio-diag: asked the running daemon");
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("postio-diag: {}", error.message());
                ExitCode::FAILURE
            }
        };
    }
    let path = path.unwrap_or_else(|| {
        eprintln!("postio-diag: nothing running; reading the live store");
        postio_session::paths::store_path()
    });
    let master = match std::env::var("POSTIO_STORE_KEY") {
        Ok(hex) => match StoreKey::from_hex(hex.trim()) {
            Ok(key) => key,
            Err(error) => {
                eprintln!("postio-diag: POSTIO_STORE_KEY is not a store key: {error}");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => match postio_session::store_key_blocking(platform_keyring().as_ref()) {
            Ok(key) => key,
            Err(error) => {
                eprintln!(
                    "postio-diag: cannot read the store key (is the keyring unlocked?): {error}"
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let file = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let store = match Store::open(&path, &master.derive(Purpose::Database)).await {
        Ok(store) => store,
        Err(error) => {
            eprintln!("postio-diag: cannot open {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let connection = match store.connect().await {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("postio-diag: cannot connect: {error}");
            return ExitCode::FAILURE;
        }
    };
    match postio_session::diag::report(connection, file, &command).await {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("postio-diag: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
