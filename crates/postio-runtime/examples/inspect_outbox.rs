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

    // The same predicates the list uses, so a disagreement with the screen is the finding.
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
        // Milliseconds (`to_millis`); a seconds clock printed absurd ages.
        let updated_secs = updated / 1000;
        println!(
            "  updated_at      {updated} ({} ago)",
            humanise(now - updated_secs)
        );

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

fn humanise(seconds: i64) -> String {
    match seconds {
        s if s < 0 => format!("{}s in the future", -s),
        s if s < 90 => format!("{s}s ago"),
        s if s < 5400 => format!("{} minutes ago", s / 60),
        s if s < 172_800 => format!("{} hours ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}
