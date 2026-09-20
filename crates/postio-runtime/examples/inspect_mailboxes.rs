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
//! # Point it at a copy of a store you care about
//!
//! * The engine has no read-only open — the `SQLITE_OPEN_READ_ONLY` flag and
//!   `PRAGMA query_only` that used to belt-and-brace this have no
//!   equivalent — so the safety is this instruction rather than a file mode.
//!   Nothing below writes; what is left is the engine's own recovery on
//!   open, which is exactly what a copy protects the live store from.
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

    // Retrieve only. An absent entry is an error here, never a mint --
    // `postio_session::store_key` would mint one, which on a populated store
    // means a key that can never open it again.
    let secrets = KeyringSecretStore::default();
    let stored = secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)).await?;
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?.derive(Purpose::Database);

    // **Point this at a copy.** There is no read-only open in the engine's
    // Rust API -- the `SQLITE_OPEN_READ_ONLY` flag and `PRAGMA query_only`
    // that used to belt-and-brace this have no equivalent -- so the safety
    // is now the instruction in the doc comment rather than the file mode.
    // Nothing below writes; what is left is the engine's own recovery on
    // open, which is exactly what a copy protects the live store from.
    let store = postio_storage::Store::open(&path, &key).await?;
    let connection = store.connect().await?;
    let _ =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM sqlite_schema", ()).await?;

    // ── every mailbox row, with what is in it ────────────────────────────
    let rows = postio_storage::sql::all(
        &connection,
        "SELECT m.id, m.account_id, m.path, m.role, m.selectable, m.parent_id,
                (SELECT count(*) FROM messages x WHERE x.mailbox_id = m.id)
           FROM mailboxes m
          ORDER BY m.account_id, m.path COLLATE NOCASE",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)?,
                row.col::<i64>(1)?,
                row.col::<String>(2)?,
                row.col::<String>(3)?,
                row.col::<i64>(4)?,
                row.col::<Option<i64>>(5)?,
                row.col::<i64>(6)?,
            ))
        },
    )
    .await?;

    println!(
        "{:>5}  {:>4}  {:<12} {:>4} {:>7} {:>9}  path",
        "id", "acct", "role", "sel", "parent", "messages"
    );
    let mut by_role: ByRole = BTreeMap::new();
    // Kept for the timing pass below, which needs every mailbox again.
    let mut listed: Vec<(i64, String, i64)> = Vec::new();
    for (id, account, path, role, selectable, parent, messages) in rows {
        listed.push((id, path.clone(), messages));
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
    let duplicates = postio_storage::sql::all(
        &connection,
        "SELECT account_id, path COLLATE NOCASE, count(*)
           FROM mailboxes
          GROUP BY account_id, path COLLATE NOCASE
         HAVING count(*) > 1",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)?,
                row.col::<String>(1)?,
                row.col::<i64>(2)?,
            ))
        },
    )
    .await?;
    let mut found = false;
    for (account, path, count) in duplicates {
        found = true;
        println!("  account {account}  {count} rows  {path}");
    }
    if !found {
        println!("  (none -- every path is stored once)");
    }

    // ── what opening each folder costs before a single row is drawn ──────
    //
    // The threaded list asks `ThreadRepository::count_of` for the folder, and
    // the folder-scoped form is a correlated subquery: for every message in
    // the mailbox, does a newer message in the same thread live here too. The
    // probe is indexed (`idx_messages_thread_mailbox`), but it is one probe
    // per message, and every page that misses the cache is a decrypt (#1237
    // is the same arithmetic from the snooze sweep).
    //
    // Timed here rather than reasoned about, because the number is a property
    // of *this* store -- how big the folder is and how much of it fits.
    println!("\nwhat the threaded list costs to open (the count before any row):");
    const MEMBER: &str = "deleted_locally = 0 AND (snoozed_until IS NULL \
                          OR snoozed_until <= (strftime('%s','now') * 1000))";
    let mut timed: Vec<(u128, String, i64, i64)> = Vec::new();
    for (id, path, messages) in &listed {
        let started = std::time::Instant::now();
        let threads = postio_storage::sql::scalar(
            &connection,
            &format!(
                "SELECT count(*) FROM messages rep
                  WHERE rep.mailbox_id = ?1 AND rep.{MEMBER}
                    AND NOT EXISTS (
                            SELECT 1 FROM messages newer
                             WHERE newer.mailbox_id = ?1 AND newer.{MEMBER}
                               AND newer.thread_id IS NOT NULL
                               AND newer.thread_id = rep.thread_id
                               AND (newer.received_at, newer.id)
                                   > (rep.received_at, rep.id))"
            ),
            [*id],
        )
        .await?;
        timed.push((
            started.elapsed().as_millis(),
            path.clone(),
            *messages,
            threads,
        ));
    }
    timed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (ms, path, messages, threads) in &timed {
        let flag = if *ms >= 1000 {
            "  <-- a person is waiting"
        } else {
            ""
        };
        println!("  {ms:>7} ms  {messages:>7} messages -> {threads:>7} rows  {path}{flag}");
    }

    // ── the per-account role map (ADR 0035) ──────────────────────────────
    // What the sidebar resolves a reserved row through. It **overrides** the
    // role a folder's own `SPECIAL-USE` attribute claims, so a row that opens
    // the wrong folder shows up here rather than in the table above -- and an
    // empty map is the ordinary state, meaning "believe the server".
    println!("\nmailbox_roles (overrides what the server's attributes said):");
    let mapped = postio_storage::sql::all(
        &connection,
        "SELECT account_id, role, path FROM mailbox_roles ORDER BY account_id, role",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)?,
                row.col::<String>(1)?,
                row.col::<String>(2)?,
            ))
        },
    )
    .await?;
    let mut any_mapped = false;
    for (account, role, path) in mapped {
        any_mapped = true;
        // Does the path it names actually exist, and hold anything?
        let found = postio_storage::sql::first(
            &connection,
            "SELECT m.id, (SELECT count(*) FROM messages x WHERE x.mailbox_id = m.id)
               FROM mailboxes m WHERE m.account_id = ?1 AND m.path = ?2",
            (account, path.clone()),
            |row| {
                use postio_storage::sql::RowExt as _;
                Ok((row.col::<i64>(0)?, row.col::<i64>(1)?))
            },
        )
        .await?;
        match found {
            Some((id, messages)) => println!(
                "  account {account}  {role:<8} -> {path:?}  (mailbox {id}, {messages} messages)"
            ),
            None => println!(
                "  account {account}  {role:<8} -> {path:?}  <-- NO SUCH MAILBOX; the row opens nothing"
            ),
        }
    }
    if !any_mapped {
        println!("  (none -- every reserved row follows the server's own attributes)");
    }

    println!("\nsync_state (the sidebar's 'never synced' comes from last_full_sync_at):");
    let states = postio_storage::sql::all(
        &connection,
        "SELECT s.mailbox_id, m.path, s.last_full_sync_at, s.highest_mod_seq, s.uid_next
           FROM sync_state s LEFT JOIN mailboxes m ON m.id = s.mailbox_id
          ORDER BY m.path COLLATE NOCASE",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)?,
                row.col::<Option<String>>(1)?,
                row.col::<Option<i64>>(2)?,
                row.col::<Option<i64>>(3)?,
                row.col::<Option<i64>>(4)?,
            ))
        },
    )
    .await?;
    let mut rows = 0;
    for (mailbox, path, full, mod_seq, uid_next) in states {
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
