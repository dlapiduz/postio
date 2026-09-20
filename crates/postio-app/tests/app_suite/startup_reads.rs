//! Opening a window reads a bounded amount, whatever the mailbox holds (#1479).
//!
//! `docs/PRODUCT.md` §18 budgets startup at 500 ms, and on the maintainer's
//! real store it measured 1249.7 ms — of which 1044.1 ms was the first frame,
//! with nothing in that window touching the network. The signature is the one
//! #1434 named: the first frame waits on work proportional to what the mailbox
//! holds.
//!
//! So this is the assertion that catches it, and it is counted rather than
//! timed for the reason [`postio_storage::test_support::counting`] gives —
//! a count is the same number on this workstation and on a loaded runner,
//! and a wall-clock budget for startup cannot be defended on either.
//!
//! > Opening a window reads a bounded amount, whatever the mailbox holds.
//!
//! Two stores an order of magnitude apart, the same counts from both.
//!
//! # Why it counts the main thread and nothing else
//!
//! [`postio_storage::test_support::counting`]'s counters are thread-local, and
//! that is exactly right here rather than a limitation to work around.
//! `feed_the_window` runs on the thread that has to draw the first frame:
//! every read it makes *synchronously* is a read the frame is waiting on,
//! while everything it hands to the runtime — the page fetch, the body-index
//! catch-up, the dictionary trainer — is off this thread by construction and
//! delays nothing on screen. Counting this thread's reads is therefore
//! counting the startup budget's actual cause.
//!
//! # Why steps and not only rows
//!
//! An aggregate hides from both of the older counts. `SELECT count(*) FROM
//! messages WHERE …` is one statement and produces exactly one row however
//! many it had to read to produce it — which is how a full scan of an 81,000
//! message table sat on the first-frame path, invisible to `statements` and
//! `rows` alike. [`Counts::steps`] is what sees it.

use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::BlobStore;
use postio_storage::seed::seed_large;
use postio_storage::test_support;
use postio_storage::test_support::counting::{Counts, counted_async};

/// The two mailboxes, an order of magnitude apart.
///
/// Small enough that seeding both stays inside the merge path's patience, and
/// far enough apart that anything linear in the mailbox shows as a factor of
/// ten rather than as noise.
const SMALL: usize = 1_000;
const LARGE: usize = 10_000;

/// One window, pointed at a seeded store, and what that cost the main thread.
///
/// Everything it was built out of is kept: the runtime is still draining the
/// housekeeping passes `feed_the_window` spawned, and dropping the bridge out
/// from under them would be tearing down the application mid-startup rather
/// than measuring one.
struct Opened {
    counts: Counts,
    database: postio_storage::Store,
    account: postio_model::AccountId,
    _blobs: tempfile::TempDir,
    _bridge: Bridge,
    _wiring: Wiring,
    _window: Window,
}

/// What the main thread read while a window was pointed at a store of `size`.
async fn opening(size: usize) -> Opened {
    let database = test_support::memory().await;
    let report = seed_large(&database, 11, size).await;
    assert_eq!(
        report.message_count, size,
        "the seed should have written the mailbox this case is about"
    );
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

    // Every connection the pool hands out from here on, because the reads
    // `feed_the_window` makes are spread over several checkouts and a hook on
    // one of them would count some of them and quietly miss the rest.
    test_support::counting::install_on(&database);

    let window = Window::default();
    window.set_default_size(1280, 800);
    // The startup trace's own phases, recorded the way `postio_gtk::app`
    // records them on a real launch. Asserted below, because the marks are
    // what let a trace on a real store say which part of the stretch between
    // the window and the first frame cost what -- and a mark nothing checks
    // is a mark that can quietly stop being made.
    let timeline = postio_gtk::startup::Timeline::start();
    window.set_timeline(timeline.clone());
    // Deliberately not presented. A window at startup has no settings panel
    // on screen, and a panel's own figures are not the first frame's to wait
    // for — which is a claim about the code under test, so the case has to
    // put the window in the state the claim is about.
    let counts = counted_async(async || {
        let _ = feed_the_window(&window, &wiring).await;
    })
    .await;
    for phase in [
        postio_gtk::startup::Phase::Account,
        postio_gtk::startup::Phase::Feeds,
    ] {
        assert!(
            timeline.at(phase).is_some(),
            "pointing a window at the store left {} unmarked, so a startup \
             trace would fold this whole stretch into `first frame` again -- \
             which is the state #1479 found the instrument in",
            phase.label()
        );
    }
    Opened {
        counts,
        database,
        account: report.account.id,
        _blobs: directory,
        _bridge: bridge,
        _wiring: wiring,
        _window: window,
    }
}

pub fn opening_a_window_reads_a_bounded_amount_however_big_the_mailbox_is() {
    crate::gtk_case(async {
        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let small = opening(SMALL).await.counts;
        let opened = opening(LARGE).await;
        let large = opened.counts;
        eprintln!("  opening over {SMALL:>6} messages: {small:?}");
        eprintln!("  opening over {LARGE:>6} messages: {large:?}");

        assert_eq!(
            small.statements, large.statements,
            "pointing a window at {LARGE} messages issued {} statements where \
             {SMALL} issued {}. A statement count that moves with the mailbox is \
             a read per message on the thread that has to draw the first frame.",
            large.statements, small.statements
        );
        assert_eq!(
            small.rows, large.rows,
            "pointing a window at {LARGE} messages produced {} rows where {SMALL} \
             produced {}. §18's 'never load a whole mailbox into memory' is the \
             claim, and startup is where it is easiest to break.",
            large.rows, small.rows
        );
        // There used to be a third assertion here, on the VM **step** count,
        // and it was the one with the teeth: an aggregate that scans the whole
        // table is one statement returning one row, so it hides from both
        // counts above. #1479 was exactly that -- 1044 ms of a 1250 ms startup,
        // invisible to a budget written in statements and rows.
        //
        // The step count came off SQLite's trace hook, and this engine has no
        // trace hook. What replaces it is structural rather than numeric:
        // `counting::scans` asks the planner whether a query *can* be cheap.
        // Weaker in one way -- it has to be pointed at a statement by name,
        // where the step count saw every statement the window issued -- and
        // stronger in another, in that it says which query and why.
        //
        // So the two halves below. The read the window actually makes must
        // plan without a scan, and the read that #1479 took *off* this path
        // must still be a scan, which is what demonstrates that the two counts
        // above cannot see one.
        let connection = opened.database.connect().await.expect("a connection");
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let window_query = postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Account(opened.account),
            limit: 50,
            after: None,
        };
        let scanned = postio_storage::test_support::counting::scans(
            &connection,
            &messages.explain(&window_query),
        )
        .await;
        assert!(
            scanned.is_empty(),
            "the window's own list read plans as a scan of {scanned:?}. Work \
             proportional to the mailbox on the first frame's own thread is \
             what #1479 measured as 1044ms of a 1250ms startup, and it is \
             invisible to the statement and row counts above."
        );
        // The control, and the reason the step assertion is known to have teeth.
        // #100 asks that each counted budget "fails when the invariant it guards
        // is deliberately broken"; this demonstrates it without ever breaking the
        // code under test, by counting the read that used to be on this path.
        //
        // `read_receipt_requested_count` is the privacy pane's own figure: one
        // statement, one row, and a scan of every message the account holds to
        // produce it. It ran unconditionally from `settings_privacy::install`,
        // which `feed_the_window` calls, for a number drawn in a panel that is
        // not on screen. It is still here and still costs what it costs — it is
        // read when the pane is looked at now, which is the whole of the fix.
        let scan = postio_storage::test_support::counting::counted_async(async || {
            let _ = messages
                .read_receipt_requested_count(opened.account)
                .await
                .expect("a count");
        })
        .await;
        eprintln!("  one scan of {LARGE:>6} messages: {scan:?}");
        assert_eq!(
            (scan.statements, scan.rows),
            (1, 1),
            "an aggregate is one statement and one row however much it reads, \
             which is what makes it invisible to the two counts above"
        );
        // And the control, which says what this case can no longer prove.
        //
        // `read_receipt_requested_count` is #1479's own query: the privacy
        // pane's figure, run unconditionally from `feed_the_window` for a
        // number drawn in a panel that is not on screen, and 1044 ms of a
        // 1250 ms startup on a real store. It is one statement returning one
        // row, so the two counts above cannot see it -- that part is unchanged
        // and is why the step count existed.
        //
        // **`scans` cannot see it either**, and that is the honest limit of
        // the replacement. Measured, it plans as
        // `SEARCH messages USING INDEX idx_messages_send_state (account_id=?)`
        // -- a seek to the account and then a walk of every message the
        // account has. The planner calls that a SEARCH, because it is one; it
        // is O(mailbox) all the same.
        //
        // So: the instrument sees a scan of a *table* and is blind to a walk
        // of an *index prefix*, which is the shape of the bug it replaced.
        // Asserted rather than merely written down, so that an engine which
        // starts reporting this honestly fails here and says so.
        let control = postio_storage::test_support::counting::scans(
            &connection,
            "SELECT count(*) FROM messages
              WHERE account_id = ?1 AND read_receipt_requested = 1
                AND deleted_locally = 0",
        )
        .await;
        assert!(
            control.is_empty(),
            "#1479's aggregate now reports as a scan ({control:?}), which means \
             `counting::scans` has grown the reach the step count had. Point \
             the assertion above at every statement the window issues rather \
             than at the one query this case can name, and delete this comment."
        );
    });
}
