use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("postio-perf.db"));
    let count: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Encrypted under the keyring key (ADR 0014); a store under any other key cannot be opened by
    // the app.
    let store_key = match read_store_key().await {
        Some(key) => key,
        None => {
            eprintln!(
                "cannot read this installation's store key. Unlock the keyring, \n\
                 and run the app once if it has never run: the key is minted \n\
                 on first start and this tool deliberately never mints one."
            );
            std::process::exit(1);
        }
    };
    let database = match postio_storage::Store::open(
        &path,
        &store_key.derive(postio_storage::key::Purpose::Database),
    )
    .await
    {
        Ok(database) => database,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            std::process::exit(1);
        }
    };

    let started = std::time::Instant::now();
    let report = postio_storage::seed::seed_large(&database, 7, count).await;
    println!(
        "seeded {} messages across {} folders into {} in {:.1}s",
        report.message_count,
        report.mailboxes.len(),
        path.display(),
        started.elapsed().as_secs_f64()
    );
}

/// This installation's master key, or `None` if it cannot be read.
/// Never mints one: minting for a locked keyring would orphan the existing store.
/// Awaited on `main`'s runtime; building a second one panics.
async fn read_store_key() -> Option<postio_storage::key::StoreKey> {
    use postio_account::secret::{AccountKey, SecretStore};

    let secrets = postio_account::secret::KeyringSecretStore::default();
    let entry = AccountKey::new(postio_storage::key::STORE_KEY_ENTRY);
    let stored = secrets.retrieve(&entry).await.ok()?;
    postio_storage::key::StoreKey::from_hex(stored.expose()).ok()
}
