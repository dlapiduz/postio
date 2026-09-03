//! `Ctrl+A` in a degraded unified view, all the way to SQLite (#811).
//!
//! ADR 0005 Q10's fourth bullet: *a whole-view selection in a degraded
//! aggregate must not act on an account Postio cannot currently see.* Until
//! this landed it did not act on the wrong account — it did not act at all,
//! because `AppState::open_unified` left no `ViewScope` behind and every bulk
//! verb rejected with "Nothing selected", a refusal the user had not earned.
//!
//! Every layer of the fix has its own test and none of them can see this one.
//! `postio-core` proves the scope carries its accounts, `postio-storage`
//! proves the predicate names only them, `postio-session` proves the verb
//! splits per account. What has no test without this one is whether the
//! *frontend* fills the scope at all: the list holds the reachable set, the
//! selection freezes it, and the adapter hands it over. A widget that froze
//! nothing would pass every one of those layers and reject the keystroke,
//! which is precisely the bug (`postio-bl2`, #596).
//!
//! So the assertions are two `SELECT`s: the reachable account's mail moved,
//! and the unreachable account's mail did not.
//!
//! Nothing here touches the network: `start_syncing` is never called, the
//! connection states are delivered by hand the way the runtime delivers them,
//! and the queued operations simply wait.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_core::{CommandId, ConnectionState, Event};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::{ListScope, MailboxRole};
use postio_session::{Wiring, actions};
use postio_storage::repository::{MessageRepository, MessageSet};
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, Database, test_support};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// How many of `account`'s messages are still outside `archive`.
///
/// Counted rather than listed: the assertion is about a whole account's mail,
/// which is what the predicate under test is about too.
fn still_outside(database: &Database, account: AccountId, archive: MailboxId) -> u32 {
    let connection = database.connection().expect("a connection");
    let repository = MessageRepository::new(&connection);
    let all = repository
        .count_set(&MessageSet::InAccounts {
            accounts: vec![account],
            except: Vec::new(),
        })
        .expect("a count");
    let filed = repository
        .count_set(&MessageSet::in_mailbox(archive))
        .expect("a count");
    all - filed
}

pub fn select_all_in_a_degraded_unified_view_archives_only_what_it_could_see() {
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
    let here = seed_small(&database, 11);
    let away = seed_extra_account(&database, "Second", "grace@example.org", 12);
    let here_archive = here
        .mailbox(MailboxRole::Archive)
        .expect("the fixture has an archive folder")
        .id;
    let away_archive = away
        .mailbox(MailboxRole::Archive)
        .expect("the second account has an archive folder")
        .id;

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // The real bus over the real store, composed the way `run` composes it.
    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    assert!(
        wired.contains(&CommandId::Archive),
        "the bus does not answer archive, so this test cannot mean anything"
    );

    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
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
    settle();

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the window drew no mail at all, so nothing below can be concluded"
    );

    // ── into the unified view, with one account away ────────────────────
    window.sidebar().test_click_account_row(0);
    assert!(
        settle_until(|| feeds.messages.scope() == Some(ListScope::Unified)),
        "the unified scope was never reached, so this is #185's wiring \
         failing rather than anything about degradation"
    );
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the unified scope was reached and drew nothing at all"
    );

    feeds.apply(&Event::ConnectionChanged {
        account: here.account.id,
        state: ConnectionState::Online,
    });
    feeds.apply(&Event::ConnectionChanged {
        account: away.account.id,
        state: ConnectionState::Offline,
    });
    settle();
    // The list's *live* reading, before any gesture. Localised deliberately:
    // if this is empty the fault is in what the window tells the list, and
    // everything below would fail for a reason that has nothing to do with
    // selections.
    assert!(
        settle_until(|| {
            let reach = list.reach();
            reach.accounts == vec![here.account.id] && reach.omitted == vec!["Second".to_owned()]
        }),
        "the list never learned which accounts it could vouch for, so the \
         gesture below has nothing to freeze: {:?}",
        list.reach()
    );

    // ── the gesture, and the verb ───────────────────────────────────────
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    settle();

    // The gesture froze it. `SelectionState::reach` answers only while the
    // selection is a predicate, so this is also the proof that `Ctrl+A`
    // reached the selection at all.
    assert_eq!(
        list.selection().reach().accounts,
        vec![here.account.id],
        "`Ctrl+A` did not freeze the accounts the view could show"
    );

    // What a person would actually see: the header does not claim a count it
    // cannot support, and it names the account it left out in the same words
    // the banner over the rows uses.
    assert_eq!(
        window.list().selection_summary().as_deref(),
        Some("All selected, except Second"),
        "a count that silently excludes an account is the same lie as no \
         banner at all, in a smaller place"
    );

    let before_away = still_outside(&database, away.account.id, away_archive);
    assert!(
        before_away > 0,
        "the second account's mail is already all filed, so leaving it alone \
         would prove nothing"
    );

    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );

    // The bus runs on the runtime's threads, so the write lands a moment
    // after the key press.
    let archived = settle_until(|| still_outside(&database, here.account.id, here_archive) == 0);
    assert!(
        archived,
        "`Ctrl+A` then `a` in the unified view archived nothing. Every layer \
         under this has passing tests; what has no test without this one is \
         whether the frontend fills the scope at all. {} of the reachable \
         account's messages are still outside its archive.",
        still_outside(&database, here.account.id, here_archive)
    );
    assert_eq!(
        still_outside(&database, away.account.id, away_archive),
        before_away,
        "an account the view could not vouch for was archived anyway. The \
         selection was scoped to what was visible when `Ctrl+A` was pressed, \
         and this account was not in it (ADR 0005 Q10)."
    );

    bridge.shutdown();
}
