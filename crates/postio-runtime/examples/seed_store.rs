//! Fill a store with a realistic mailbox, for measuring against.
//!
//! `postio-88b` asks whether the assembled application meets docs/PRODUCT.md §18's
//! budgets on a *realistic account*, and an empty database answers nothing:
//! startup on no mail is fast for the wrong reason. This writes one.
//!
//! ```sh
//! cargo run -p postio-runtime --example seed_store -- /tmp/postio.db 20000
//! POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 \
//!   POSTIO_STARTUP_EXIT=1 cargo run -p postio-app
//! ```
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network, and the
//! mail it writes is `postio-model`'s own corpus of invented people.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("postio-perf.db"));
    let count: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);

    // A blob directory beside it, the way `postio-app` lays them out, so a
    // store seeded here is one the application can open unchanged.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // The store is encrypted under the key in the keyring (ADR 0014), and a
    // store seeded under any other key is one `postio-app` cannot open — which
    // would make this tool useless for the thing it exists for.
    let store_key = match read_store_key() {
        Some(key) => key,
        None => {
            eprintln!(
                "cannot read this installation's store key. Unlock the keyring, \n\
                 and run postio-app once if it has never run: the key is minted \n\
                 on first start and this tool deliberately never mints one."
            );
            std::process::exit(1);
        }
    };
    let database = match postio_storage::Database::open(
        &path,
        &store_key.derive(postio_storage::key::Purpose::Database),
    ) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            std::process::exit(1);
        }
    };

    let started = std::time::Instant::now();
    let report = postio_storage::seed::seed_large(&database, 7, count);
    println!(
        "seeded {} messages across {} folders into {} in {:.1}s",
        report.message_count,
        report.mailboxes.len(),
        path.display(),
        started.elapsed().as_secs_f64()
    );
}

/// This installation's master key, or `None` if it cannot be read.
///
/// **Never mints one.** `postio_session::store_key` is what may create a key,
/// and it is careful about when: only a *missing* entry is a first run, since
/// minting for a locked keyring would encrypt the next thing written under
/// something the existing store knows nothing about. A development tool has no
/// business making that judgement, so this only ever reads — a store that has
/// no key yet is one the application has not opened, and the fix is to open it.
fn read_store_key() -> Option<postio_storage::key::StoreKey> {
    use postio_account::secret::{AccountKey, SecretStore};

    let secrets = postio_account::secret::KeyringSecretStore::default();
    let entry = AccountKey::new(postio_storage::key::STORE_KEY_ENTRY);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let stored = runtime.block_on(secrets.retrieve(&entry)).ok()?;
    postio_storage::key::StoreKey::from_hex(stored.expose()).ok()
}
