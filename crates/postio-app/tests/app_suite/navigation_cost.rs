//! What the transitions between surfaces cost (#1216).
//!
//! The budgets in `CLAUDE.md` are `<500 ms` to usable and `<16 ms` for an
//! ordinary interaction. Search and the reader have been measured to death;
//! the *navigations* had not been measured at all.
//!
//! This drives the real window through `Window::act` and the sidebar -- the
//! same doors a keystroke and a click use -- over a 20,000-message seeded
//! store, and prints what each costs.
//!
//! # What it found, and what that excludes
//!
//! ```text
//! [plumbing] empty round trip        ~50-113 us
//! [store]    thread page (50 rows)      ~2 ms
//! [store]    count(*)                  ~0.6 ms
//! act (sidebar widget work)             ~30 ms
//! folder switch -> first row on screen  ~1.2 s
//! open the composer                     ~93 ms
//! ```
//!
//! **The store is not what a folder switch waits for, and neither is the
//! transport.** A page answers in two milliseconds; a job carrying nothing
//! through the same runtime, channel and main context round-trips in fifty
//! microseconds; and the synchronous widget work is thirty. The remaining
//! ~1.2 s is in the feed, the list model, or GTK -- and it is consistent
//! across rounds, so it is not a cold start.
//!
//! Recorded here rather than guessed at. The three lines above are what a
//! next attempt does not have to re-measure.
//!
//! The composer at ~93 ms is over the 16 ms interaction budget on its own.
//!
//! # The bound
//!
//! Deliberately loose, and it is a *guard* rather than the finding: this runs
//! on a shared machine under a headless compositor, and the point is to catch
//! a transition that has become many seconds. The printed numbers are what to
//! read.

#![allow(unsafe_code)]
// `std::env::set_var` is unsafe since Rust 2024. Set as the first statement of
// a single-threaded case, which is the one moment it is sound.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::BlobStore;
use postio_storage::seed::seed_large;
use postio_storage::test_support;

use crate::settle_until;

/// Messages to seed. Large enough that a folder switch has to page rather
/// than answer from a handful of rows.
const MESSAGES: usize = 20_000;

/// What a transition may cost before it is a bug rather than a measurement.
///
/// Deliberately loose: this runs on a shared machine under a headless
/// compositor, and the point is to catch a transition that has become
/// *seconds*, not to police tens of milliseconds. The printed numbers are the
/// finding; the assertion is the guard.
const OUTER_BOUND: Duration = Duration::from_millis(5_000);

fn timed(label: &str, mut body: impl FnMut()) -> Duration {
    let started = Instant::now();
    body();
    let took = started.elapsed();
    eprintln!("  {label:<34} {took:>10.2?}");
    assert!(
        took < OUTER_BOUND,
        "{label} took {took:?}, past the {OUTER_BOUND:?} this asserts -- a \
         transition that slow is a defect, not a slow machine"
    );
    took
}

pub fn switching_surfaces_stays_within_a_blink() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_large(&database, 11, MESSAGES);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        postio_core::bridge::event_channel().0,
        bridge.commands(),
    );
    let window = Window::default();
    window.set_default_size(1280, 800);
    window.present();
    let _ = feed_the_window(&window, &wiring);
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the seeded store should fill the list"
    );

    // The plumbing on its own: a trivial job through the same runtime and
    // channel a page fetch uses, awaited on the main context the same way.
    // If a round trip carrying nothing costs what a page costs, the store is
    // not what a folder switch is waiting for.
    for round in 1..=3 {
        let (tx, rx) = async_channel::bounded::<u8>(1);
        let started = Instant::now();
        wiring.runtime.spawn(async move {
            let _ = tx.send(1).await;
        });
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        glib::spawn_future_local({
            let done = done.clone();
            async move {
                let _ = rx.recv().await;
                done.set(true);
            }
        });
        settle_until(|| done.get());
        eprintln!(
            "  [plumbing] empty round trip {round}   {:>10.2?}",
            started.elapsed()
        );
    }

    // A settle that pumps without sleeping, to tell real waiting from this
    // harness's own 10 ms polling granularity. If a folder switch shrinks by
    // roughly ten times under it, the second it appeared to take was the
    // measurement, not the app.
    fn settle_tight(done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
        while Instant::now() < deadline {
            while gtk::glib::MainContext::default().iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        done()
    }

    eprintln!("navigation over {} messages:", report.message_count);

    // The store side of a folder switch, timed directly, so the UI numbers
    // below can be attributed rather than guessed at.
    {
        use postio_storage::repository::{ListQuery, MessageRepository};
        let connection = database.connection().expect("a connection");
        let mailbox = report
            .mailboxes
            .iter()
            .find(|m| m.selectable)
            .expect("a mailbox")
            .id;
        let query = ListQuery::mailbox(mailbox);
        let started = Instant::now();
        let rows = MessageRepository::new(&connection)
            .page(&query)
            .expect("a page");
        eprintln!(
            "  [store] first page ({} rows)      {:>10.2?}",
            rows.len(),
            started.elapsed()
        );
        let started = Instant::now();
        let total: i64 = connection
            .query_row(
                "SELECT total FROM mailboxes WHERE id = ?1",
                [mailbox.get()],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        eprintln!(
            "  [store] cached total ({total})       {:>10.2?}",
            started.elapsed()
        );
        // The query a folder switch *actually* runs: folders thread, so the
        // list is paged by conversation rather than by message (ADR 0015).
        {
            use postio_storage::repository::{ThreadListQuery, ThreadRepository};
            let started = Instant::now();
            let threads = ThreadRepository::new(&connection)
                .page(&ThreadListQuery {
                    account_id: report.account.id,
                    mailbox: Some(mailbox),
                    limit: 50,
                    after: None,
                })
                .expect("a thread page");
            eprintln!(
                "  [store] THREAD page ({} rows)     {:>10.2?}",
                threads.len(),
                started.elapsed()
            );
        }
        let started = Instant::now();
        let counted: i64 = connection
            .query_row(
                "SELECT count(*) FROM messages WHERE mailbox_id = ?1 AND deleted_locally = 0",
                [mailbox.get()],
                |r| r.get(0),
            )
            .expect("a count");
        eprintln!(
            "  [store] count(*) ({counted})          {:>10.2?}",
            started.elapsed()
        );
    }

    // ── switching folder, there and back ────────────────────────────────
    let folders: Vec<_> = report
        .mailboxes
        .iter()
        .filter(|mailbox| mailbox.selectable)
        .take(2)
        .map(|mailbox| mailbox.id)
        .collect();
    if folders.len() == 2 {
        timed("switch folder", || {
            window.sidebar().select(folders[1]);
            settle_until(|| window.sidebar().selected() == Some(folders[1]));
        });
        timed("switch back", || {
            window.sidebar().select(folders[0]);
            settle_until(|| window.sidebar().selected() == Some(folders[0]));
        });
    }

    // ── the composer taking over the pane ───────────────────────────────
    // Repeated, because the first composition is the one a person notices and
    // the ones after it are what it should cost. The editing surface is a
    // `WebView` whose first load starts a web process, so before #1216's warm
    // the first open was 34ms against 9ms for the rest.
    for round in 1..=5 {
        let widgets_before = postio_gtk::row::rows_built();
        let started = Instant::now();
        window.act(postio_core::Command::Compose { draft: None });
        let acted = started.elapsed();
        let started = Instant::now();
        settle_until(|| window.composer().is_open());
        let settled = started.elapsed();
        eprintln!(
            "  composer round {round}: act {acted:>10.2?}  settle {settled:>10.2?}  rows {}",
            postio_gtk::row::rows_built() - widgets_before
        );
        window.composer().discard();
        settle_until(|| !window.composer().is_open());
    }
    window.act(postio_core::Command::Compose { draft: None });
    settle_until(|| window.composer().is_open());

    // ── reordering the list ─────────────────────────────────────────────
    // Repeated, because the first of anything pays for what the others find
    // warm, and one reading cannot tell a slow transition from a cold one.
    for round in 1..=4 {
        // Split, because `Feed::open` spawns the page fetch rather than
        // running it: if the cost is in `act` it is synchronous widget work,
        // and if it is in the pump it is the store answering.
        let widgets_before = postio_gtk::row::rows_built();
        let fetches_before = postio_gtk::feed::fetches();
        let emissions_before = postio_gtk::list::emissions();
        let started = Instant::now();
        window.act(postio_core::Command::NextFolder);
        let acted = started.elapsed();
        let started = Instant::now();
        settle_tight(|| window.list().model().n_items() > 0);
        let settled = started.elapsed();
        eprintln!("  next folder, round {round}: act {acted:>10.2?}  settle {settled:>10.2?}");
        eprintln!(
            "      built {} row widgets, {} page fetches; window {}x{}, list {}x{}",
            postio_gtk::row::rows_built() - widgets_before,
            postio_gtk::feed::fetches() - fetches_before,
            window.width(),
            window.height(),
            window.list().width(),
            window.list().height()
        );
        eprintln!(
            "      {} items_changed",
            postio_gtk::list::emissions() - emissions_before
        );
        eprintln!(
            "      landed on {:?}, list holds {}",
            window.sidebar().selected(),
            window.list().model().n_items()
        );
    }
    for round in 1..=3 {
        timed(&format!("prev folder, round {round}"), || {
            window.act(postio_core::Command::PrevFolder);
            settle_until(|| true);
        });
    }
}
