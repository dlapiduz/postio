//! After a join, mail from either of the person's addresses carries the name
//! the user chose -- in the list and in the reader -- and the reader still
//! shows what the mail itself said (specs/005-contacts FR-032, T046).
//!
//! Read from what is drawn: the list rows' spoken labels, the reader header's
//! sender line and its "as sent" text. A test that asked the store would pass
//! with the name never reaching the screen, which is the between-layers
//! defect this project keeps finding.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use crate::{settle, settle_until};
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_core::{Command, ContactJoinAction};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{EmailAddress, Message};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::repository::{AccountRepository, ContactRepository, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const NAME: &str = "Countess of Lovelace";

/// Every row's spoken label, which says what the row draws.
fn spoken(window: &Window) -> Vec<String> {
    let model = window.list().model();
    (0..model.n_items())
        .filter_map(|position| {
            model
                .item(position)
                .and_downcast::<postio_gtk::list::MessageRow>()
                .and_then(|item| item.row())
                .map(|row| postio_gtk::row::accessible_label(&row))
        })
        .collect()
}

pub fn a_joined_persons_name_reaches_the_list_and_the_reader() {
    crate::gtk_case(async {
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

        // ── ada writes from two addresses, under two header names ──────────
        let database = test_support::memory().await;
        let report = seed_small(&database, 11).await;
        let inbox = report
            .mailbox(postio_model::MailboxRole::Inbox)
            .expect("the seed makes an inbox")
            .id;
        let (work, home) = {
            let connection = database.connect().await.expect("a connection");
            let account = AccountRepository::new(&connection)
                .list()
                .await
                .expect("accounts")
                .into_iter()
                .next()
                .expect("the seed makes an account");
            for (name, email, subject) in [
                ("A. L.", "ada@work.example", "Engines"),
                ("ada", "ada@home.example", "Notes on the engine"),
            ] {
                let mut message = Message::new(account.id, inbox, chrono::Utc::now());
                message.from = vec![EmailAddress::new(Some(name), email)];
                message.subject = Some(subject.into());
                MessageRepository::new(&connection)
                    .create(&mut message)
                    .await
                    .expect("ada's mail");
                ContactRepository::new(&connection)
                    .record_message(&message, std::slice::from_ref(&account.address))
                    .await
                    .expect("recorded as sync would");
            }
            let contacts = ContactRepository::new(&connection);
            let owner = async |email: &str| {
                contacts
                    .by_address(email)
                    .await
                    .expect("lookup")
                    .expect("recorded")
                    .id
            };
            (
                owner("ada@work.example").await,
                owner("ada@home.example").await,
            )
        };
        ensure_search_index(&database)
            .await
            .expect("the index is part of opening the store");

        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        // A real bus: the join has to reach the store and come back as an
        // event, or the list has nothing to repaint for.
        let state = postio_core::state::SharedState::default();
        let bus = postio_app::actions::wire(
            Dispatcher::builder(),
            postio_app::actions::Actions::new(database.clone(), state.clone()),
        )
        .build();
        let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
        let (bridge, replies) = Bridge::new(bus).expect("a runtime");
        let (sink, events) = event_channel();
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
        let feeds = wired.feeds.clone();
        let notifier = postio_app::notifications::Notifier::new(
            database.clone(),
            wiring.store.clone(),
            wiring.runtime.clone(),
            Default::default(),
        );
        postio_app::commands::install(
            &window,
            &feeds,
            state.clone(),
            wiring.commands.clone(),
            bus_verbs,
        );
        for stream in [events, replies] {
            postio_app::commands::drain(&window, &feeds, stream, notifier.clone(), state.clone());
        }
        assert!(
            settle_until(async || spoken(&window).iter().any(|l| l.contains("Engines"))).await,
            "ada's mail never listed"
        );
        assert!(
            !spoken(&window).iter().any(|l| l.contains(NAME)),
            "nobody has named ada yet, so no row can say so"
        );

        // ── the join, through the bus the keys use ─────────────────────────
        window.act(Command::ContactJoin(ContactJoinAction::Join {
            into: work,
            others: vec![home],
            name: NAME.into(),
            organization: None,
        }));
        let ada_rows = async || {
            spoken(&window)
                .iter()
                .filter(|label| label.contains("Engines") || label.contains("Notes on the"))
                .all(|label| label.contains(NAME))
        };
        assert!(
            settle_until(ada_rows).await,
            "after the join the list still reads {:?}",
            spoken(&window)
                .into_iter()
                .filter(|l| l.contains("ngine"))
                .collect::<Vec<_>>()
        );

        // ── the reader: the chosen name, and what the mail said ────────────
        let finder = window.finder();
        finder.open(Mode::Search);
        finder.set_query(Query {
            mode: Mode::Search,
            text: "with:ada@work.example".to_owned(),
        });
        if let Some(live) = finder.live() {
            live.flush();
        }
        assert!(
            settle_until(async || feeds.messages.showing_results()).await,
            "the search never showed its hit"
        );
        window.list().first_row();
        let header = window.reader().header();
        assert!(
            settle_until(async || header.sender_label().contains(NAME)).await,
            "the reader's sender line reads {:?}",
            header.sender_label()
        );
        assert_eq!(
            window.reader().sender_as_sent().as_deref(),
            Some("As sent: A. L. <ada@work.example>"),
            "the mail's own words stay one look away"
        );
        settle();
        bridge.shutdown();
    });
}
