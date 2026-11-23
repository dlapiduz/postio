//! `postio-diag`: what a store holds, what it costs, and what the background lanes still owe --
//! counts, sizes and ids, never mail.
//!
//! With Postio running the live store is refused (ADR 0041); every subcommand issues only `SELECT`
//! and `PRAGMA`. The key never prints.

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
  classify  the classifier over every inbox, with the config's rules: counts and ids
  search <words>  the search box's query, timed, as typed and quoted

Reads only. Opens the live store -- close Postio first -- or the copy --store
names. The key comes from the keyring, or POSTIO_STORE_KEY.";

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
            "search" if command.is_none() => {
                let words: Vec<String> = arguments.by_ref().collect();
                command = Some(format!("search {}", words.join(" ")));
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

    let path = path.unwrap_or_else(postio_session::paths::store_path);
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
    let report = if command == "classify" {
        // The person's own rules, so a hold is counted as the pass would make it.
        let focus = postio_config::Config::load()
            .map(|config| config.focus)
            .unwrap_or_default();
        postio_session::diag::classify(connection, &focus).await
    } else {
        postio_session::diag::report(connection, file, &command).await
    };
    match report {
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
