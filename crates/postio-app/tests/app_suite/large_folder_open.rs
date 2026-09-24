//! Opening a folder costs a screen's worth of pages, whatever it holds
//! (#1534).
//!
//! A real account could not open its Archive: 60,907 messages, 36,183
//! conversation rows, and the list issued **49 distinct page requests in 20
//! seconds** at scattered offsets — 330, 542, 391, 178, 66, 463 — one
//! starting as the last was answered, never settling. The same session opened
//! INBOX (128 rows) in three sequential reads and stopped.
//!
//! `ListWindow` keeps [`CACHE_PAGES`] resident, which is eight, so a working
//! set wider than eight pages evicts itself and every evicted position is
//! asked for again. The cache is not the bug — it is what turns whatever is
//! asking widely into an unbounded loop instead of a slow open.
//!
//! **Why nothing caught it.** A page request is an ordinary windowed query,
//! so `postio_storage`'s counters see nothing unusual; each read is fast and
//! correct. And the seeded fixtures are in the low thousands, which fits the
//! cache — the defect appears only once a folder outgrows it, which is to say
//! after a year of archiving.
//!
//! Counted rather than timed, per Principle V: what a folder costs to open is
//! roughly linear in page requests, and that number is the same on every
//! machine.
//!
//! One test function, for the reason `wiring.rs` gives.

use crate::settle_until;
use gtk::glib;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_session::Wiring;
use postio_storage::seed::seed_large;
use postio_storage::{BlobStore, test_support};

/// Rows enough to outgrow the eight-page cache several times over.
///
/// `CACHE_PAGES` is 8 and `PAGE_SIZE` is 50, so 400 rows fit. Twelve thousand
/// messages leaves the biggest folder far past that while seeding in a few
/// seconds — the real account was 60,907, and the failure does not need the
/// full size, only more than the cache holds.
const MESSAGES: usize = 12_000;

pub fn opening_a_large_folder_asks_for_a_bounded_number_of_pages() {
    crate::gtk_case(async {
        if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }

        let database = test_support::memory().await;
        let seeded = seed_large(&database, 11, MESSAGES).await;
        // In conversations, as a real folder lists them: landing on a row is
        // then opening a conversation, which is the path the `j` presses
        // below are about.
        postio_storage::seed::thread_seeded_messages(&database, seeded.account.id, 3).await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        // The folder the seed weights most of its messages into, which is the one
        // that outgrows the cache.
        let (biggest, rows) = {
            let connection = database.connect().await.expect("a connection");
            postio_storage::sql::one(
                &connection,
                "SELECT mailbox_id, count(*) FROM messages
              GROUP BY mailbox_id ORDER BY count(*) DESC LIMIT 1",
                (),
                |row| {
                    use postio_storage::sql::RowExt as _;
                    Ok((row.col::<i64>(0)?, row.col::<i64>(1)?))
                },
            )
            .await
            .expect("the seed put messages somewhere")
        };
        assert!(
            rows > 2_000,
            "the fixture needs a folder far past the {} rows the cache holds, got {rows}",
            postio_gtk::list::CACHE_PAGES as u32 * postio_gtk::list::PAGE_SIZE
        );

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account");

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 0).await,
            "the opening folder never filled"
        );

        // ── open the big one, and count what that costs ──────────────────────
        let before = postio_ui::test_support::pages_requested();
        wired.feeds.messages.open(postio_model::ListScope::Mailbox(
            postio_model::ids::MailboxId::new(biggest),
        ));
        assert!(
            settle_until(async || list.model().n_items() as i64 > 0).await,
            "the large folder never filled"
        );

        // Let anything still coming actually arrive, or this asserts that a loop
        // has not finished rather than that there is not one. Generous, because
        // the defect is a loop that runs until something stops it: if requests
        // are still arriving after this, they were never going to stop.
        for _ in 0..200 {
            while glib::MainContext::default().iteration(false) {}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let asked = postio_ui::test_support::pages_requested() - before;
        eprintln!(
            "DIAG rows={rows} n_items={} asked={asked}",
            list.model().n_items()
        );
        // A screen of rows is one page, and a list may reasonably read a couple
        // either side plus the page a cursor lands on. `CACHE_PAGES` is the whole
        // budget: asking for more than fits means something is being evicted and
        // asked for again, which is the loop.
        let budget = postio_gtk::list::CACHE_PAGES as u64;
        assert!(
            asked <= budget,
            "opening a {rows}-row folder asked for {asked} pages, and only {budget} \
         fit in the cache — so at least {} of them were evicted and asked for \
         again. A folder costs a screen to open, whatever it holds (#1534).",
            asked.saturating_sub(budget)
        );

        // ── and moving through it ─────────────────────────────────────────────
        //
        // Landing on a row opens its conversation, and the subset the list
        // already holds goes up first. That subset was found by walking every
        // position in the model -- and a position the window does not hold
        // is a page request, so each `j` asked for the whole folder.
        let before = postio_ui::test_support::pages_requested();
        for _ in 0..5 {
            window.handle_key(gtk::gdk::Key::j, gtk::gdk::ModifierType::empty());
            while glib::MainContext::default().iteration(false) {}
        }
        for _ in 0..100 {
            while glib::MainContext::default().iteration(false) {}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let walked = postio_ui::test_support::pages_requested() - before;
        eprintln!("DIAG five j presses asked={walked}");
        assert!(
            walked <= 1,
            "five `j` presses near the top of a {rows}-row folder asked for \
             {walked} pages; the rows they land on are already on screen"
        );

        // ── and the way it was actually met: switch while it is still loading ──
        //
        // "I switch folders when the 'inbox is empty' message is displayed and I
        // switch again and the app loses track." Each switch resets the window and
        // bumps the generation, so replies for the folder just left arrive into a
        // window that has moved on. What must not happen is that the list ends up
        // asking for pages without ever settling.
        let folders: Vec<i64> = {
            let connection = database.connect().await.expect("a connection");
            postio_storage::sql::all(
            &connection,
            "SELECT mailbox_id, count(*) FROM messages GROUP BY mailbox_id ORDER BY count(*) DESC",
            (),
            |row| {
                use postio_storage::sql::RowExt as _;
                row.col::<i64>(0)
            },
        )
        .await
        .expect("query")
        };
        assert!(folders.len() >= 2, "need two folders to switch between");

        let before = postio_ui::test_support::pages_requested();
        // No settling in between: that is the gesture.
        for folder in folders.iter().chain(folders.iter()).take(8) {
            wired.feeds.messages.open(postio_model::ListScope::Mailbox(
                postio_model::ids::MailboxId::new(*folder),
            ));
            // One turn of the loop, which is all a fast hand gives it.
            while glib::MainContext::default().iteration(false) {}
        }

        // Now let it settle, and keep letting it: the failure is a list that never
        // stops asking.
        for _ in 0..200 {
            while glib::MainContext::default().iteration(false) {}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let churned = postio_ui::test_support::pages_requested() - before;

        // Eight switches, each entitled to a screenful. Anything far past that is
        // a list that kept asking after the hand stopped moving.
        let switch_budget = 8 * postio_gtk::list::CACHE_PAGES as u64;
        assert!(
            churned <= switch_budget,
            "switching folders eight times asked for {churned} pages, past the \
         {switch_budget} that eight screenfuls would cost — the list did not \
         settle (#1534)"
        );
        assert!(
            list.model().n_items() > 0,
            "after switching, the list shows nothing at all: the app lost track \
         of which folder it is on (#1534)"
        );
    })
}
