//! A send nobody could confirm is visible, and can be settled (#674).
//!
//! ADR 0021 Decision 3 turns "we cannot know" into something a person can
//! act on, and the acting is the part a unit test cannot reach. The sync
//! layer's own tests prove the outcome and the state; `command_registry.rs`
//! proves the verb is in the vocabulary. Neither can answer the question
//! `/issue` insists on before anything closes: **can a person reach it?**
//!
//! So this drives the running application. A draft left `Unconfirmed` by an
//! interrupted submission is drawn in the Drafts folder like any other row,
//! and `Mark as sent` — invoked the way the palette invokes it, through
//! `Window::act` — settles it in SQLite. Not a toast: a toast is not a place
//! a message can be found again ten minutes later.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, actions, commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_core::{Command, CommandId};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Draft, DraftState, EmailAddress, MailboxRole};
use postio_storage::repository::DraftRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const SUBJECT: &str = "Tide gate interlock";



pub fn an_unconfirmed_send_is_listed_and_can_be_marked_as_sent() {
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
    let report = seed_small(&database, 9);
    let account = report.account.id;
    let drafts_folder = report
        .mailbox(MailboxRole::Drafts)
        .expect("the fixture has a Drafts folder")
        .clone();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // Exactly what an interrupted submission leaves behind: the draft is
    // still here, it is not `Failed`, and nothing is going to retry it.
    let draft_id = {
        let connection = database.connection().expect("a connection");
        let drafts = DraftRepository::new(&connection);
        let mut draft = Draft::new(account);
        draft.subject = SUBJECT.to_owned();
        draft.to = vec![EmailAddress::new(None::<String>, "quinn@example.net")];
        draft.body.text = Some("It may have gone.".to_owned());
        let id = drafts.save(&mut draft).expect("save the draft");
        drafts
            .set_state(id, DraftState::Unconfirmed)
            .expect("the state the send path leaves");
        id
    };

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    assert!(
        wired.contains(&CommandId::MarkSent),
        "the verb has to be on the bus, or the invocation below proves \
         nothing about the running application"
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
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);

    // ── it is in the Drafts folder, like any other draft ─────────────────
    // The point of a durable state rather than a toast: the user comes back
    // to this later and it is still there to be found.
    feeds
        .messages
        .open(postio_model::ListScope::Mailbox(drafts_folder.id));
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "an unconfirmed send is drawn in Drafts like any other draft; this \
         folder came up empty"
    );
    let row = settle_until(|| {
        (0..list.model().n_items()).any(|position| {
            list.model()
                .peek(position)
                .and_then(|id| {
                    let connection = database.connection().ok()?;
                    DraftRepository::new(&connection).by_message(id).ok()?
                })
                .is_some_and(|draft| draft.id == draft_id)
        })
    });
    assert!(
        row,
        "the unconfirmed draft has no row, so there is nothing for a person \
         to find or act on"
    );

    // ── and `Mark as sent` settles it ────────────────────────────────────
    // Through `Window::act`, which is what the palette calls: a test that
    // called the handler directly would pass in a build where the command
    // reached nothing, which is exactly the failure #767 was.
    window.act(Command::MarkSent {
        draft: Some(draft_id),
    });

    let settled = settle_until(|| {
        let connection = database.connection().expect("a connection");
        DraftRepository::new(&connection)
            .get(draft_id)
            .expect("read")
            .is_some_and(|draft| draft.state == DraftState::Sent)
    });
    assert!(
        settled,
        "the user said the message arrived and Postio did not record it -- \
         which leaves them the two exits #674 exists to replace: discard it, \
         or send it a second time"
    );

    bridge.shutdown();
}
