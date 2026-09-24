//! Possible duplicates, driven by the keys (specs/005-contacts User Story 4).
//!
//! Two people who share a name are offered; `m` then `Return` joins them and
//! `u` takes it back; `X` dismisses the pair and it is not offered again.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use crate::contacts_support::{display, press, wire};
use crate::settle_until;
use gtk::prelude::*;
use postio_model::EmailAddress;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

pub fn m_joins_a_suggestion_and_x_dismisses_one() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if !display() {
            return;
        }
        let database = test_support::memory().await;
        postio_storage::seed::seed_small(&database, 11).await;
        {
            let connection = database.connect().await.expect("a connection");
            let contacts = ContactRepository::new(&connection);
            for email in ["zq@work.example", "zq@home.example"] {
                contacts
                    .create(
                        Some("Zebulon Quux"),
                        &[EmailAddress::new(None::<String>, email)],
                    )
                    .await
                    .expect("a person");
            }
        }
        let app = wire(&database).await;
        let window = &app.window;
        assert!(settle_until(async || window.list().model().n_items() > 0).await);
        let pane = window.contacts();
        let zebulon = || {
            pane.suggestion_lines()
                .into_iter()
                .filter(|line| line.contains("zq@"))
                .count()
        };

        press(window, "g");
        press(window, "c");
        press(window, "v");
        press(window, "s");
        assert!(pane.suggestions_open());
        assert!(
            settle_until(async || zebulon() == 1).await,
            "suggested {:?}",
            pane.suggestion_lines()
        );
        // The seed may suggest pairs of its own; put the cursor on ours.
        let at = pane
            .suggestion_lines()
            .iter()
            .position(|line| line.contains("zq@"))
            .expect("ours");
        pane.focus_suggestion(at);

        // ── m, Return: joined, and no longer a suggestion ──────────────────
        press(window, "m");
        assert!(
            settle_until(async || pane.join_open()).await,
            "hint {:?}",
            pane.hint_text()
        );
        press(window, "Return");
        assert!(
            settle_until(async || zebulon() == 0).await,
            "still suggested after the join: {:?}",
            pane.suggestion_lines()
        );

        // ── u: two people again, and the pair is back ──────────────────────
        press(window, "u");
        assert!(
            settle_until(async || zebulon() == 1).await,
            "undo did not bring the pair back: {:?}",
            pane.suggestion_lines()
        );

        // ── X: dismissed, for good ─────────────────────────────────────────
        let at = pane
            .suggestion_lines()
            .iter()
            .position(|line| line.contains("zq@"))
            .expect("ours");
        pane.focus_suggestion(at);
        press(window, "X");
        assert!(settle_until(async || zebulon() == 0).await);
        pane.refresh();
        crate::settle();
        assert_eq!(zebulon(), 0, "not offered again");
        app.bridge.shutdown();
    });
}
