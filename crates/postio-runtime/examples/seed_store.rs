//! Fill a store with a realistic mailbox, for measuring against.
//!
//! `postio-88b` asks whether the assembled application meets spec.md §18's
//! budgets on a *realistic account*, and an empty database answers nothing:
//! startup on no mail is fast for the wrong reason. This writes one.
//!
//! ```sh
//! cargo run -p postio-runtime --example seed_store -- /tmp/postio.db 20000
//! POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 \
//!   POSTIO_STARTUP_EXIT=1 cargo run -p postio-app
//! ```
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network, and the
//! mail it writes is `postio-model`'s own corpus of invented people.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("postio-perf.db"));
    let count: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);

    // A blob directory beside it, the way `postio-app` lays them out, so a
    // store seeded here is one the application can open unchanged.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let database = match postio_storage::Database::open(&path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            std::process::exit(1);
        }
    };

    let started = std::time::Instant::now();
    let report = postio_storage::seed::seed_large(&database, 7, count);
    println!(
        "seeded {} messages across {} folders into {} in {:.1}s",
        report.message_count,
        report.mailboxes.len(),
        path.display(),
        started.elapsed().as_secs_f64()
    );
}
