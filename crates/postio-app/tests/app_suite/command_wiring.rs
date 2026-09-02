//! Every `CommandId` reaches somewhere (#756).
//!
//! `ToggleSidebar` was a registry command with no handler anywhere -- not an
//! arm in `Window::handled_here`, not a handler on the bus -- so the palette
//! and `Ctrl+B` both silently did nothing. It was invisible to every
//! existing test because `gtk_suite/gtk_finder.rs` only asserts a command
//! reaches `Window::connect_command`, which `ToggleSidebar` does; nothing
//! downstream of that spy was ever checked, which is exactly why an orphan
//! command reads as a passing test.
//!
//! This sweeps every id `CommandId::ALL` names through a bare `Window` wired
//! to the production bus (`postio_session::actions::wire` +
//! `postio_session::refresh::wire`, the same composition
//! `postio-app::lib::open_with` builds) and flags any id that escapes
//! `Window::handled_here` without the bus answering it either.
//!
//! # The bus is not the only other answer
//!
//! A first version of this test treated "escapes the window, and the bus
//! does not wire it" as the whole definition of orphaned, and it flagged
//! about a third of every command in the registry. Nearly all of those
//! turned out to be real, working commands answered by a `connect_command`
//! subscriber this test does not install: `Composer::dispatch`
//! (`crates/postio-gtk/src/composer.rs`), the saved-search and
//! `EditConfig` wiring in `crates/postio-gtk/src/config.rs`, the
//! `AddAccount` dialog in `crates/postio-app/src/add_account.rs`, and the
//! result-order toggle in `crates/postio-app/src/search.rs`. Standing those
//! subsystems up here just to watch them do nothing new would duplicate the
//! coverage `compose_typing.rs`, `send_wiring.rs`, `reply_source.rs`,
//! `settings_credential_wiring.rs`, `add_account_wiring.rs` and
//! `search_wiring.rs` already have, so those ids are named below instead,
//! each with where its real answer lives.
//!
//! Sweeping for real also turned up ids that answered to nothing at all --
//! `PrevView`, `NextScope`, `AddLabel`, and (needing runtime confirmation)
//! `OpenMessage`'s search-preview path. Those were #756's bug, not #756's
//! fix: filed as #765, #766 and #767. `PrevView` and `NextScope` are wired
//! now (`gtk_prev_view.rs`, `gtk_next_scope.rs` prove it for real, through
//! `Window::act`). `AddLabel` turned out to have no label support behind
//! it at all -- no repository to list or create one, no picker, nothing to
//! wire it *to* -- so #766 removed the command rather than offer a menu
//! item that could never do anything; #780 tracks building label support
//! for real. `OpenMessage` was the last one left: nothing answered it, so
//! the search preview's `Ret` sent a command the dispatcher rejected and
//! opened nothing. #767 wired it in `Window::act` and took it off the list,
//! which is how a session confirms a fix here -- `search_open.rs` drives the
//! gesture end to end.
//!
//! No real dispatcher subscriber besides the spy is ever installed
//! (`commands::install` is never called, and neither is
//! `postio_gtk::composer::install`): nothing in this sweep actually
//! archives, sends, or opens an external editor -- it only observes whether
//! a command would have escaped to one.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::state::SharedState;
use postio_core::{Command, CommandId};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::refresh::EngineSlot;
use postio_session::{actions, refresh};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// `Window::act` threads these through `follow_drill_in` rather than
/// `handled_here`: the local effect happens (closing a thread, drilling
/// into one), but the original command is still delivered afterwards for
/// whatever besides the window might care that it happened (see
/// `follow_drill_in`'s doc comment). So they escape by design on every
/// invocation, not because nothing answers them -- `Back` with nothing to
/// back out of, or `Thread` with no row under the cursor, is a no-op by
/// design.
const FOLLOW_DRILL_IN_OWNED: &[CommandId] = &[
    CommandId::Back,
    CommandId::Thread,
    // #765: the keyboard-only sibling of `Back`, same split, same reason.
    CommandId::PrevView,
];

/// Answered by `Composer::dispatch`, a `connect_command` subscriber this
/// sweep does not install (`postio_gtk::composer::install` is never
/// called). Covered by its own tests: `compose_typing.rs`,
/// `compose_detach.rs`, `send_wiring.rs`, `send_later_wiring.rs`,
/// `reply_source.rs`, `reply_identity.rs`, `resume_draft.rs`,
/// `resume_queued_draft.rs`.
const COMPOSER_OWNED: &[CommandId] = &[
    CommandId::Compose,
    CommandId::Send,
    CommandId::ScheduleSend,
    CommandId::SaveDraft,
    CommandId::DiscardDraft,
    CommandId::AttachFile,
    CommandId::DetachComposer,
    CommandId::Reply,
    CommandId::ReplyAll,
    CommandId::Forward,
    CommandId::Bold,
    CommandId::Italic,
    CommandId::BulletList,
    CommandId::NumberedList,
    CommandId::QuoteBlock,
    CommandId::InsertLink,
];

/// Answered by `crates/postio-gtk/src/config.rs`'s saved-search and
/// `EditConfig` wiring, and `crates/postio-app/src/add_account.rs`'s
/// `AddAccount` dialog wiring -- both `connect_command` subscribers this
/// sweep does not install. Covered by `settings_accounts_wiring.rs`,
/// `settings_credential_wiring.rs`, `add_account_wiring.rs`,
/// `sidebar_backfill_wiring.rs`.
const CONFIG_AND_ACCOUNT_OWNED: &[CommandId] = &[
    CommandId::AddAccount,
    CommandId::EditConfig,
    CommandId::SaveSearch,
    CommandId::RenameSavedSearch,
    CommandId::MoveSavedSearchUp,
    CommandId::MoveSavedSearchDown,
    CommandId::DeleteSavedSearch,
];

/// Answered by `crates/postio-app/src/search.rs`'s own `connect_command`
/// subscriber, active only while a search's results are showing. Covered by
/// `search_wiring.rs`.
const SEARCH_OWNED: &[CommandId] = &[CommandId::ToggleResultOrder];

/// Genuinely orphaned -- found by this sweep, not #756's to fix. Remove an
/// entry once its issue lands a real handler; the sweep will fail the same
/// way it did for `ToggleSidebar` if one is removed too early.
///
/// Empty, and worth keeping: every id this sweep found unanswered has since
/// been wired or removed. `OpenMessage` was the last of them (#767) and is
/// answered in `Window::act` now, proven end to end by `search_open.rs`.
const KNOWN_ORPHANS: &[(CommandId, &str)] = &[];

pub fn every_command_id_is_handled_locally_or_wired_to_the_bus() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    seed_small(&database, 11);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // Composed exactly as `open_with` composes it, so `wired` is the same
    // list the real application would check a command against.
    let state = SharedState::default();
    let builder = actions::wire(
        postio_core::dispatch::Dispatcher::builder(),
        actions::Actions::new(database.clone(), state.clone()),
    );
    let bus = refresh::wire(builder, EngineSlot::default(), state.clone()).build();
    let wired: Vec<CommandId> = bus.wired().collect();

    let (bridge, _replies) = postio_core::bridge::Bridge::new(bus).expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = postio_session::Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    feed_the_window(&window, &wiring).expect("the seeded store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "no rows to open a thread on"
    );

    // Opened directly, not through a command: `ToggleThreadUnread` and
    // `ToggleThreadOrder` are only ever handled locally while a thread is
    // open, and this sweep has to exercise that arm rather than the guard
    // that keeps it from firing outside a thread.
    let model = list.model();
    let mut thread_row = None;
    for index in 0..model.n_items() {
        if let Some(row) = model
            .item(index)
            .and_then(|object| object.downcast::<postio_gtk::list::MessageRow>().ok())
            .and_then(|item| item.row())
            && row.is_thread()
        {
            thread_row = Some(row);
            break;
        }
    }
    if let Some(row) = thread_row {
        window.open_thread(&row);
        while glib::MainContext::default().iteration(false) {}
        assert!(window.thread_open(), "opening a thread row should open one");
    } else {
        eprintln!(
            "note: the seeded folder has no thread row, so ToggleThreadUnread/\
             ToggleThreadOrder are exercised unopened this run"
        );
    }

    let escaped: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_command({
        let escaped = Rc::clone(&escaped);
        move |id| escaped.borrow_mut().push(id)
    });

    let known_orphans: Vec<CommandId> = KNOWN_ORPHANS.iter().map(|(id, _)| *id).collect();
    let mut orphans = Vec::new();
    for &id in CommandId::ALL {
        if FOLLOW_DRILL_IN_OWNED.contains(&id)
            || COMPOSER_OWNED.contains(&id)
            || CONFIG_AND_ACCOUNT_OWNED.contains(&id)
            || SEARCH_OWNED.contains(&id)
            || known_orphans.contains(&id)
        {
            continue;
        }
        escaped.borrow_mut().clear();
        window.act(Command::default_for(id));
        while glib::MainContext::default().iteration(false) {}
        if escaped.borrow().contains(&id) && !wired.contains(&id) {
            orphans.push(id);
        }
    }

    assert!(
        orphans.is_empty(),
        "these commands reach neither `Window::handled_here` nor the bus, so \
         invoking them does nothing: {orphans:?}. Give each one a \
         `handled_here` arm, wire it in postio_session::actions (or \
         refresh), or -- if it is answered by a subsystem this sweep does \
         not install, like the composer -- add it to the matching list \
         above with why."
    );

    // The allow-lists above are supposed to be honest about which commands
    // are still broken, not a place orphans go to be forgotten -- so a
    // resolved one should be caught leaving the list, the same way a new
    // one is caught arriving.
    for &(id, issue) in KNOWN_ORPHANS {
        escaped.borrow_mut().clear();
        window.act(Command::default_for(id));
        while glib::MainContext::default().iteration(false) {}
        let still_orphaned = escaped.borrow().contains(&id) && !wired.contains(&id);
        assert!(
            still_orphaned,
            "{id} is in KNOWN_ORPHANS citing {issue}, but it is answered \
             now -- remove it from the list so this test keeps meaning \
             something"
        );
    }
}
