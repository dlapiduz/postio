//! `AddLabel` reaches a message, through the application's own bus (#780).
//!
//! `AddLabel` was a registered command that answered to nothing, and #766
//! removed it rather than leave a menu item that could never do anything —
//! the `postio-bl2` shape this crate's wiring tests exist for. #780 built the
//! support and brought the command back, so this is what says it is real: a
//! command invoked the way the menu item and `L` invoke it, and a message
//! that afterwards actually carries the label.
//!
//! Two halves, because the command has two shapes. With a label it writes;
//! with `None` it asks, and "asks" means the picker opens rather than the
//! dispatcher rejecting it — which is exactly the dead end #766 removed.
//!
//! # Why this shares `app_suite`'s process (#973)
//!
//! None of the reasons to stay out applies: not in the watchdog's name list
//! (#272), no display of its own (#45/#114) beyond the one the suite already
//! has, and no wall-clock budget (#841). It sets its own `XDG_STATE_HOME`,
//! which is safe for the reason its neighbours rely on — one case at a time,
//! on one thread, and `postio_config::paths` caches nothing.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This runs before the app under test starts.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wired, Wiring, commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Label, LabelId, MessageId};
use postio_storage::repository::{LabelRepository, ListQuery, ListScope, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, Database, test_support};

/// The labels on `message`, straight from the store.
fn labels_of(database: &Database, message: MessageId) -> Vec<LabelId> {
    let connection = database.connection().expect("a connection");
    LabelRepository::new(&connection)
        .for_message(message)
        .expect("a read")
}

pub fn a_label_command_puts_a_label_on_the_message_it_names() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded case.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A label to reach for, and a message to put it on.
    let (work, first) = {
        let connection = database.connection().expect("a connection");
        let mut work = Label::new(report.account.id, "Work");
        LabelRepository::new(&connection)
            .create(&mut work)
            .expect("create a label");
        let inbox = report
            .mailbox(postio_model::MailboxRole::Inbox)
            .expect("an inbox");
        let first = MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: 1,
                after: None,
            })
            .expect("a page")
            .first()
            .expect("the fixture seeded a message")
            .id;
        (work.id, first)
    };

    // A real bus, not a no-op handler: the question is whether the command
    // reaches a verb that writes to SQLite, and a bus that swallowed it would
    // make this pass while proving nothing.
    let state = SharedState::default();
    let bus = postio_app::actions::wire(
        Dispatcher::builder(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
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
    while glib::MainContext::default().iteration(false) {}

    let Wired { feeds, .. } =
        feed_the_window(&window, &wiring).expect("the seeded store has an account");
    commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        bus_verbs.clone(),
    );
    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "the list is empty, so there is nothing to label"
    );

    assert!(
        labels_of(&database, first).is_empty(),
        "the message is labelled before anything did it, so the assertion \
         below would pass without the command working"
    );

    // ── the answered command writes ──────────────────────────────────────
    window.act(postio_core::Command::AddLabel {
        target: postio_core::MessageTarget::Messages(vec![first]),
        label: Some(work),
        on: None,
    });
    assert!(
        settle_until(|| labels_of(&database, first) == vec![work]),
        "the command never reached a verb that writes: the message carries \
         {:?}",
        labels_of(&database, first)
    );

    // ── and `u` takes it back, which the registry promises ───────────────
    window.act(postio_core::Command::Undo);
    assert!(
        settle_until(|| labels_of(&database, first).is_empty()),
        "`u` left the label on, and AddLabel is registered Recovery::Undo"
    );
}

pub fn a_label_command_with_no_label_opens_the_picker() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded case.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // `None` is half a request. The window asks, rather than passing it to a
    // dispatcher that would answer "Pick a label to add" — which is what the
    // row's context menu and `L` would otherwise have done, and is exactly
    // the dead end #766 removed the command for being.
    window.act(postio_core::Command::AddLabel {
        target: postio_core::MessageTarget::Selection,
        label: None,
        on: None,
    });
    while glib::MainContext::default().iteration(false) {}

    assert!(window.finder().is_open(), "the picker did not open");
    assert_eq!(
        window.finder().mode(),
        Mode::Label,
        "the box opened on the wrong mode, so `+`'s labels are not what it \
         is offering"
    );
}
