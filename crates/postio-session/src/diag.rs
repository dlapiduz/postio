//! What a store holds, what it costs, and what the background lanes still
//! owe -- counts, sizes, ids and header tokens, never mail.
//!
//! The reports `postio-diag` prints. A library rather than the binary's own
//! code because the daemon owns the store while it runs (ADR 0041): with it
//! running, `postio-diag` asks it for the report over the socket, and the
//! daemon runs these same queries on its own connection. Without it, the
//! binary opens the store and runs them itself.
//!
//! Every report issues `SELECT` and `PRAGMA` and nothing else.

use std::fmt::Write as _;

/// One line of a report: `say!(self, "...", args)`, or `say!(self)` for a
/// blank one.
macro_rules! say {
    ($report:expr) => {
        $report.line(format_args!(""))
    };
    ($report:expr, $($arg:tt)*) => {
        $report.line(format_args!($($arg)*))
    };
}

use postio_storage::sql::{self, RowExt};

/// The reports there are, and what each tells.
pub const COMMANDS: &[(&str, &str)] = &[
    (
        "census",
        "what the store holds and what it costs: tables, bytes, folders",
    ),
    (
        "encoding",
        "the bodies carrying the decode caveat, and what their parts declared",
    ),
    ("shape", "rows per page, with and without the body columns"),
    (
        "pending",
        "what the background lanes still owe: backfill, index, queue",
    ),
];

/// Run report `command` over `connection`, a store whose file is `file`
/// bytes, and answer its text.
pub async fn report(
    connection: postio_storage::Checkout,
    file: u64,
    command: &str,
) -> Result<String, String> {
    let report = Report {
        connection,
        file,
        out: std::sync::Mutex::new(String::new()),
    };
    let outcome = match command {
        "census" => report.census().await,
        "encoding" => report.encoding().await,
        "shape" => report.shape().await,
        "pending" => report.pending().await,
        other => return Err(format!("no such report {other:?}")),
    };
    outcome.map_err(|error| error.to_string())?;
    Ok(report.out.into_inner().unwrap_or_default())
}

struct Report {
    connection: postio_storage::Checkout,
    file: u64,
    out: std::sync::Mutex<String>,
}

pub(crate) type Outcome = Result<(), postio_storage::Error>;

impl Report {
    /// Add a line to the report's text.
    fn line(&self, text: std::fmt::Arguments<'_>) {
        let mut out = self.out.lock().expect("the report's text");
        let _ = out.write_fmt(text);
        out.push('\n');
    }

    async fn count(&self, sql: &str) -> i64 {
        sql::scalar(&self.connection, sql, ()).await.unwrap_or(-1)
    }

    /// What the store holds and what it costs.
    async fn census(&self) -> Outcome {
        let messages = self.count("SELECT count(*) FROM messages").await;
        let bodied = self
            .count("SELECT count(*) FROM messages WHERE body_text IS NOT NULL OR body_html IS NOT NULL")
            .await;
        let indexed = self
            .count("SELECT count(*) FROM message_search_bodies")
            .await;
        let headers = self.count("SELECT count(*) FROM message_headers").await;
        let threads = self.count("SELECT count(*) FROM threads").await;
        let mailboxes = self.count("SELECT count(*) FROM mailboxes").await;
        let queued = self.count("SELECT count(*) FROM operation_queue").await;
        let page_size = self.count("PRAGMA page_size").await;
        let pages = self.count("PRAGMA page_count").await;
        let free = self.count("PRAGMA freelist_count").await;

        say!(self, "file            {:>12}", bytes(self.file));
        say!(
            self,
            "pages           {pages:>12} x {page_size} B  ({free} free, {})",
            bytes((free.max(0) as u64) * page_size.max(0) as u64)
        );
        say!(self, "mailboxes       {mailboxes:>12}");
        say!(self, "messages        {messages:>12}");
        say!(self, "  with a body   {bodied:>12}");
        say!(self, "  fts-indexed   {indexed:>12}");
        say!(self, "threads         {threads:>12}");
        say!(self, "header rows     {headers:>12}");
        say!(self, "queued ops      {queued:>12}");

        // Where the bytes are: the *content* each table holds, since this
        // engine has no `dbstat` (ADR 0038). The difference between these
        // sums and the file is index and page overhead.
        let header_bytes = self
            .count("SELECT coalesce(sum(length(name) + length(value)), 0) FROM message_headers")
            .await;
        let body_bytes = self
            .count(
                "SELECT coalesce(sum(length(coalesce(body_text, '')) \
                 + length(coalesce(body_html, ''))), 0) FROM messages",
            )
            .await;
        let search_bytes = self
            .count("SELECT coalesce(sum(length(body_search)), 0) FROM message_search_bodies")
            .await;
        let envelope_bytes = self
            .count(
                "SELECT coalesce(sum(length(coalesce(subject, '')) \
                 + length(coalesce(preview, '')) + length(coalesce(remote_id, ''))), 0) \
                 FROM messages",
            )
            .await;
        say!(self, "\ncontent bytes (no index or page overhead):");
        say!(
            self,
            "  header rows   {:>12}",
            bytes(header_bytes.max(0) as u64)
        );
        say!(
            self,
            "  bodies        {:>12}  (as stored: zstd where smaller)",
            bytes(body_bytes.max(0) as u64)
        );
        say!(
            self,
            "  fts source    {:>12}",
            bytes(search_bytes.max(0) as u64)
        );
        say!(
            self,
            "  envelopes     {:>12}",
            bytes(envelope_bytes.max(0) as u64)
        );
        let content = (header_bytes + body_bytes + search_bytes + envelope_bytes).max(0) as u64;
        say!(self, "  ---           {:>12}", bytes(content));
        if self.file > content {
            say!(
                self,
                "  overhead      {:>12}  ({:.0}% of the file)",
                bytes(self.file - content),
                (self.file - content) as f64 / self.file as f64 * 100.0
            );
        }

        // The interaction budget, on a real store rather than a fixture.
        let deep = std::time::Instant::now();
        let _ = self
            .count(
                "SELECT count(*) FROM (SELECT id FROM messages ORDER BY received_at DESC, id DESC \
                 LIMIT 50 OFFSET 20000)",
            )
            .await;
        say!(
            self,
            "\na page 20,000 rows deep   {:>8.1} ms",
            deep.elapsed().as_secs_f64() * 1000.0
        );

        say!(self, "\nper folder:");
        say!(
            self,
            "{:>5}  {:>9}  {:>9}  {:>8}  {:>14}  path",
            "id",
            "local",
            "uid_next",
            "full?",
            "last attempt"
        );
        let rows = sql::all(
            &self.connection,
            "SELECT m.id, m.path, count(x.id), coalesce(s.uid_next, 0), coalesce(s.last_seen_at, 0), \
                    CASE WHEN s.last_full_sync_at IS NULL THEN 0 ELSE 1 END \
               FROM mailboxes m \
               LEFT JOIN sync_state s ON s.mailbox_id = m.id \
               LEFT JOIN messages x ON x.mailbox_id = m.id \
              GROUP BY m.id ORDER BY count(x.id) DESC",
            (),
            |row| {
                Ok((
                    row.col::<i64>(0)?,
                    row.col::<String>(1)?,
                    row.col::<i64>(2)?,
                    row.col::<i64>(3)?,
                    row.col::<i64>(4)?,
                    row.col::<i64>(5)?,
                ))
            },
        )
        .await?;
        for (id, path, local, uid_next, seen, full) in rows {
            say!(
                self,
                "{id:>5}  {local:>9}  {uid_next:>9}  {:>8}  {:>14}  {path}",
                if full == 1 { "yes" } else { "NO" },
                if seen == 0 {
                    "never".to_owned()
                } else {
                    format!("{seen}")
                }
            );
        }
        Ok(())
    }

    /// The bodies carrying the decode caveat, and what their parts declared.
    ///
    /// A part's media type, charset and transfer encoding are structure, not
    /// mail; the body is read only to answer two booleans about it.
    async fn encoding(&self) -> Outcome {
        let total = self.count("SELECT count(*) FROM messages").await;
        let bodied = self
            .count("SELECT count(*) FROM messages WHERE body_text IS NOT NULL OR body_html IS NOT NULL")
            .await;
        let flagged = self
            .count("SELECT count(*) FROM messages WHERE body_encoding_problems = 1")
            .await;
        say!(
            self,
            "messages {total}, with a body {bodied}, flagged {flagged}"
        );

        let rows = sql::all(
            &self.connection,
            "SELECT id, mailbox_id, text_part_id, text_part_headers, html_part_id, html_part_headers,
                    body_text, body_html, body_parsed_with,
                    (SELECT count(*) FROM attachments a WHERE a.message_id = messages.id)
               FROM messages WHERE body_encoding_problems = 1 ORDER BY id LIMIT 60",
            (),
            |row| {
                Ok((
                    row.col::<i64>(0)?,
                    row.col::<i64>(1)?,
                    row.opt_text(2)?,
                    row.opt_text(3)?,
                    row.opt_text(4)?,
                    row.opt_text(5)?,
                    row.col::<Option<Vec<u8>>>(6)?
                        .map(postio_storage::body_codec::unpack)
                        .map(|text| (text.len(), text.contains('\u{fffd}'))),
                    row.col::<Option<Vec<u8>>>(7)?
                        .map(postio_storage::body_codec::unpack)
                        .map(|html| (html.len(), html.contains('\u{fffd}'))),
                    row.col::<i64>(8)?,
                    row.col::<i64>(9)?,
                ))
            },
        )
        .await?;
        let shape = |body: Option<(usize, bool)>| match body {
            Some((len, fffd)) => format!("len {len} fffd {fffd}"),
            None => "absent".to_owned(),
        };
        for (
            id,
            mailbox,
            text_id,
            text_headers,
            html_id,
            html_headers,
            text,
            html,
            parsed,
            parts,
        ) in rows
        {
            say!(
                self,
                "message {id} mailbox {mailbox}: text part {} [{}] {}; html part {} [{}] {}; \
                 attachments {parts}; parsed with v{parsed}",
                text_id.as_deref().unwrap_or("-"),
                tokens(text_headers.as_deref()),
                shape(text),
                html_id.as_deref().unwrap_or("-"),
                tokens(html_headers.as_deref()),
                shape(html),
            );
        }
        Ok(())
    }

    /// Rows per page, with and without the body columns: the number that
    /// decides how many pages a list or a hydrate has to read and decrypt.
    async fn shape(&self) -> Outcome {
        let page = self.count("PRAGMA page_size").await.max(1);
        let (rows, body, rest): (i64, i64, i64) = sql::one(
            &self.connection,
            "SELECT count(*),
                    coalesce(sum(length(coalesce(body_text, '')) + length(coalesce(body_html, ''))), 0),
                    coalesce(sum(length(coalesce(subject, '')) + length(coalesce(preview, ''))
                               + length(coalesce(rfc_message_id, '')) + 120), 0)
               FROM messages",
            (),
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
        )
        .await?;
        if rows == 0 {
            say!(self, "messages: no rows");
            return Ok(());
        }
        let fold = self
            .count("SELECT coalesce(sum(length(body_search)), 0) FROM message_search_bodies")
            .await
            .max(0);
        say!(self, "page size {page} bytes");
        say!(self, "messages: {rows} rows");
        say!(
            self,
            "  bodies (as stored):  {:>10} bytes ({:.1} MiB), mean {:.0}/row",
            body,
            body as f64 / 1048576.0,
            body as f64 / rows as f64
        );
        say!(
            self,
            "  everything else:    ~{:>10} bytes ({:.1} MiB), mean {:.0}/row",
            rest,
            rest as f64 / 1048576.0,
            rest as f64 / rows as f64
        );
        say!(
            self,
            "  bodies are {:.0}% of the messages table",
            100.0 * body as f64 / (body + rest).max(1) as f64
        );
        say!(
            self,
            "  fts source (message_search_bodies): {:>10} bytes ({:.1} MiB)",
            fold,
            fold as f64 / 1048576.0
        );
        let with = page as f64 / ((body + rest) as f64 / rows as f64);
        let without = page as f64 / (rest as f64 / rows as f64);
        say!(
            self,
            "  rows per {page}-byte page: {with:.1} as stored, {without:.1} without the bodies ({:.1}x)",
            without / with
        );
        say!(
            self,
            "\nthe file itself: {:.1} MiB",
            self.file as f64 / 1048576.0
        );
        Ok(())
    }

    /// What the background lanes still owe.
    async fn pending(&self) -> Outcome {
        say!(self, "bodies, by state:");
        let states = sql::all(
            &self.connection,
            "SELECT body_state, count(*) FROM messages WHERE deleted_locally = 0
              GROUP BY body_state ORDER BY count(*) DESC",
            (),
            |row| Ok((row.col::<String>(0)?, row.col::<i64>(1)?)),
        )
        .await?;
        for (state, count) in states {
            say!(self, "  {state:<14} {count:>10}");
        }
        let backfill = self
            .count(
                "SELECT count(*) FROM messages
                  WHERE body_state IN ('not_fetched', 'headers_only')
                    AND uid IS NOT NULL AND remote_id IS NOT NULL AND deleted_locally = 0",
            )
            .await;
        // `n/a` on a store from before `body_parsed_with` existed, rather
        // than a number that would be a guess.
        let refetch = sql::scalar(
            &self.connection,
            "SELECT count(*) FROM messages
              WHERE body_parsed_with < ?1 AND body_encoding_problems = 1 AND deleted_locally = 0",
            [i64::from(postio_model::mime::PARSER_VERSION)],
        )
        .await
        .map(|count| count.to_string())
        .unwrap_or_else(|_| "n/a".to_owned());
        let unindexed = self
            .count(
                "SELECT count(*) FROM messages m
                  WHERE m.body_state IN ('full', 'partial')
                    AND NOT EXISTS (SELECT 1 FROM message_search_bodies b WHERE b.message_id = m.id)",
            )
            .await;
        say!(self, "\nowed:");
        say!(self, "  backfill candidates              {backfill:>10}");
        say!(
            self,
            "  re-fetches an older parser owes  {refetch:>10}  (parser v{})",
            postio_model::mime::PARSER_VERSION
        );
        say!(self, "  bodies awaiting the indexer      {unindexed:>10}");

        say!(self, "\noperation queue, by state:");
        let queue = sql::all(
            &self.connection,
            "SELECT state, count(*) FROM operation_queue GROUP BY state ORDER BY count(*) DESC",
            (),
            |row| Ok((row.col::<String>(0)?, row.col::<i64>(1)?)),
        )
        .await?;
        if queue.is_empty() {
            say!(self, "  empty");
        }
        for (state, count) in queue {
            say!(self, "  {state:<14} {count:>10}");
        }
        Ok(())
    }
}

/// The structural tokens of a stored part-header block: media type, charset
/// and transfer encoding, and how the block ends. Anything else in the block
/// -- a filename, a description -- is somebody's mail and is not printed.
fn tokens(headers: Option<&str>) -> String {
    let Some(headers) = headers else {
        return "no headers".to_owned();
    };
    let mut out = Vec::new();
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-type:") {
            let value = value.trim();
            let media = value.split(';').next().unwrap_or("").trim().to_owned();
            let charset = value
                .split(';')
                .map(str::trim)
                .find_map(|param| param.strip_prefix("charset="))
                .map(|charset| charset.trim_matches('"').to_owned());
            out.push(format!(
                "type={media} charset={}",
                charset.as_deref().unwrap_or("-")
            ));
        } else if let Some(value) = lower.strip_prefix("content-transfer-encoding:") {
            out.push(format!("cte={}", value.trim()));
        } else if let Some((name, _)) = line.split_once(':') {
            out.push(format!("{}=…", name.trim().to_ascii_lowercase()));
        }
    }
    if out.is_empty() {
        format!("{} bytes, no header lines", headers.len())
    } else {
        out.join(" ")
    }
}

fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
