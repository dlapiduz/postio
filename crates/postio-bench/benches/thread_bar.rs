#![allow(missing_docs)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use chrono::{Duration as Span, Local, TimeZone, Utc};
use postio_model::thread_view::{Line, ThreadMember};
use postio_model::{AccountId, EmailAddress, MessageId, Thread};
use postio_ui::thread_bar::chips;

const MEMBERS: usize = 500;

const SAMPLES: usize = 2_000;

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
