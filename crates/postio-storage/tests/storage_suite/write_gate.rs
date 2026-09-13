//! The write gate hands the lock to a person before a backfill. #425.
//!
//! `postio-session/tests/interactive_write.rs` is the end-to-end claim — an
//! archive keystroke does not queue behind a first sync. This is the property
//! underneath it, stated where it can be asserted without a mail server: that
//! priority, not arrival order, decides who writes next.
//!
//! That is the whole reason the gate exists. SQLite's own answer to two
//! writers is `busy_timeout`, which is a retry loop with no ordering in it at
//! all — so "the interactive writer goes first" is not something the database
//! can be asked for, and has to be arranged above it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use postio_storage::{WriteGate, WritePriority, test_support};

/// Long enough for a task that has said it is about to block to actually be
/// blocked. Not a performance assertion — it only establishes *arrival order*,
/// which is the thing these tests need to be able to set up. Both tests would
/// still be correct if it were longer; they would merely be slower.
const ENOUGH_TO_BLOCK: Duration = Duration::from_millis(50);

/// Spin until an interactive writer has registered itself.
///
/// `interactive_is_waiting` is the gate's own observable, and waiting on it is
/// what lets these tests establish arrival order without a sleep. The yield is
/// what makes it safe on a single-worker runtime: a spin loop that never
/// awaits starves the very task it is waiting for.
async fn until_interactive_is_waiting(gate: &WriteGate) {
    while !gate.interactive_is_waiting() {
        tokio::task::yield_now().await;
    }
}

async fn gate() -> WriteGate {
    // Through a real database, because that is how every caller reaches one
    // and it is worth knowing the wiring is there.
    test_support::memory().await.write_gate().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interactive_writer_goes_first_even_though_it_asked_second() {
    let gate = gate().await;
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    // Nobody writes until this is dropped, so both threads below are
    // definitely queued rather than racing to be first.
    let blocking = gate.acquire(WritePriority::Interactive).await;

    // The backfill asks first...
    let background = tokio::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        async move {
            let _permit = gate.acquire(WritePriority::Background).await;
            order.lock().unwrap().push("background");
        }
    });
    tokio::time::sleep(ENOUGH_TO_BLOCK).await;

    // ...and the keystroke asks second.
    let interactive = tokio::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        async move {
            let _permit = gate.acquire(WritePriority::Interactive).await;
            order.lock().unwrap().push("interactive");
        }
    });
    // Observable rather than slept on: the interactive writer counts itself as
    // waiting before it blocks, which is exactly what the background writer
    // has to be able to see.
    until_interactive_is_waiting(&gate).await;

    drop(blocking);
    interactive.await.expect("the interactive writer finishes");
    background.await.expect("the background writer finishes");

    assert_eq!(
        *order.lock().unwrap(),
        vec!["interactive", "background"],
        "the backfill was already queued and took the lock anyway. Arrival \
         order is what SQLite's own busy_timeout gives, and giving it is the \
         bug — a person's write has to overtake bulk work that got there \
         first, or a first sync locks them out for as long as it runs."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_writer_waits_for_a_queued_interactive_one() {
    // The same property from the other side, and the one that actually bounds
    // the wait: a background writer must not *begin* while an interactive
    // writer is waiting. Beginning and then yielding would be yielding after
    // taking SQLite's lock, which is too late to help.
    let gate = gate().await;

    let blocking = gate.acquire(WritePriority::Background).await;

    let waiting = tokio::spawn({
        let gate = gate.clone();
        async move {
            let permit = gate.acquire(WritePriority::Interactive).await;
            // Held, so the background attempt below has something to fail
            // against rather than a lock that is merely free.
            tokio::time::sleep(ENOUGH_TO_BLOCK).await;
            drop(permit);
        }
    });
    until_interactive_is_waiting(&gate).await;

    drop(blocking);

    // With an interactive writer queued, this must not be granted until that
    // one has come and gone.
    let started = std::time::Instant::now();
    let _permit = gate.acquire(WritePriority::Background).await;
    let waited = started.elapsed();

    assert!(
        !gate.interactive_is_waiting(),
        "a background writer was granted the lock with an interactive writer \
         still queued behind it"
    );
    assert!(
        waited >= ENOUGH_TO_BLOCK / 2,
        "the background writer was granted the lock immediately ({waited:?}), \
         so it did not wait for the interactive writer that was already queued"
    );
    waiting.await.expect("the interactive writer finishes");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_interactive_writers_do_not_hold_the_lock_at_once() {
    // The gate is a lock as well as a queue: whatever the priorities, exactly
    // one permit is outstanding at a time. Without this the sync batch and a
    // keystroke could both be inside `BEGIN IMMEDIATE`, which is the
    // SQLITE_BUSY that #79 was.
    let gate = gate().await;
    let holders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let tasks: Vec<_> = (0..8)
        .map(|n| {
            let gate = gate.clone();
            let holders = Arc::clone(&holders);
            let peak = Arc::clone(&peak);
            let priority = if n % 2 == 0 {
                WritePriority::Interactive
            } else {
                WritePriority::Background
            };
            tokio::spawn(async move {
                for _ in 0..50 {
                    let _permit = gate.acquire(priority).await;
                    let now = holders.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    holders.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.expect("a writer finishes");
    }

    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "two writers held the gate at the same time"
    );
}

/// A writer that asks once is served, however busy its rivals are.
///
/// The gate wakes **every** waiter on release and lets them race for the
/// mutex. That was harmless while the only background writers were the
/// occasional ones — a resync, an egress flush, a housekeeping sweep — and
/// stopped being harmless when the body backfill started taking a permit for
/// every body and header it writes. A folder sync's single acquisition then
/// has to win a race against a continuous stream of them, every time, for
/// ever.
///
/// Observed on a live account: a `Drafts` sync of twenty-five messages
/// started and had not finished seven minutes later, and because a wave does
/// not return until all of its passes do, **no other folder was ever
/// synced** — 59,000 messages of `Archive` stayed on the server.
///
/// The loop below is the backfill's shape: acquire, do a little work, release,
/// immediately ask again.
#[tokio::test(flavor = "multi_thread")]
async fn a_lone_writer_is_not_starved_by_a_busy_one() {
    let gate = gate().await;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let busy = tokio::spawn({
        let gate = gate.clone();
        let stop = Arc::clone(&stop);
        async move {
            let mut taken = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let permit = gate.acquire(WritePriority::Background).await;
                taken += 1;
                tokio::task::yield_now().await;
                drop(permit);
            }
            taken
        }
    });

    // Let the busy writer get properly under way, so the lone one arrives
    // into real contention rather than an idle gate.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lone = tokio::time::timeout(Duration::from_secs(10), async {
        let _permit = gate.acquire(WritePriority::Background).await;
    })
    .await;

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let taken = busy.await.expect("the busy writer");

    assert!(
        lone.is_ok(),
        "a writer that asked once never got the gate while a rival took it \
         {taken} times. A folder sync starves behind the body backfill this \
         way, and one starved pass stops every later sync wave."
    );
}
