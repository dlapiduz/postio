//! Deleting a person, and bringing them back (specs/005-contacts User Story
//! 3, FR-023, FR-023a, SC-005).
//!
//! `d` on a person takes them out of the list and out of completion, and
//! more mail from their address does not bring them back; `v d` shows them,
//! `r` restores them, and `u` after a delete is a restore too.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use crate::contacts_support::{display, drawn, press, wire};
use crate::settle_until;
use gtk::prelude::*;
use postio_model::{EmailAddress, Message};
use postio_storage::repository::{AccountRepository, ContactRepository, MailboxRepository};
use postio_storage::test_support;

async fn offered(database: &postio_storage::Store, prefix: &str) -> Vec<String> {
    let connection = database.connect().await.expect("a connection");
    ContactRepository::new(&connection)
        .complete(prefix, 8)
        .await
        .expect("complete")
        .into_iter()
        .map(|p| p.display_name().to_owned())
        .collect()
}

pub fn d_deletes_and_more_mail_does_not_bring_them_back() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if !display() {
            return;
        }
        let database = test_support::memory().await;
        postio_storage::seed::seed_small(&database, 11).await;
        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            let account = AccountRepository::new(&connection)
                .list()
                .await
                .expect("accounts")
                .into_iter()
                .next()
                .expect("an account");
            let inbox = MailboxRepository::new(&connection)
                .list_for_account(account.id)
                .await
                .expect("folders")
                .into_iter()
                .find(|m| m.role == postio_model::MailboxRole::Inbox)
                .expect("an inbox")
                .id;
            ContactRepository::new(&connection)
                .create(
                    Some("Zebulon Quux"),
                    &[EmailAddress::new(None::<String>, "zq@example.net")],
                )
                .await
                .expect("a person the user made");
            (account, inbox)
        };
        let app = wire(&database).await;
        let window = &app.window;
        assert!(settle_until(async || window.list().model().n_items() > 0).await);

        press(window, "g");
        press(window, "c");
        let pane = window.contacts();
        pane.filter().set_text("zebulon");
        assert!(
            settle_until(async || drawn(window) == ["Zebulon Quux"]).await,
            "drew {:?}",
            drawn(window)
        );

        // ── d ───────────────────────────────────────────────────────────────
        press(window, "d");
        assert!(
            settle_until(async || drawn(window).is_empty()).await,
            "the deleted person is still drawn: {:?}",
            drawn(window)
        );
        assert!(offered(&database, "zeb").await.is_empty(), "nor offered");

        // Mail from their address, recorded as sync records it.
        {
            let connection = database.connect().await.expect("a connection");
            let mut message = Message::new(account.id, inbox, chrono::Utc::now());
            message.from = vec![EmailAddress::new(Some("Zebulon Quux"), "zq@example.net")];
            ContactRepository::new(&connection)
                .record_message(&message, std::slice::from_ref(&account.address))
                .await
                .expect("recorded");
        }
        pane.refresh();
        crate::settle();
        assert!(
            drawn(window).is_empty(),
            "more mail did not bring them back"
        );
        assert!(offered(&database, "zeb").await.is_empty());

        // ── v d, r ──────────────────────────────────────────────────────────
        press(window, "v");
        press(window, "d");
        assert!(
            settle_until(async || drawn(window) == ["Zebulon Quux"]).await,
            "the Deleted view drew {:?}",
            drawn(window)
        );
        press(window, "r");
        assert!(
            settle_until(async || drawn(window).is_empty()).await,
            "restored, so gone from the Deleted view"
        );
        assert_eq!(offered(&database, "zeb").await, ["Zebulon Quux"]);

        // ── d, then u ───────────────────────────────────────────────────────
        press(window, "v");
        press(window, "d");
        assert!(settle_until(async || drawn(window) == ["Zebulon Quux"]).await);
        press(window, "d");
        assert!(settle_until(async || drawn(window).is_empty()).await);
        press(window, "u");
        assert!(
            settle_until(async || drawn(window) == ["Zebulon Quux"]).await,
            "u after d drew {:?}",
            drawn(window)
        );
        app.bridge.shutdown();
    });
}
