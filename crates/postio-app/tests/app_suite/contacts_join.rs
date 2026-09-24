//! Joining two people from the keyboard, and taking it back
//! (specs/005-contacts User Story 2, SC-004).
//!
//! `g c`, `x`, down, `x`, `m`, `Return`: one row where there were two, one
//! person completion offers with both addresses, preferred first -- and `u`
//! puts both back. Against a real bus, because the question is whether the
//! keys reach the store and the store's answer reaches the screen.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use crate::{settle, settle_until};
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{EmailAddress, Message};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::repository::{
    AccountRepository, ContactRepository, MailboxRepository, MessageRepository,
};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

fn drawn(window: &Window) -> Vec<String> {
    let model = window.contacts().model();
    (0..model.n_items())
        .filter_map(|position| model.row(position).map(|row| row.name))
        .collect()
}

pub fn x_x_m_return_joins_and_u_takes_it_back() {
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

        // ── one person the user made, one they wrote to from mail ──────────
        let database = test_support::memory().await;
        seed_small(&database, 11).await;
        {
            let connection = database.connect().await.expect("a connection");
            let account = AccountRepository::new(&connection)
                .list()
                .await
                .expect("accounts")
                .into_iter()
                .next()
                .expect("the seed makes an account");
            let sent_folder = MailboxRepository::new(&connection)
                .list_for_account(account.id)
                .await
                .expect("folders")
                .into_iter()
                .find(|m| m.role == postio_model::MailboxRole::Sent)
                .expect("a Sent folder");
            let mut sent = Message::new(account.id, sent_folder.id, chrono::Utc::now());
            sent.from = vec![account.address.clone()];
            sent.to = vec![EmailAddress::new(Some("A. L."), "ada@home.example")];
            MessageRepository::new(&connection)
                .create(&mut sent)
                .await
                .expect("a sent message");
            ContactRepository::new(&connection)
                .record_message(&sent, std::slice::from_ref(&account.address))
                .await
                .expect("recorded as sync would");
            ContactRepository::new(&connection)
                .create(
                    Some("Ada Lovelace"),
                    &[EmailAddress::new(None::<String>, "ada@work.example")],
                )
                .await
                .expect("a person the user made");
        }
        ensure_search_index(&database).await.expect("the index");

        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
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
        postio_app::commands::install(
            &window,
            &feeds,
            state.clone(),
            wiring.commands.clone(),
            bus_verbs,
        );
        let notifier = postio_app::notifications::Notifier::new(
            database.clone(),
            wiring.store.clone(),
            wiring.runtime.clone(),
            Default::default(),
        );
        for stream in [events, replies] {
            postio_app::commands::drain(&window, &feeds, stream, notifier.clone(), state.clone());
        }
        assert!(settle_until(async || window.list().model().n_items() > 0).await);

        press(&window, "g");
        press(&window, "c");
        assert!(
            settle_until(async || drawn(&window).len() == 2).await,
            "drew {:?}",
            drawn(&window)
        );
        assert_eq!(drawn(&window), ["A. L.", "Ada Lovelace"]);

        // ── x, down, x, m, Return ──────────────────────────────────────────
        let pane = window.contacts();
        press(&window, "x");
        pane.set_cursor(1); // the list's own Down
        settle();
        press(&window, "x");
        press(&window, "m");
        assert!(
            settle_until(async || pane.join_open()).await,
            "m opened no join; hint {:?}",
            pane.hint_text()
        );
        assert_eq!(
            pane.join_names(),
            ["Ada Lovelace", "A. L."],
            "the name the user chose is offered first"
        );
        press(&window, "Return");
        assert!(
            settle_until(async || drawn(&window) == ["Ada Lovelace"]).await,
            "after the join the list drew {:?}",
            drawn(&window)
        );

        // ── completion offers one person with both addresses ───────────────
        {
            let connection = database.connect().await.expect("a connection");
            // The seed has other people called Ada; these are the ones that
            // own either of her two addresses.
            let offered: Vec<_> = ContactRepository::new(&connection)
                .complete("ada", 8)
                .await
                .expect("complete")
                .into_iter()
                .filter(|person| {
                    person.addresses.iter().any(|a| {
                        a.address.address == "ada@work.example"
                            || a.address.address == "ada@home.example"
                    })
                })
                .collect();
            assert_eq!(offered.len(), 1, "one person, not two");
            let addresses: Vec<&str> = offered[0]
                .addresses
                .iter()
                .map(|a| a.address.address.as_str())
                .collect();
            assert_eq!(
                addresses,
                ["ada@work.example", "ada@home.example"],
                "the survivor's own address stays preferred, and first"
            );
        }

        // ── u ───────────────────────────────────────────────────────────────
        press(&window, "u");
        assert!(
            settle_until(async || drawn(&window).len() == 2).await,
            "undo drew {:?}",
            drawn(&window)
        );
        assert_eq!(drawn(&window), ["A. L.", "Ada Lovelace"]);
        bridge.shutdown();
    });
}
