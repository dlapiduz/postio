//! Read-only: what mailbox rows the live store holds, and which one each role
//! resolves to (#1178).
//!
//! The sidebar draws one row per role and demotes any twin to `FOLDERS`
//! (`postio_ui::sidebar::sections`), so `Archive` appearing twice means the
//! store holds two mailbox rows claiming that role. This says which rows
//! those are — their paths, their `SPECIAL-USE`-derived roles, and how many
//! messages sit in each — so the answer is a fact about the account rather
//! than an inference from a screenshot.
//!
//! # Why this is safe to point at a personal store
//!
//! * The database is opened `SQLITE_OPEN_READ_ONLY` and then put in
//!   `PRAGMA query_only`, so nothing here can write — not a checkpoint, not
//!   a vacuum, not a schema migration. That matters beyond the obvious: the
//!   WAL on the box this was written for is 676 MB (#1175), and an ordinary
//!   read-write open would checkpoint it and destroy the evidence for that
//!   issue while "fixing" it.
//! * The keyring entry is **retrieved, never minted**. `postio_session`'s
//!   `store_key` mints a fresh key when it finds none, which would replace
//!   the key a populated store is encrypted under; this refuses instead.
//! * It selects ids, paths, roles and counts. No subject, no address, no
//!   body — nothing that is mail. The key is never printed.
//!
//! Run it against the default store:
//!
//! ```sh
//! cargo run -p postio-runtime --example inspect_mailboxes
//! ```

use std::collections::BTreeMap;

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::key::{Purpose, STORE_KEY_ENTRY, StoreKey};
use rusqlite::{Connection, OpenFlags};

/// One mailbox competing for a role: its row id, its path, and how much mail
/// is in it. The counts are the point — telling two look-alike folders apart
/// is what #1178 turned on.
struct Claimant {
    id: i64,
    path: String,
    messages: i64,
}

/// Claimants keyed by the account and role they are competing for.
type ByRole = BTreeMap<(i64, String), Vec<Claimant>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

    // Retrieve only. An absent entry is an error here, never a mint --
    // `postio_session::store_key` would mint one, which on a populated store
    // means a key that can never open it again.
    let secrets = KeyringSecretStore::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let stored = runtime.block_on(secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)))?;
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?.derive(Purpose::Database);

    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("PRAGMA cipher_memory_security = OFF;")?;
    {
        let hex = key.to_hex();
        connection.execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))?;
    }
    // Belt and braces on top of the read-only flag.
    connection.execute_batch("PRAGMA query_only = ON;")?;
    connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
        row.get::<_, i64>(0)
    })?;

    let wal: i64 = connection
        .query_row("PRAGMA wal_checkpoint", [], |row| row.get::<_, i64>(1))
        .unwrap_or(-1);
    println!("wal frames: {wal}   (-1 = could not ask read-only)\n");

    // ── every mailbox row, with what is in it ────────────────────────────
    let mut statement = connection.prepare(
        "SELECT m.id, m.account_id, m.path, m.role, m.selectable, m.parent_id,
                (SELECT count(*) FROM messages x WHERE x.mailbox_id = m.id)
           FROM mailboxes m
          ORDER BY m.account_id, m.path COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;

    println!(
        "{:>5}  {:>4}  {:<12} {:>4} {:>7} {:>9}  path",
        "id", "acct", "role", "sel", "parent", "messages"
    );
    let mut by_role: ByRole = BTreeMap::new();
    for row in rows {
        let (id, account, path, role, selectable, parent, messages) = row?;
        println!(
            "{id:>5}  {account:>4}  {role:<12} {selectable:>4} {:>7} {messages:>9}  {path}",
            parent.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
        );
        if role != "regular" {
            by_role
                .entry((account, role))
                .or_default()
                .push(Claimant { id, path, messages });
        }
    }

    // ── the roles with more than one claimant ────────────────────────────
    println!("\nroles claimed by more than one mailbox:");
    let mut any = false;
    for ((account, role), mut claimants) in by_role {
        if claimants.len() < 2 {
            continue;
        }
        any = true;
        // The sidebar and `MailboxRepository::by_role` both crown the lowest
        // path, so this ordering is the one that decides.
        claimants.sort_by(|a, b| a.path.cmp(&b.path));
        println!("  account {account}  role {role}");
        for (index, claimant) in claimants.iter().enumerate() {
            let crown = if index == 0 { "CROWNED" } else { "demoted" };
            println!(
                "    {crown:>7}  id {:>4}  {:>8} messages  {}",
                claimant.id, claimant.messages, claimant.path
            );
        }
    }
    if !any {
        println!("  (none -- every role has exactly one mailbox)");
    }

    // ── the same server folder stored twice ──────────────────────────────
    println!("\npaths that appear more than once in one account:");
    let mut duplicates = connection.prepare(
        "SELECT account_id, path COLLATE NOCASE, count(*)
           FROM mailboxes
          GROUP BY account_id, path COLLATE NOCASE
         HAVING count(*) > 1",
    )?;
    let mut found = false;
    for row in duplicates.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })? {
        let (account, path, count) = row?;
        found = true;
        println!("  account {account}  {count} rows  {path}");
    }
    if !found {
        println!("  (none -- every path is stored once)");
    }

    // ── has anything ever finished a full pass ───────────────────────────
    println!("\nsync_state (the sidebar's 'never synced' comes from last_full_sync_at):");
    let mut sync = connection.prepare(
        "SELECT s.mailbox_id, m.path, s.last_full_sync_at, s.highest_mod_seq, s.uid_next
           FROM sync_state s LEFT JOIN mailboxes m ON m.id = s.mailbox_id
          ORDER BY m.path COLLATE NOCASE",
    )?;
    let mut rows = 0;
    for row in sync.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<i64>>(4)?,
        ))
    })? {
        let (mailbox, path, full, mod_seq, uid_next) = row?;
        rows += 1;
        println!(
            "  mailbox {mailbox:>4}  full_sync {:<14} mod_seq {:<12} uid_next {:<8} {}",
            full.map(|v| v.to_string())
                .unwrap_or_else(|| "NEVER".into()),
            mod_seq.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            uid_next
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            path.unwrap_or_else(|| "<no mailbox row>".into()),
        );
    }
    if rows == 0 {
        println!("  (no sync_state rows at all)");
    }

    Ok(())
}

fn dirs_store_path() -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").expect("HOME");
            std::path::Path::new(&home).join(".local/share/postio/postio.db")
        })
}
