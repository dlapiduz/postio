//! US1 scenario 1: given a synced store and no network, when Focus starts,
//! the inbox is drawn -- without waiting on any server.
//!
//! The store is a fixture, opened by a host as Focus's startup opens one,
//! and nothing here can reach a network: no sync is started, and the
//! account's server is never dialled. What is asserted is what a person
//! sees: the rows on screen, newest first, saying what the mail says.

use gtk::prelude::*;
use postio_gtk::startup;
use postio_gtk::window::FocusWindow;

use crate::support::{self, Fixture};

pub fn the_inbox_is_listed_from_the_store_with_no_network() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "The numbers are attached.",
                30,
            )
            .await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Uploaded the second draft.",
                20,
            )
            .await;
        fixture
            .file(
                ("Tomás Reyes", "tomas@example.net"),
                "Staffing plan",
                "Sharing the draft before Monday.",
                10,
            )
            .await;

        let window = FocusWindow::new(None);
        window.present();
        let session = startup::adopt(&window, fixture.host(), &postio_config::Config::default());

        assert!(
            crate::settle_until(async || window.rows_on_screen().len() >= 3).await,
            "Focus never listed the inbox from the store: rows on screen {:?}",
            window.rows_on_screen()
        );
        let rows = window.rows_on_screen();
        for (row, (sender, subject)) in rows.iter().zip([
            ("Tomás Reyes", "Staffing plan"),
            ("Lena Park", "Harbor draft"),
            ("Ada Moreno", "Atlas budget"),
        ]) {
            assert!(
                row.contains(sender) && row.contains(subject),
                "the inbox is newest first, each row saying who and what: \
                 expected {sender} / {subject}, the row says {row:?} (all: {rows:?})"
            );
        }
        assert!(
            !session.syncing(),
            "nothing here may start sync: the scenario is the store alone"
        );
        support::keep(session);
    });
}
