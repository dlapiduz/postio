//! The message window's thread bar at 500 members (spec 013 T086, plan.md
//! step 4), against the 16 ms interaction budget.
//!
//! `postio_ui::thread_bar::chips` is a pure function the controller runs on
//! every step of the window's walk and on every resize, over the whole
//! thread. This times it over a thread of 500 messages at the widths a
//! window has: eight chips (a narrow window, where the middle folds into
//! "... N more"), and room for every one. A search matching a tenth of the
//! thread is the worst case for the `matched` lookup.
//!
//! **Reported, never gated** (Constitution V): `bench.yml` compiles this and
//! times nothing. Per plan.md's stop rule, a p95 over 16 ms stops step 4 and
//! asks the maintainer, with the option of capping the read.
//!
//! ```sh
//! cargo bench -p postio-bench --bench thread_bar
//! ```

#![allow(missing_docs)]
// A bench is not public API.

use std::hint::black_box;
use std::time::{Duration, Instant};

use chrono::{Duration as Span, Local, TimeZone, Utc};
use postio_model::thread_view::{Line, ThreadMember};
use postio_model::{AccountId, EmailAddress, MessageId, Thread};
use postio_ui::thread_bar::chips;

/// Members of the thread: past anything the seed has.
const MEMBERS: usize = 500;

/// Calls timed per case.
const SAMPLES: usize = 2_000;

/// The thread, as the store holds it.
fn thread() -> Thread {
    let first = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
    Thread {
        subject: Some("Harbor API draft v3".to_owned()),
        message_ids: (1..=MEMBERS as i64).map(MessageId::new).collect(),
        message_count: MEMBERS as u32,
        first_at: first,
        last_at: first + Span::hours(MEMBERS as i64),
        ..Thread::new(AccountId::new(1))
    }
}

/// What reading it gives: one member per message, a tenth unread.
fn members(thread: &Thread) -> Vec<ThreadMember> {
    let names = ["Ben Adeyemi", "Ana Ruiz", "Lena Park", "Tomas Reyes"];
    thread
        .message_ids
        .iter()
        .enumerate()
        .map(|(index, id)| ThreadMember {
            id: *id,
            from: Some(EmailAddress::new(
                Some(names[index % names.len()]),
                "someone@example.net",
            )),
            own: index % 7 == 0,
            at: thread.first_at + Span::hours(index as i64),
            seen: index % 10 != 0,
            has_attachments: false,
            line: Line::Quoted,
            answers: None,
            marker: false,
        })
        .collect()
}

/// p50 and p95 of `run` over [`SAMPLES`] calls.
fn percentiles(mut run: impl FnMut()) -> (Duration, Duration) {
    for _ in 0..50 {
        run();
    }
    let mut taken: Vec<Duration> = (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect();
    taken.sort();
    (taken[SAMPLES / 2], taken[SAMPLES * 95 / 100])
}

fn main() {
    let thread = thread();
    let members = members(&thread);
    let now = Local.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
    let matches: Vec<MessageId> = thread.message_ids.iter().step_by(10).copied().collect();
    println!("thread_bar::chips over {MEMBERS} members, {SAMPLES} calls each");
    println!("{:<34} {:>12} {:>12}", "case", "p50", "p95");
    for (name, room, found) in [
        ("room 8, no search", 8_u32, &[][..]),
        ("room 8, 50 matches", 8, &matches[..]),
        ("room 500, no search", 500, &[][..]),
        ("room 500, 50 matches", 500, &matches[..]),
    ] {
        let (p50, p95) = percentiles(|| {
            black_box(chips(
                black_box(&members),
                black_box(MEMBERS / 2),
                black_box(found),
                black_box(room),
                now,
            ));
        });
        println!("{name:<34} {p50:>12.2?} {p95:>12.2?}");
    }
}
