use std::path::{Path, PathBuf};
use std::time::Instant;

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::Store;
use postio_storage::key::{STORE_KEY_ENTRY, StoreKey};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => return Err("pass the path to a *copy* of a store; see the module docs".into()),
    };
    let live = std::env::var("HOME")
        .map(|home| Path::new(&home).join(".local/share/postio/postio.db"))
        .unwrap_or_default();
    if path == live {
        return Err(
            "refusing to open the live store: this opens read-write, which \
             checkpoints the WAL and destroys what is being measured. Copy it \
             first -- `cp --reflink=always` is instant on btrfs."
                .into(),
        );
    }

    let secrets = KeyringSecretStore::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let keyring_started = Instant::now();
    let stored = runtime.block_on(secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)))?;
    let keyring = keyring_started.elapsed().as_secs_f64() * 1000.0;
    println!("keyring round trip:        {keyring:>8.0} ms");
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?;

    println!("store: {}", path.display());
    report_sizes(&path);

    let first = time_open(&path, &key).await?;
    println!("\nopen #1 (as found):        {first:>8.0} ms");
    report_sizes(&path);

    {
        let database =
            Store::open(&path, &key.derive(postio_storage::key::Purpose::Database)).await?;
        // `truncate_log`, not the pragma: the engine refuses a statement whose rows nobody reads.
        database.truncate_log().await?;
    }
    println!("\nafter wal_checkpoint(TRUNCATE):");
    report_sizes(&path);

    let second = time_open(&path, &key).await?;
    let third = time_open(&path, &key).await?;
    println!("\nopen #2 (WAL truncated):   {second:>8.0} ms");
    println!("open #3 (WAL truncated):   {third:>8.0} ms");

    let saved = first - second.max(third);
    println!(
        "\nthe WAL accounts for {saved:.0} ms of the first open ({:.0}%)",
        if first > 0.0 {
            saved / first * 100.0
        } else {
            0.0
        }
    );
    Ok(())
}

async fn time_open(path: &Path, key: &StoreKey) -> Result<f64, Box<dyn std::error::Error>> {
    let subkey = key.derive(postio_storage::key::Purpose::Database);
    let started = Instant::now();
    let database = Store::open(path, &subkey).await?;
    // One real read: an open that touched no page has not paid for the WAL index.
    let connection = database.connect().await?;
    let _ = postio_storage::sql::scalar(&connection, "SELECT count(*) FROM mailboxes", ()).await?;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    drop(connection);
    drop(database);
    Ok(elapsed)
}

fn report_sizes(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let candidate = PathBuf::from(name);
        match std::fs::metadata(&candidate) {
            Ok(meta) => println!(
                "  {:<10} {:>10.1} MB",
                candidate
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                meta.len() as f64 / 1_048_576.0
            ),
            Err(_) => continue,
        }
    }
}
