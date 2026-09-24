//! Groups made on the Contacts screen and picked in the composer
//! (specs/005-contacts User Story 5, FR-040, FR-041).
//!
//! `g n` makes "Family" of the marked person, `l` adds another, and typing
//! the group's name in a new draft's To field fills in both members'
//! preferred addresses -- read at that moment, so changing the group
//! afterwards leaves the draft alone.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use crate::contacts_support::{display, drawn, press, wire};
use crate::{settle, settle_until};
use gtk::prelude::*;
use postio_model::{Draft, EmailAddress};
use postio_storage::repository::{AccountRepository, ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

pub fn a_group_made_by_key_fills_a_draft_with_preferred_addresses() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if !display() {
            return;
        }
        let database = test_support::memory().await;
        postio_storage::seed::seed_small(&database, 11).await;
        let account = {
            let connection = database.connect().await.expect("a connection");
            let contacts = ContactRepository::new(&connection);
            contacts
                .create(
                    Some("Yara Quux"),
                    &[
                        EmailAddress::new(None::<String>, "yara@work.example"),
                        EmailAddress::new(None::<String>, "yara@home.example"),
                    ],
                )
                .await
                .expect("yara");
            contacts
                .create(
                    Some("Zebulon Quux"),
                    &[EmailAddress::new(None::<String>, "zq@example.net")],
                )
                .await
                .expect("zebulon");
            AccountRepository::new(&connection)
                .list()
                .await
                .expect("accounts")
                .into_iter()
                .next()
                .expect("an account")
                .id
        };
        let app = wire(&database).await;
        let window = &app.window;
        assert!(settle_until(async || window.list().model().n_items() > 0).await);

        // ── g n with Yara marked, then l for Zebulon ───────────────────────
        press(window, "g");
        press(window, "c");
        let pane = window.contacts();
        pane.filter().set_text("quux");
        assert!(
            settle_until(async || drawn(window) == ["Yara Quux", "Zebulon Quux"]).await,
            "drew {:?}",
            drawn(window)
        );
        press(window, "x");
        press(window, "g");
        press(window, "n");
        assert!(pane.group_panel_open(), "g n asked for no name");
        pane.submit_group_name("Quuxes");
        assert!(
            settle_until(async || pane.group_lines().contains(&"Quuxes".to_owned())).await,
            "groups drew {:?}",
            pane.group_lines()
        );
        pane.set_cursor(1);
        settle();
        press(window, "l");
        assert!(pane.group_panel_open(), "l asked for no group");
        let at = pane
            .group_lines()
            .iter()
            .position(|g| g == "Quuxes")
            .expect("ours");
        pane.choose_group(at);
        let members = async || {
            let connection = database.connect().await.expect("a connection");
            let groups = ContactGroupRepository::new(&connection);
            let Some(group) = groups
                .list()
                .await
                .expect("list")
                .into_iter()
                .find(|g| g.name == "Quuxes")
            else {
                return 0;
            };
            groups.members(group.id).await.expect("members").len()
        };
        assert!(settle_until(async || members().await == 2).await);

        // ── the composer: both preferred addresses, read now ───────────────
        press(window, "Escape");
        let composer = window.composer();
        composer.open(Draft::new(account));
        settle();
        composer.test_set_to("Quuxe");
        assert!(
            settle_until(async || composer.test_recipient_suggestion_count() > 0).await,
            "the group was not offered"
        );
        assert!(composer.test_accept_recipient_suggestion());
        settle();
        let to: Vec<String> = composer
            .draft()
            .to
            .iter()
            .map(|a| a.address.clone())
            .collect();
        assert_eq!(
            to,
            ["yara@work.example", "zq@example.net"],
            "preferred addresses"
        );

        {
            let connection = database.connect().await.expect("a connection");
            let groups = ContactGroupRepository::new(&connection);
            let group = groups
                .list()
                .await
                .expect("list")
                .into_iter()
                .find(|g| g.name == "Quuxes")
                .expect("ours");
            let zebulon = ContactRepository::new(&connection)
                .by_address("zq@example.net")
                .await
                .expect("lookup")
                .expect("zebulon")
                .id;
            groups
                .remove_members(group.id, &[zebulon])
                .await
                .expect("edit the group");
        }
        settle();
        assert_eq!(
            composer.draft().to.len(),
            2,
            "the draft keeps what the group was when it was picked (FR-041)"
        );
        composer.discard();
        app.bridge.shutdown();
    });
}
