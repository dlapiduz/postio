//! Read-only: how much of the `messages` table is body, and what that costs
//! the queries that never read one. Sizes and counts only.
use postio_storage::key::{Purpose, StoreKey};
use rusqlite::{Connection, OpenFlags};

fn main() {
    let master = StoreKey::from_hex(std::env::var("POSTIO_DIAG_KEY").unwrap().trim()).unwrap();
    let key = master.derive(Purpose::Database);
    let path = std::env::args().nth(1).unwrap();
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    postio_storage::db::configure(&c, &key).expect("unlock");

    // `PRAGMA page_size` answers as text under SQLCipher, which returns its
    // own `cipher_page_size` row first.
    let page: i64 = c
        .query_row("PRAGMA page_size", [], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(4096);
    println!("page size {page} bytes");

    let (rows, body, total): (i64, i64, i64) = c
        .query_row(
            "SELECT count(*),
                coalesce(sum(length(coalesce(body_text,'')) + length(coalesce(body_html,''))
                           + length(coalesce(body_headers,''))), 0),
                coalesce(sum(length(coalesce(subject,'')) + length(coalesce(preview,''))
                           + length(coalesce(rfc_message_id,'')) + 120), 0)
           FROM messages",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    println!("messages: {rows} rows");
    println!(
        "  body columns:      {:>10} bytes ({:.1} MiB), mean {:.0}/row",
        body,
        body as f64 / 1048576.0,
        body as f64 / rows as f64
    );
    println!(
        "  everything else:  ~{:>10} bytes ({:.1} MiB), mean {:.0}/row",
        total,
        total as f64 / 1048576.0,
        total as f64 / rows as f64
    );
    println!(
        "  body is {:.0}% of the table",
        100.0 * body as f64 / (body + total) as f64
    );

    // Rows per page, with and without the bodies: the number that decides how
    // many pages a list or a hydrate has to read and decrypt.
    let with = page as f64 / ((body + total) as f64 / rows as f64);
    let without = page as f64 / (total as f64 / rows as f64);
    println!(
        "  rows per {page}-byte page: {:.1} as stored, {:.1} if bodies moved out ({:.1}x)",
        with,
        without,
        without / with
    );

    println!("\nwhere the database goes, by pages:");
    let mut st = match c.prepare(
        "SELECT name, count(*) pages FROM dbstat GROUP BY name ORDER BY pages DESC LIMIT 12",
    ) {
        Ok(st) => st,
        Err(_) => {
            println!("  dbstat unavailable in this build");
            return;
        }
    };
    let rows: Vec<(String, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .flatten()
        .collect();
    let total: i64 = rows.iter().map(|(_, p)| p).sum();
    for (name, pages) in &rows {
        println!(
            "  {name:<34} {:>7.1} MiB  {:>5.1}%",
            *pages as f64 * page as f64 / 1048576.0,
            100.0 * *pages as f64 / total as f64
        );
    }
}
