//! Which stored bodies carry the "could not be decoded" caveat, and why.
//!
//! ```text
//! POSTIO_STORE=~/scratch/postio-run/state/data/postio/postio.db \
//! POSTIO_STORE_KEY=$(secret-tool lookup application postio \
//!     account 'local store encryption key' xdg:schema org.postio.Account) \
//!   cargo run -p postio-storage --example encoding_census
//! ```
//!
//! **Point it at a copy.** Same read-only promise as `store_census`: it
//! issues `SELECT` and nothing else.
//!
//! Ids, counts and header *tokens* only — a part's declared media type,
//! charset and transfer encoding are structure, not mail. No subject, no
//! address, no body text is printed; the body is read only to answer two
//! booleans (is it there, does it contain U+FFFD).

use postio_storage::Store;
use postio_storage::key::StoreKey;
use postio_storage::sql::RowExt;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let path = std::env::var("POSTIO_STORE").expect("POSTIO_STORE is the path to postio.db");
    let path = shellexpand(&path);
    let hex = std::env::var("POSTIO_STORE_KEY").expect("POSTIO_STORE_KEY is the hex store key");
    let key = StoreKey::from_hex(hex.trim()).expect("a 32-byte hex store key");
    let store = Store::open(&path, &key.derive(postio_storage::key::Purpose::Database))
        .await
        .expect("open the store");
    let connection = store.connect().await.expect("a connection");

    let total: i64 = postio_storage::sql::scalar(&connection, "SELECT count(*) FROM messages", ())
        .await
        .unwrap_or(-1);
    let bodied: i64 = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM messages WHERE body_text IS NOT NULL OR body_html IS NOT NULL",
        (),
    )
    .await
    .unwrap_or(-1);
    let flagged: i64 = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM messages WHERE body_encoding_problems = 1",
        (),
    )
    .await
    .unwrap_or(-1);
    println!("messages {total}, with a body {bodied}, flagged {flagged}");

    let rows = postio_storage::sql::all(
        &connection,
        "SELECT id, mailbox_id, text_part_id, text_part_headers, html_part_id, html_part_headers,
                body_text, body_html,
                (SELECT count(*) FROM attachments a WHERE a.message_id = messages.id),
                body_headers_truncated
           FROM messages WHERE body_encoding_problems = 1 ORDER BY id LIMIT 40",
        (),
        |row| {
            Ok((
                row.col::<i64>(0)?,
                row.col::<i64>(1)?,
                row.opt_text(2)?,
                row.opt_text(3)?,
                row.opt_text(4)?,
                row.opt_text(5)?,
                // The columns hold whichever shape `body_codec` chose;
                // measured here after unpacking, and only for the two
                // booleans the census prints.
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
    .await
    .expect("read the flagged rows");

    for (id, mailbox, text_id, text_headers, html_id, html_headers, text, html, parts, truncated) in
        rows
    {
        let shape = |body: Option<(usize, bool)>| match body {
            Some((len, fffd)) => format!("len {len} fffd {fffd}"),
            None => "absent".to_owned(),
        };
        println!(
            "message {id} mailbox {mailbox}: text part {} [{}] {}; html part {} [{}] {}; attachments {parts}; headers truncated {truncated}",
            text_id.as_deref().unwrap_or("-"),
            tokens(text_headers.as_deref()),
            shape(text),
            html_id.as_deref().unwrap_or("-"),
            tokens(html_headers.as_deref()),
            shape(html),
        );
    }

    // The attachments of the flagged messages: type and encoding tokens, size.
    let parts = postio_storage::sql::all(
        &connection,
        "SELECT a.message_id, a.part_id, a.mime_type, a.size, a.part_headers, a.blob_id IS NOT NULL
           FROM attachments a JOIN messages m ON m.id = a.message_id
          WHERE m.body_encoding_problems = 1 ORDER BY a.message_id, a.position LIMIT 60",
        (),
        |row| {
            Ok((
                row.col::<i64>(0)?,
                row.opt_text(1)?,
                row.col::<String>(2)?,
                row.col::<i64>(3)?,
                row.opt_text(4)?,
                row.col::<i64>(5)?,
            ))
        },
    )
    .await
    .expect("read the flagged messages' parts");
    for (message, part, mime, size, headers, downloaded) in parts {
        println!(
            "  message {message} part {} {mime} {size} bytes [{}] downloaded {downloaded}",
            part.as_deref().unwrap_or("-"),
            tokens(headers.as_deref())
        );
    }
}

/// The structural tokens of a stored part-header block: media type, charset
/// and transfer encoding. Anything else in the block — a filename, a
/// description — is somebody's mail and is not printed.
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
                .map(|c| c.trim_matches('"').to_owned());
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
    // How the block ends is structure too: an entity is rebuilt as
    // `headers + CRLF + section`, so a block without its own terminator
    // glues the first body line onto the last header.
    let ending = if headers.ends_with("\r\n\r\n") {
        "ends=blank-line"
    } else if headers.ends_with("\r\n") {
        "ends=crlf"
    } else if headers.ends_with('\n') {
        "ends=lf"
    } else {
        "ends=none"
    };
    out.push(format!("{ending} lines={}", headers.lines().count()));
    if out.len() == 1 {
        format!("{} bytes, no header lines, {ending}", headers.len())
    } else {
        out.join(" ")
    }
}

fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => path.to_owned(),
    }
}
