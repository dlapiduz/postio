//! The sidebar's Unified root actually draws mail (#185, ADR 0005 Q4).
//!
//! This is the wiring half of #185, and the reason the rest of it sat
//! unfinished: `Sidebar::set_accounts` takes `offer_unified` because a row
//! that selects a scope nothing can draw is a dead end, and until
//! `ListScope::Unified` existed nothing could draw one. So the assertion
//! here is deliberately not "the strip has three rows" — `gtk_sidebar_accounts.rs`
//! covers that, and it passed the whole time the row led nowhere. It is
//! *clicking Unified fills the list from more than one account's folders*,
//! which is the claim that fails when the scope is unreachable, when the
//! store refuses it, or when the click never reaches the feed.
//!
//! The second account is seeded from the same corpus into its own folder
//! tree, so the two accounts hold overlapping conversations — which is what
//! makes the unified list's cross-account grouping run rather than being a
//! concatenation that happens to look right.
//!
//! Nothing here touches the network: `feed_the_window` reads the local
//! store, and `start_syncing` — the half that opens a socket — is never
//! called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ListScope;
use postio_session::Wiring;
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, test_support};



pub fn picking_unified_lists_mail_from_every_account() {
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

    // ── two accounts, each with its own folder tree and mail ────────────
    let database = test_support::memory();
    let first = seed_small(&database, 11);
    let second = seed_extra_account(&database, "Second", "grace@example.org", 12);
    assert!(
        first.message_count > 0 && second.message_count > 0,
        "both fixtures have to hold mail or this test cannot fail"
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;

    // ── it opens on one account's inbox, as it always has ───────────────
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the window opened on an inbox and drew nothing, so nothing below \
         can be concluded about the unified view"
    );
    let inbox_rows = list.model().n_items();
    assert!(matches!(
        feeds.messages.scope(),
        Some(ListScope::Mailbox(_))
    ));

    // The window has to know it too, not just the feed. `Requirement::
    // SingleAccount` is evaluated against `Window::scope`, so this is what
    // decides whether "Move to…" is in the palette and the cheat sheet -- and
    // `AccountScope`'s own default is Unified, which means a window nobody
    // tells is a window that hides Move from everybody, single account or
    // not.
    assert!(
        window.scope().is_single_account(),
        "opened on one account's inbox and the window still reports {:?}",
        window.scope()
    );

    // ── the strip offers Unified, and picking it is what a click does ───
    let sidebar = window.sidebar();
    assert_eq!(
        sidebar.account_rows().first().map(String::as_str),
        Some("Unified"),
        "with two accounts the strip's first row is the unified root -- if \
         this is absent, `offer_unified` is still false"
    );
    sidebar.test_click_account_row(0);

    let switched = settle_until(|| feeds.messages.scope() == Some(ListScope::Unified));
    assert!(
        switched,
        "clicking Unified left the list on {:?}: the strip reported the \
         scope and nothing re-pointed the feed at it",
        feeds.messages.scope()
    );

    // ── and it drew mail, more of it than the one inbox held ────────────
    //
    // The count, not merely non-emptiness: a unified list that answered with
    // the same rows the inbox already had would pass an "is it empty" check
    // while showing one account's mail under a label promising every
    // account's. Both accounts' whole folder trees are in scope here, so it
    // has to be strictly more than a single inbox.
    let filled = settle_until(|| list.model().n_items() > inbox_rows);
    assert!(
        filled,
        "unified shows {} rows and one account's inbox showed {inbox_rows}: \
         every folder in both accounts is in scope, so this is the list \
         still drawing the folder it was on",
        list.model().n_items()
    );
    assert!(
        list.model().peek(0).is_some(),
        "the unified list reports rows and cannot name the first one"
    );

    // And the window followed, so the commands that need somewhere in *one*
    // account to put a message stop being offered (ADR 0005 Q4).
    assert!(
        !window.scope().is_single_account(),
        "the list is unified and the window still reports {:?}, so the \
         palette would go on offering a Move with no account to move within",
        window.scope()
    );

    bridge.shutdown();
}
