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
//! # Point it at a copy of a store you care about
//!
//! The same rules as [`inspect_mailboxes`], for the same reasons: the engine
//! has no read-only open, so the safety is this instruction rather than a
//! file mode — nothing below writes, and a copy protects the live store from
//! the engine's own recovery on open. The keyring entry is **retrieved,
//! never minted**, because minting would replace the key a populated store
//! is encrypted under; and it selects ids, states, timestamps and counts
//! only.
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

    let secrets = KeyringSecretStore::default();
    let stored = secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)).await?;
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?.derive(Purpose::Database);

    let store = postio_storage::Store::open(&path, &key).await?;
    let connection = store.connect().await?;

    let now = chrono::Utc::now().timestamp();
    println!("now:   {now}\n");

    // ── what the Outbox and Drafts are showing ───────────────────────────
    //
    // The same predicates the list uses, so a disagreement between this and
    // the screen is itself the finding.
    async fn counted(
        connection: &postio_storage::Checkout,
        sql: &str,
    ) -> Result<i64, postio_storage::Error> {
        postio_storage::sql::scalar(connection, sql, ()).await
    }
    println!(
        "messages.send_state: queued {}  sending {}  failed {}  unconfirmed {}  sent {}",
        counted(
            &connection,
            "SELECT count(*) FROM messages WHERE send_state = 'queued'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM messages WHERE send_state = 'sending'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM messages WHERE send_state = 'failed'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM messages WHERE send_state = 'unconfirmed'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM messages WHERE send_state = 'sent'"
        )
        .await?,
    );
    println!(
        "drafts.state:        queued {}  sending {}  failed {}  unconfirmed {}  editing {}\n",
        counted(
            &connection,
            "SELECT count(*) FROM drafts WHERE state = 'queued'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM drafts WHERE state = 'sending'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM drafts WHERE state = 'failed'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM drafts WHERE state = 'unconfirmed'"
        )
        .await?,
        counted(
            &connection,
            "SELECT count(*) FROM drafts WHERE state = 'editing'"
        )
        .await?,
    );

    // ── every draft that is not merely being written ─────────────────────
    let rows = postio_storage::sql::all(
        &connection,
        "SELECT d.id, d.account_id, d.state, d.message_id, d.updated_at,
                m.send_state, m.send_at,
                q.id, q.state, q.attempts, q.next_attempt_at,
                length(q.last_error), q.op_type
           FROM drafts d
           LEFT JOIN messages m ON m.id = d.message_id
           LEFT JOIN operation_queue q
                  ON q.target_kind = 'draft' AND q.target_id = d.id
                 AND q.op_type = 'send'
          WHERE d.state <> 'editing'
          ORDER BY d.account_id, d.id",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)?,
                row.col::<i64>(1)?,
                row.col::<String>(2)?,
                row.col::<Option<i64>>(3)?,
                row.col::<i64>(4)?,
                row.col::<Option<String>>(5)?,
                row.col::<Option<i64>>(6)?,
                row.col::<Option<i64>>(7)?,
                row.col::<Option<String>>(8)?,
                row.col::<Option<i64>>(9)?,
                row.col::<Option<i64>>(10)?,
                row.col::<Option<i64>>(11)?,
            ))
        },
    )
    .await?;

    let mut any = false;
    for (
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
    ) in rows
    {
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
        let live = matches!(op_state.as_deref(), Some("pending") | Some("in_flight"));
        match op {
            Some(op) => {
                let due = match next {
                    Some(n) if n > now => format!("waiting {}s", n - now),
                    Some(n) => format!("due {}s ago", now - n),
                    None => "due now".to_owned(),
                };
                println!(
                    "  operation       {op} {}{}, attempts {}, {due}{}",
                    op_state.clone().unwrap_or_default(),
                    if live { "" } else { "  <-- not live" },
                    attempts.unwrap_or(0),
                    match error_len {
                        Some(n) => format!(", last_error {n} chars (not printed)"),
                        None => String::new(),
                    }
                );
            }
            None => println!("  operation       NONE  <-- nothing will ever pick this up"),
        }
        // Milliseconds, not seconds: `DraftRepository` writes these through
        // `to_millis`. Comparing them to a seconds clock printed "-1787389526040s
        // ago", which is the sort of number that gets read as corruption.
        let updated_secs = updated / 1000;
        println!(
            "  updated_at      {updated} ({} ago)",
            humanise(now - updated_secs)
        );

        // The verdict, said plainly, because the columns above are the
        // evidence for it rather than the answer.
        let verdict = match (state.as_str(), live, send_at) {
            (_, false, _) if op.is_some() => {
                "STUCK: the operation behind this finished and the draft was left \
                 queued. Nothing will retry it. Send it again."
            }
            (_, false, _) => "STUCK: queued with no operation at all. Send it again.",
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

/// A duration in seconds, said the way a person reads one.
fn humanise(seconds: i64) -> String {
    match seconds {
        s if s < 0 => format!("{}s in the future", -s),
        s if s < 90 => format!("{s}s ago"),
        s if s < 5400 => format!("{} minutes ago", s / 60),
        s if s < 172_800 => format!("{} hours ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}
