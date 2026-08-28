//! Search against a real, synced store.
//!
//! `postio-1ag` is the ninth instance of a surface that was built, tested and
//! never fed, so a seeded fixture is not enough to close it: every one of the
//! eight before it passed its own tests. `search_wiring.rs` drives the same
//! code over `seed_small`, which is a store this repository builds; this
//! drives it over a store a *server* built, with the index that
//! `open_store` created on a real account.
//!
//! What that catches and the seeded test cannot: an FTS schema that was never
//! created on a real database (`postio-x4e`), a query that is fast on thirty
//! eight messages and not on eighty thousand, and a store whose mail arrived
//! through sync rather than through `MessageRepository::create`.
//!
//! # Running it
//!
//! Ignored by default, like every other test here that needs something this
//! repository cannot build:
//!
//! ```console
//! POSTIO_TEST_STORE=~/scratch/postio-run/state/data/postio/postio.db \
//!   cargo test -p postio-app --test search_live -- --ignored
//! ```
//!
//! Point it at a scratch store, not at your real one. It opens the database
//! read-write, because `Database::open` migrates and `ensure_search_index`
//! creates the FTS schema — that is the whole point, since those are the two
//! things `open_store` does that a fixture does not.
//!
//! # It never prints mail
//!
//! Counts, timings and ids only. This runs against somebody's actual
//! correspondence and a failure message is the last place a subject line
//! should turn up (CLAUDE.md: logs never carry message content). Every
//! assertion here is a number.
//!
//! Nothing here touches the network.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};

/// Where the store is. Absent means "skip", not "fail".
const STORE: &str = "POSTIO_TEST_STORE";

/// What to search for. A word common enough to hit in most mailboxes, and
/// overridable because "most" is not "every".
const QUERY_VAR: &str = "POSTIO_TEST_QUERY";
const QUERY_DEFAULT: &str = "invoice";

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

#[test]
#[ignore = "needs a real synced store; set POSTIO_TEST_STORE"]
fn a_real_account_answers_a_real_query() {
    let Ok(path) = std::env::var(STORE) else {
        eprintln!("skipping: set {STORE} to a synced store to exercise this");
        return;
    };
    let query = std::env::var(QUERY_VAR).unwrap_or_else(|_| QUERY_DEFAULT.to_owned());

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // Opened exactly the way `open_store` opens it, including the index and
    // the key: if the FTS schema is missing on a real database, that is the
    // bug, and creating it here is what proves the application creates it.
    //
    // The key comes from the real keyring, because a real store is encrypted
    // under a real key (ADR 0014) and there is no other way in. A locked
    // keyring is a skip and not a failure — this test is about search.
    let secrets = postio_imap::secret::KeyringSecretStore::default();
    let Ok(store_key) = postio_session::store_key_blocking(&secrets) else {
        eprintln!("skipping: the store key is not readable (is the keyring unlocked?)");
        return;
    };
    let (database, blobs) =
        postio_session::open_store_at(&path, &store_key).expect("the store opens");
    ensure_search_index(&database).expect("the index is part of opening the store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // Through the composition root, so this drives one install rather than a
    // second `View` racing the one `feed_the_window` already made.
    let view = postio_app::feed_the_window(&window, &wiring)
        .expect("the store has an account")
        .search
        .expect("search installed");

    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: query.clone(),
    });
    let live = finder.live().expect("the box has a live readout");
    live.flush();

    assert!(
        settle_until(|| live.outcome().is_some()),
        "searching a real store never produced a readout"
    );
    let outcome = live.outcome().expect("answered");
    // The only thing printed about the mail is how much of it matched.
    eprintln!(
        "live search: {} hits in {} ms (capped: {})",
        outcome.hits,
        outcome.elapsed.as_millis(),
        outcome.capped
    );

    assert!(
        outcome.hits > 0,
        "`{query}` found nothing in a real account. Either the index was \
         never created on this store, or the executor is not reading it — \
         override {QUERY_VAR} if the word is genuinely absent."
    );
    // CLAUDE.md's local-search budget, against a real mailbox rather than a
    // fixture — the only size that settles it.
    //
    // Only in release. `cargo test` is a debug build with no optimisation,
    // and this same query measured 164 ms there against 100 ms; asserting
    // that number would be reporting the profile, not the query. Run it as
    // `cargo test --release -p postio-app --test search_live -- --ignored`.
    #[cfg(not(debug_assertions))]
    assert!(
        outcome.elapsed.as_millis() < 100,
        "the search took {} ms against the <100 ms budget, on a real mailbox",
        outcome.elapsed.as_millis()
    );
    // In either profile: a search that takes this long is not slow, it is
    // broken, and saying so is worth more than saying nothing.
    assert!(
        outcome.elapsed.as_millis() < 2_000,
        "the search took {} ms, which is not a budget problem but a bug",
        outcome.elapsed.as_millis()
    );
    assert!(
        settle_until(|| !view.panel().offered().is_empty()),
        "the refine column offered nothing for {} hits",
        outcome.hits
    );
    assert!(
        settle_until(|| view.preview().focused().is_some()),
        "the preview showed none of the {} hits",
        outcome.hits
    );

    finder.set_query(Query {
        mode: Mode::Contact,
        text: String::new(),
    });
    assert!(
        settle_until(|| !finder.matched_contacts().is_empty()),
        "`@` listed no correspondents for an account that has synced mail"
    );
    eprintln!(
        "live `@`: {} correspondents",
        finder.matched_contacts().len()
    );

    bridge.shutdown();
}
