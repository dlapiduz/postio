//! Say what this build will do with the local store, and start it over when
//! it cannot be carried forward.
//!
//! # Use
//!
//! ```sh
//! cargo run -p postio-session --bin postio-store -- status
//! cargo run -p postio-session --bin postio-store -- reset
//! scripts/run-isolated.sh HEAD --reset-store   # the scratch store's
//! ```
//!
//! `status` reads the store's schema stamp and says whether the next open
//! uses it as it is, migrates it in place, or refuses it. It writes nothing.
//!
//! `reset` is `postio_session::start_over`, the same thing Focus's "Start a
//! fresh store" does: the database, its sidecars and its blobs are moved into
//! `set-aside/<when>/` beside them -- not deleted -- and a fresh store is
//! started with the accounts carried across. `config.toml` and the keyring
//! are not touched. Snoozes, reminders, Focus's filing history, and drafts
//! and changes not yet sent stay in the store that was set aside.
//!
//! The store is the one every Postio uses: `$POSTIO_STORE`, or
//! `$XDG_DATA_HOME/postio/postio.db`. Outside `scripts/run-isolated.sh`
//! that is your real one.

use std::process::ExitCode;

use postio_account::secret::platform_keyring;
use postio_storage::key::Purpose;
use postio_storage::{Store, schema};

const USAGE: &str = "usage: postio-store status | reset\n\
    \n  status  say whether this build opens the store as it is, migrates it, or cannot\
    \n  reset   set the store aside and start a fresh one, keeping your accounts and config.toml";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("postio-store: cannot start a runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match command.as_deref() {
        Some("status") => runtime.block_on(status()),
        Some("reset") => runtime.block_on(reset()),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The store key, or the sentence for why not.
fn store_key() -> Result<postio_storage::key::StoreKey, ExitCode> {
    postio_session::store_key_blocking(platform_keyring().as_ref()).map_err(|error| {
        eprintln!("postio-store: cannot read the store key (is the keyring unlocked?): {error}");
        ExitCode::FAILURE
    })
}

async fn status() -> ExitCode {
    let path = postio_session::paths::store_path();
    println!("store:  {}", path.display());
    if !path.exists() {
        println!("there is no store yet: the next Postio to open makes one");
        return ExitCode::SUCCESS;
    }
    let key = match store_key() {
        Ok(key) => key,
        Err(code) => return code,
    };
    let stamp = match Store::stamp_at(&path, &key.derive(Purpose::Database)).await {
        Ok(stamp) => stamp,
        Err(error) => {
            eprintln!("postio-store: {error}");
            return ExitCode::FAILURE;
        }
    };
    let hex = |stamp: i64| format!("{:08x}", stamp as i32 as u32);
    println!(
        "schema: {} (this build's is {})",
        hex(stamp),
        hex(schema::FINGERPRINT)
    );
    match schema::migrations_from(stamp) {
        Some(steps) if steps.is_empty() => println!("this build opens it as it is"),
        Some(steps) => println!(
            "this build migrates it in place on its next open ({} step{}), keeping everything",
            steps.len(),
            if steps.len() == 1 { "" } else { "s" }
        ),
        None => println!(
            "this build cannot carry it forward: `postio-store reset` sets it aside and \
             starts a fresh store"
        ),
    }
    ExitCode::SUCCESS
}

async fn reset() -> ExitCode {
    let path = postio_session::paths::store_path();
    println!("store:  {}", path.display());
    if !path.exists() {
        println!("there is no store to reset: the next Postio to open makes one");
        return ExitCode::SUCCESS;
    }
    let key = match store_key() {
        Ok(key) => key,
        Err(code) => return code,
    };
    match postio_session::start_over::start_over_at(&path, &key).await {
        Ok(started) => {
            println!("set aside: {}", started.set_aside.display());
            println!(
                "a fresh store was started with {} account{}; config.toml and the keyring \
                 were not touched",
                started.accounts,
                if started.accounts == 1 { "" } else { "s" }
            );
            println!(
                "snoozes, reminders, Focus's filing history, and drafts and changes not yet \
                 sent stay in the store that was set aside"
            );
            println!("open Postio and it syncs your mail again from the server");
            ExitCode::SUCCESS
        }
        Err(sentence) => {
            eprintln!("postio-store: {sentence}");
            ExitCode::FAILURE
        }
    }
}
