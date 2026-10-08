//! Opening a window reads a bounded amount, whatever the mailbox holds
//! (#1479).
//!
//! `docs/PRODUCT.md` §18 budgets startup at 500 ms. The signature of a miss
//! is that the first frame waits on work proportional to what the mailbox
//! holds. This is counted rather than timed, for the reason
//! [`postio_storage::test_support::counting`] gives: a count is the same
//! number on this workstation and on a loaded runner.
//!
//! > Opening a window reads a bounded amount, whatever the mailbox holds.
//!
//! Two stores an order of magnitude apart, the same counts from both.
//!
//! The counters are thread-local, which is exactly right: `startup::adopt`
//! runs on the thread that has to draw the first frame, so every read it
//! makes *synchronously* is a read the frame is waiting on, while what it
//! hands to the host's runtime -- the page fetch, the body-index catch-up --
//! is off this thread by construction.

use gtk::prelude::*;
use postio_storage::test_support::counting::{Counts, checkouts, counted};

use crate::support::{self, Fixture};

/// The two mailboxes, an order of magnitude apart: far enough that anything
/// linear in the mailbox shows as a factor of ten rather than as noise.
const SMALL: usize = 1_000;
const LARGE: usize = 10_000;

/// A window pointed at a store of `size`, and what that cost the main
/// thread. Everything built is kept: the host's runtime is still draining
/// what `adopt` spawned, and dropping it would be tearing the application
/// down mid-startup rather than measuring one.
struct Opened {
    counts: Counts,
    connections: u64,
}

async fn opening(size: usize) -> Opened {
    let fixture = Fixture::large(size).await;
    let host = fixture.host();
    // Deliberately not presented: a window at startup has no settings panel
    // on screen, and a panel's own figures are not the first frame's to wait
    // for.
    let window = postio_gtk::window::FocusWindow::new(None);
    window.set_default_size(1280, 800);
    let timeline = postio_widgets::startup::Timeline::start();
    postio_gtk::startup::time(&window, timeline.clone());

    let before = checkouts();
    let mut session = None;
    let counts = counted(|| {
        session = Some(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
    });
    let connections = checkouts() - before;
    for phase in [
        postio_widgets::startup::Phase::Account,
        postio_widgets::startup::Phase::Feeds,
    ] {
        assert!(
            timeline.at(phase).is_some(),
            "pointing a window at the store left {} unmarked, so a startup \
             trace would fold this whole stretch into `first frame`",
            phase.label()
        );
    }
    support::keep(session.expect("a session"));
    support::keep(window);
    support::keep(fixture);
    Opened {
        counts,
        connections,
    }
}

pub fn opening_a_window_reads_a_bounded_amount_however_big_the_mailbox_is() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let small = opening(SMALL).await;
        let large = opening(LARGE).await;
        eprintln!("  opening over {SMALL:>6} messages: {:?}", small.counts);
        eprintln!("  opening over {LARGE:>6} messages: {:?}", large.counts);
        eprintln!(
            "  connections opened: {} over {SMALL}, {} over {LARGE}",
            small.connections, large.connections
        );
        // What is left is the floor: the host's warm readers, opened once on
        // first use, and a write's own connection -- the same number
        // whatever the mailbox holds.
        assert_eq!(
            small.connections, large.connections,
            "the connections opened move with the mailbox"
        );
        assert!(
            large.connections <= 5,
            "pointing a window at the store opened {} store connections; the \
             warm readers answer these reads without one",
            large.connections
        );
        assert_eq!(
            small.counts.statements, large.counts.statements,
            "pointing a window at {LARGE} messages issued {} statements where \
             {SMALL} issued {}. A statement count that moves with the mailbox is \
             a read per message on the thread that has to draw the first frame.",
            large.counts.statements, small.counts.statements
        );
        assert_eq!(
            small.counts.rows, large.counts.rows,
            "pointing a window at {LARGE} messages produced {} rows where {SMALL} \
             produced {}. §18's 'never load a whole mailbox into memory' is the \
             claim, and startup is where it is easiest to break.",
            large.counts.rows, small.counts.rows
        );
    });
}
