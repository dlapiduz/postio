//! What a page cache smaller than the working set costs, under SQLCipher.
//!
//! #1216 chased a burn that no test could reproduce: a real mailbox spent
//! 62% of its sampled CPU in SQLCipher page work, 45.9% of it in
//! `sha512_block_data_order_avx2`, with `__libc_pread64` at 9.1% proving the
//! reads reached disk. Every synthetic store stayed flat -- `idle_store_cpu.rs`
//! measures 100, 5,000 and 50,000 messages and finds no difference -- because
//! all of them fit the cache.
//!
//! `db.rs` names the lever in the pragma's own doc comment:
//!
//! > `cache_size = -16000` ... **This is the first lever if a bench trips,
//! > because under SQLCipher every page miss costs a decrypt rather than a
//! > `memcpy`.**
//!
//! This is that bench. It is `#[ignore]`d: it seeds 400,000 messages and takes
//! about three and a half minutes, which is a bench's cost and not a gate's.
//!
//! ```text
//! cargo test -p postio-storage --test cache_pressure -- --ignored --nocapture
//! ```
//!
//! # What it showed
//!
//! At 60,000 messages the working set already fits 16 MiB, and more cache buys
//! nothing -- which is exactly why the default has never looked wrong:
//!
//! ```text
//! cache     512 KiB -> 1.64s      cache   16000 KiB -> 420ms
//! cache    2000 KiB -> 1.34s      cache  262144 KiB -> 450ms
//! ```
//!
//! At 400,000 it does not:
//!
//! ```text
//! cache   16000 KiB -> 4.29s
//! cache   65536 KiB -> 3.27s   (-24%)
//! cache  262144 KiB -> 3.16s   (-26%)
//! cache   16000 KiB -> 4.16s   <- the control
//! ```
//!
//! The last row is the point. Repeating the smallest cache *after* the largest
//! returns the cost, so this is the cache and not the file warming up, the
//! page order, or the run getting faster as it goes.
//!
//! # What it does not show
//!
//! That 64 MiB is the right number. The working set is a property of the
//! mailbox and the query, and these rows are synthetic; a real store of the
//! same message count carries larger rows. What generalises is the shape --
//! below the working set the cost rises sharply, above it more cache is free
//! to no one -- and that a fixed 16 MiB cannot be right for both a test store
//! and a gigabyte mailbox.

use std::time::{Duration, Instant};

use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::seed::seed_large;
use postio_storage::test_support;

/// How many messages to seed. Chosen to be past where 16 MiB suffices.
const MESSAGES: usize = 400_000;

/// Cache sizes in KiB, ending where it began: the repeat is the control.
const SIZES: [i64; 4] = [16_000, 65_536, 262_144, 16_000];

/// This process's CPU time, user plus system.
fn cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    let tail = &stat[stat.rfind(')').expect("the comm field ends") + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    Duration::from_secs_f64((utime + stime) as f64 / 100.0)
}

/// Page deep into `mailbox` and back, with the cache set to `kib`.
fn sweep(
    database: &postio_storage::Database,
    mailbox: postio_model::MailboxId,
    kib: i64,
) -> (Duration, usize) {
    let connection = database.connection().expect("a connection");
    connection
        .pragma_update(None, "cache_size", -kib)
        .expect("set the cache");
    // So each size starts from the same place rather than inheriting the last
    // one's pages -- without this the sweep measures the order it ran in.
    let _ = connection.pragma_update(None, "shrink_memory", 1i64);

    let before = cpu();
    let mut seen = 0usize;
    for _ in 0..3 {
        for offset in (0..80_000).step_by(500) {
            seen += MessageRepository::new(&connection)
                .page_at(&ListQuery::mailbox(mailbox), offset)
                .expect("a page")
                .len();
        }
    }
    (cpu().saturating_sub(before), seen)
}

#[test]
#[ignore = "seeds 400,000 messages; a bench, not a gate"]
fn a_cache_below_the_working_set_costs_cpu() {
    let database = test_support::memory();
    let report = seed_large(&database, 11, MESSAGES);
    // The inbox: `seed_large` weights most of its messages there.
    let mailbox = report
        .mailboxes
        .iter()
        .find(|m| m.role == postio_model::MailboxRole::Inbox)
        .or_else(|| report.mailboxes.first())
        .expect("a mailbox")
        .id;
    eprintln!("seeded {} messages", report.message_count);

    let started = Instant::now();
    let mut burned = Vec::new();
    for kib in SIZES {
        let (cost, seen) = sweep(&database, mailbox, kib);
        assert!(seen > 0, "the workload read nothing at {kib} KiB");
        eprintln!("cache {kib:>7} KiB -> {cost:?}");
        burned.push(cost);
    }
    eprintln!("(whole sweep {:?})", started.elapsed());

    let small = burned[0];
    let large = burned[1];
    let control = burned[3];

    assert!(
        large < small,
        "a larger cache did not help: {small:?} at 16 MiB against {large:?} at 64 MiB. \
         Either the working set now fits 16 MiB -- raise MESSAGES -- or the lever \
         `db.rs` names has stopped being one."
    );
    // The control is what makes the line above mean anything: if the run simply
    // got faster as it went, this would be fast too.
    assert!(
        control > large,
        "the smallest cache was cheap when it ran last ({control:?} against {small:?} \
         when it ran first), so this measured the order of the sweep rather than \
         the cache."
    );
}
