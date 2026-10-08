//! The empty inbox (US6 scenario 4, screen 16): "Inbox is empty", and under
//! it only what exists -- when the next digest comes if there are digests,
//! and the filtered count if filtering is on.

use gtk::prelude::*;
use postio_storage::repository::{
    FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason,
};

use crate::support::{self, Fixture};

/// Open `fixture`'s store under `config`, and wait for the empty page.
async fn open_empty(
    fixture: &Fixture,
    config: &postio_config::Config,
) -> postio_gtk::window::FocusWindow {
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    support::keep(postio_gtk::startup::adopt(&window, fixture.host(), config));
    assert!(
        crate::settle_until(async || !empty_says(&window).is_empty()).await,
        "the empty inbox never showed"
    );
    window
}

/// What the empty page says, as a person reads it.
fn empty_says(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    support::with_class(window, "focus-empty")
        .first()
        .map(support::texts)
        .unwrap_or_default()
}

pub fn with_filtering_the_empty_inbox_counts_what_was_filtered_today() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        // Two messages filed away this morning, out of the inbox.
        let connection = fixture.database.connect().await.expect("a connection");
        let archive =
            postio_storage::test_support::mailbox(&connection, &fixture.account, "Filtered mail")
                .await;
        for _ in 0..2 {
            let mut message =
                postio_model::Message::new(fixture.account.id, archive.id, chrono::Utc::now());
            postio_storage::repository::MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            FilterDecisionRepository::new(&connection)
                .record(&FilterDecision {
                    message: message.id,
                    reason: FilterReason::Promotion,
                    source: None,
                    layer: FilterLayer::Header,
                    decided_at: chrono::Utc::now(),
                })
                .await
                .expect("recorded");
        }
        drop(connection);

        let window = open_empty(&fixture, &postio_config::Config::default()).await;
        let said = empty_says(&window);
        assert!(said.contains(&"Inbox is empty".to_owned()), "{said:?}");
        assert!(
            said.iter().all(|line| !line.starts_with("Next digest")),
            "no digests, so no next digest: {said:?}"
        );
        for expected in ["2 filtered today", "archive", "compose"] {
            assert!(
                said.contains(&expected.to_owned()),
                "no {expected:?}: {said:?}"
            );
        }
        for key in ["g f", "g r", "c"] {
            assert!(
                said.contains(&key.to_owned()),
                "no keycap {key:?}: {said:?}"
            );
        }
        assert!(window.rows_on_screen().is_empty(), "no rows behind it");
    });
}

pub fn without_filtering_the_empty_inbox_names_the_next_digest_and_no_count() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let config = postio_config::Config::from_toml_str(
            "[focus]\nfiltering = false\n\n[[focus.digests]]\nname = \"Newsletters\"\n\
             queries = [\"from:news@example.com\"]\ncadence = \"weekly\"\nday = \"saturday\"\n\
             at = \"16:00\"\n",
        )
        .expect("a config");
        let window = open_empty(&fixture, &config).await;
        let said = empty_says(&window);
        assert!(
            said.iter()
                .any(|line| line.starts_with("Next digest: Weekly \u{b7} Newsletters, ")),
            "the next digest is named: {said:?}"
        );
        assert!(
            said.iter().all(|line| !line.contains("filtered today")),
            "filtering is off, so no filtered count: {said:?}"
        );
        assert!(said.contains(&"archive".to_owned()) && said.contains(&"compose".to_owned()));
    });
}
