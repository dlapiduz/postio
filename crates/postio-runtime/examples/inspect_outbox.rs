//! Read-only: why a message is sitting in the Outbox and not leaving.
//!
//! The Outbox lists drafts whose `send_state` is `queued` or `sending`
//! (spec 003). A message that stays there has one of a small number of
//! causes, and they are told apart by facts this prints:
//!
//! * **scheduled** — `messages.send_at` is in the future. Working as asked.
//! * **waiting on backoff** — the `operation_queue` row exists and its
//!   `next_attempt_at` is in the future after a failed attempt.
//! * **nothing is draining** — the operation is `pending` and due, so the
//!   drainer is not running or the account is offline.
//! * **orphaned** — `send_state` says queued and there is **no operation
//!   row at all**. Nothing will ever pick it up, and re-sending is the only
//!   way out. This is the one that is genuinely stuck.
//! * **interrupted** — `sending` with no in-flight operation means the
//!   process doing the submission is gone. `postio_sync::send` promotes that
//!   to `unconfirmed` when it next runs the operation, so this resolves on
//!   its own *if* the operation is still there.
//!
//! # Why this is safe to point at a personal store
//!
//! The same three guarantees as [`inspect_mailboxes`], for the same reasons:
//! the database is opened `SQLITE_OPEN_READ_ONLY` and then put in
//! `PRAGMA query_only`, so nothing here can write — not a checkpoint, not a
//! vacuum, not a migration; the keyring entry is **retrieved, never minted**,
//! because minting would replace the key a populated store is encrypted
//! under; and it selects ids, states, timestamps and counts only.
//!
//! **No subject, no address, no body, and no `last_error` text** — an SMTP
//! rejection routinely quotes the recipient back, so the error is reported as
//! its length and first word rather than printed. Principle VI: nothing that
//! is mail leaves the machine, and this output is the sort of thing that gets
//! pasted into an issue.
//!
//! ```sh
//! cargo run -p postio-runtime --example inspect_outbox
//! ```

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::key::{Purpose, STORE_KEY_ENTRY, StoreKey};
use rusqlite::{Connection, OpenFlags};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

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
    connection.execute_batch("PRAGMA query_only = ON;")?;

    let now = chrono::Utc::now().timestamp();
    println!("now:   {now}\n");

    // ── what the Outbox and Drafts are showing ───────────────────────────
    //
    // The same predicates the list uses, so a disagreement between this and
    // the screen is itself the finding.
    let counted = |sql: &str| -> rusqlite::Result<i64> {
        connection.query_row(sql, [], |row| row.get::<_, i64>(0))
    };
    println!(
        "messages.send_state: queued {}  sending {}  failed {}  unconfirmed {}  sent {}",
        counted("SELECT count(*) FROM messages WHERE send_state = 'queued'")?,
        counted("SELECT count(*) FROM messages WHERE send_state = 'sending'")?,
        counted("SELECT count(*) FROM messages WHERE send_state = 'failed'")?,
        counted("SELECT count(*) FROM messages WHERE send_state = 'unconfirmed'")?,
        counted("SELECT count(*) FROM messages WHERE send_state = 'sent'")?,
    );
    println!(
        "drafts.state:        queued {}  sending {}  failed {}  unconfirmed {}  editing {}\n",
        counted("SELECT count(*) FROM drafts WHERE state = 'queued'")?,
        counted("SELECT count(*) FROM drafts WHERE state = 'sending'")?,
        counted("SELECT count(*) FROM drafts WHERE state = 'failed'")?,
        counted("SELECT count(*) FROM drafts WHERE state = 'unconfirmed'")?,
        counted("SELECT count(*) FROM drafts WHERE state = 'editing'")?,
    );

    // ── every draft that is not merely being written ─────────────────────
    let mut statement = connection.prepare(
        "SELECT d.id, d.account_id, d.state, d.message_id, d.updated_at,
                m.send_state, m.send_at,
                q.id, q.state, q.attempts, q.next_attempt_at,
                length(q.last_error), q.op_type
           FROM drafts d
           LEFT JOIN messages m ON m.id = d.message_id
           LEFT JOIN operation_queue q
                  ON q.target_kind = 'draft' AND q.target_id = d.id
                 AND q.op_type = 'send' AND q.state IN ('pending', 'in_flight')
          WHERE d.state <> 'editing'
          ORDER BY d.account_id, d.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<i64>>(9)?,
            row.get::<_, Option<i64>>(10)?,
            row.get::<_, Option<i64>>(11)?,
        ))
    })?;

    let mut any = false;
    for row in rows {
        let (
            id,
            account,
            state,
            message,
            updated,
            send_state,
            send_at,
            op,
            op_state,
            attempts,
            next,
            error_len,
        ) = row?;
        any = true;
        println!("draft {id} (account {account})");
        println!("  drafts.state    {state}");
        match (message, send_state) {
            (Some(m), Some(s)) => println!("  mirror row      messages {m}, send_state {s}"),
            (Some(m), None) => println!(
                "  mirror row      messages {m}, send_state NULL  <-- the list cannot see this"
            ),
            (None, _) => {
                println!("  mirror row      NONE  <-- no messages row, so no list shows it")
            }
        }
        match send_at {
            Some(at) if at > now => println!(
                "  send_at         {at}  scheduled, {}s in the future",
                at - now
            ),
            Some(at) => println!("  send_at         {at}  due {}s ago", now - at),
            None => println!("  send_at         none (send as soon as possible)"),
        }
        match op {
            Some(op) => {
                let due = match next {
                    Some(n) if n > now => format!("waiting {}s", n - now),
                    Some(n) => format!("due {}s ago", now - n),
                    None => "due now".to_owned(),
                };
                println!(
                    "  operation       {op} {}, attempts {}, {due}{}",
                    op_state.unwrap_or_default(),
                    attempts.unwrap_or(0),
                    match error_len {
                        Some(n) => format!(", last_error {n} chars (not printed)"),
                        None => String::new(),
                    }
                );
            }
            None => println!("  operation       NONE  <-- nothing will ever pick this up"),
        }
        println!("  updated_at      {updated} ({}s ago)", now - updated);

        // The verdict, said plainly, because the columns above are the
        // evidence for it rather than the answer.
        let verdict = match (state.as_str(), op.is_some(), send_at) {
            (_, false, _) => "STUCK: queued with no operation. Open it in Drafts and send again.",
            ("queued", true, Some(at)) if at > now => "waiting for its scheduled time",
            ("sending", true, _) => {
                "a submission was interrupted; it resolves to \
                                     'unconfirmed' when the drainer next runs it"
            }
            (_, true, _) => match next {
                Some(n) if n > now => "backing off after a failed attempt",
                _ => "due, so the drainer is not running or the account is offline",
            },
        };
        println!("  verdict         {verdict}\n");
    }
    if !any {
        println!("no draft is queued, sending, failed or unconfirmed: the Outbox is empty.");
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
