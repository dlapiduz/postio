//! What archiving a multi-select costs, split by which thread pays for it.
//!
//! postio-agr's third acceptance criterion — "list updates within the 16ms
//! budget" — was argued rather than measured: `Actions::run`
//! (`postio-app::actions`) is dispatched onto a tokio worker
//! (`postio_core::dispatch::Dispatcher::dispatch` calls `tokio::spawn`), never
//! onto the thread GTK runs on, so the interaction budget was never actually
//! at risk from the SQLite writes themselves. What *is* on that thread is
//! applying the events the action emits — `postio_gtk::feed::Feed::apply` —
//! and this is what proves that side stays cheap regardless of how many
//! messages were selected.
//!
//! # Two halves, two measurements
//!
//! * **The worker half** — `Actions::run` itself: resolve the selection, move
//!   the rows, enqueue the sync operations, commit. Reported for the record,
//!   not gated: it never touches the thread the budget is about. A large
//!   multi-select used to mean one `enqueue()` call per message, each opening
//!   its own savepoint and refreshing `has_pending_operations` with an
//!   `EXISTS` subquery — 500 selected rows was 1,000+ statements inside one
//!   transaction. `OperationQueueRepository::enqueue_many` replaced that loop
//!   with one `INSERT ... SELECT` and one flag `UPDATE`, whatever the
//!   selection's size; the number this bench prints is what that was worth.
//! * **The UI-thread half** — feeding the resulting events to a `Feed`. For a
//!   removal that is `Feed::reload()`: one `invalidate()` and one
//!   asynchronous page request, never a scan of the ids the event carries.
//!   This is gated against [`INTERACTION_BUDGET`], because this is the half
//!   the budget is actually about.
//!
//! # No display needed
//!
//! `postio_gtk::list::MessageList` is a plain `GListModel`, not a widget —
//! `postio-gtk/tests/feed.rs` already exercises `Feed` this way, "without a
//! database, a display or a runtime". The one thing carried over from that
//! file: only one function here drives the thread-default main context that
//! `glib::spawn_future_local` needs, because two would fight over it.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-app --bench action_round_trip
//! ```
//!
//! CI compiles this and does not time it — a shared runner is too noisy to
//! trust for a millisecond budget. The manual pass at the end asserts with a
//! real `Instant` regardless, so running it locally fails loudly on a genuine
//! regression rather than only reporting one.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor now
// reaches bench targets -- the old per-crate `#![warn(missing_docs)]` in
// `lib.rs` never did. A bench is not public API, so documenting a
// macro-generated item would be ceremony rather than information.

use std::future::pending;
use std::time::Instant;

use chrono::Utc;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use postio_app::actions::Actions;
use postio_core::bridge::{EventSink, EventStream, event_channel};
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_core::state::{AppState, SharedState};
use postio_core::{Command, Event, MessageTarget};
use postio_gtk::feed::{Feed, ListScope, MessageSource, PageFuture, PageRequest};
use postio_gtk::list::MessageList;
use postio_model::{Message, MessageId};
use postio_storage::Database;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A source that never answers.
///
/// Nothing here reads a page back — the bench is about what applying the
/// *action's* events costs, not about a page request resolving, so there is
/// nothing to give this a real answer for.
struct Never;

impl MessageSource for Never {
    fn fetch(&self, _request: PageRequest) -> PageFuture {
        Box::pin(pending())
    }
}

/// An account with an inbox and an archive, a bus over it, and a `Feed`
/// watching the inbox the way an open window's list would.
struct World {
    database: Database,
    account_id: postio_model::AccountId,
    inbox: postio_model::MailboxId,
    actions: Actions,
    state: SharedState,
    sink: EventSink,
    events: EventStream,
    feed: Feed,
    /// Kept alive: `Feed` only holds a weak reference to it.
    _list: MessageList,
}

fn world() -> World {
    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        test_support::mailbox(&connection, &account, "Archive");
        (account, inbox)
    };
    let state = SharedState::default();
    let (sink, events) = event_channel();
    let (quiet, _) = event_channel();
    state.update(&quiet, |app: &mut AppState| app.open_mailbox(inbox));

    let list = MessageList::new();
    let feed = Feed::new(&list, std::rc::Rc::new(Never));
    feed.open(ListScope::Mailbox(inbox));

    World {
        actions: Actions::new(database.clone(), state.clone()),
        database,
        account_id: account.id,
        inbox,
        state,
        sink,
        events,
        feed,
        _list: list,
    }
}

impl World {
    /// A fresh message in the inbox, selected the way clicking it would be.
    fn message(&self) -> MessageId {
        let connection = self.database.connection().expect("a connection");
        let mut message = Message::new(self.account_id, self.inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message")
    }

    fn select(&self, ids: &[MessageId]) {
        let (quiet, _) = event_channel();
        self.state.update(&quiet, |app: &mut AppState| {
            app.select(ids.to_vec(), ids.first().copied())
        });
    }

    /// `n` fresh messages in the inbox, selected. Excluded from every timed
    /// region: this is what a real selection already looked like before the
    /// key was pressed.
    fn seeded(&self, n: usize) -> Vec<MessageId> {
        let ids: Vec<MessageId> = (0..n).map(|_| self.message()).collect();
        self.select(&ids);
        ids
    }

    /// Archive the current selection, and feed every resulting event to the
    /// watching `Feed` — the whole round trip a keystroke sets off.
    fn archive_and_apply(&self) {
        self.actions
            .run(
                &Command::Archive {
                    target: MessageTarget::Selection,
                },
                &self.sink,
            )
            .expect("archive");
        while let Some(event) = self.events.try_next() {
            self.feed.apply(&event);
        }
    }

    fn drain(&self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(event) = self.events.try_next() {
            events.push(event);
        }
        events
    }
}

/// Selection sizes worth a number: one row, a screenful, and the size the
/// bead itself named as worth measuring.
const SIZES: [usize; 3] = [1, 50, 500];

/// How many times the budget check runs before taking the fastest — the same
/// tolerance `thread_drill.rs` uses, for the same reason: a single timing on
/// a machine building something else measures the scheduler, not the code.
const RUNS: usize = 5;

fn bench_archive_round_trip(c: &mut Criterion) {
    let world = world();

    for n in SIZES {
        c.bench_function(&format!("archive round trip, {n} selected"), |b| {
            // `PerIteration`, not `SmallInput` (#622): `seeded` doesn't just
            // build a value to hand `archive_and_apply` — it also overwrites
            // `world.state`'s selection, which `archive_and_apply` reads back
            // through `MessageTarget::Selection`, the same route a real `a`
            // keypress takes. `SmallInput` collects a whole batch of `seeded`
            // calls before running any routine, so only the batch's *last*
            // selection survives to be read; every routine call after the
            // first then finds its target already archived and is rejected.
            // `PerIteration` runs one `seeded` immediately before each
            // `archive_and_apply`, which is what this setup's side effect
            // requires.
            b.iter_batched(
                || world.seeded(n),
                |_ids| world.archive_and_apply(),
                BatchSize::PerIteration,
            )
        });
    }

    // Criterion reports; this fails. A budget nobody notices breaking is not
    // a budget, which is why `postio-core`'s own budget benches assert as
    // well as measure.
    for n in SIZES {
        let mut worker_best = None;
        let mut ui_best = None;
        for _ in 0..RUNS {
            world.seeded(n);
            let worker_start = Instant::now();
            world
                .actions
                .run(
                    &Command::Archive {
                        target: MessageTarget::Selection,
                    },
                    &world.sink,
                )
                .expect("archive");
            let worker_elapsed = worker_start.elapsed();

            let events = world.drain();
            let ui_start = Instant::now();
            for event in &events {
                world.feed.apply(event);
            }
            let ui_elapsed = ui_start.elapsed();

            worker_best = Some(
                worker_best.map_or(worker_elapsed, |best: std::time::Duration| {
                    best.min(worker_elapsed)
                }),
            );
            ui_best =
                Some(ui_best.map_or(ui_elapsed, |best: std::time::Duration| best.min(ui_elapsed)));
        }
        let worker_best = worker_best.expect("at least one run");
        let ui_best = ui_best.expect("at least one run");

        eprintln!(
            "archive of {n} selected: worker (off the UI thread) {worker_best:?}, \
             applying the resulting events (on it) {ui_best:?} against a \
             {INTERACTION_BUDGET:?} budget"
        );

        // The claim this bench exists to check: applying what the action
        // emitted costs the same whether one row moved or five hundred did,
        // because a removal is `Feed::reload()` and never a scan of the ids
        // the event happens to carry.
        if let Err(exceeded) = check_budget(ui_best, INTERACTION_BUDGET) {
            panic!(
                "applying the events from archiving {n} selected messages is \
                 over budget: {exceeded:?} (best of {RUNS})"
            );
        }
    }
}

criterion_group!(benches, bench_archive_round_trip);
criterion_main!(benches);
