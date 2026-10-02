//! Hostile mail opened the way a person opens it (spec 006 US3).
//!
//! `postio-render`'s hostile suite proves each fixture is contained and
//! bounded against the renderer alone. This is the application half: each
//! one stored, listed and opened from the message list, and after each the
//! open message shows a drawn body or the plain-text fallback -- never a
//! blank pane -- and the window still answers the next keystroke. The
//! process being alive to assert anything is the third claim.

use postio_model::test_corpus::{self, Category};
use postio_storage::repository::{MessageRepository, StoredBody};

use crate::support::{self, Fixture};

/// The subject of the row the cursor is on.
fn cursor_subject(window: &postio_focus::window::FocusWindow) -> Option<String> {
    window
        .cursor_row()?
        .as_conversation()?
        .summary
        .representative
        .subject
        .clone()
}

pub fn each_hostile_message_opens_and_the_app_keeps_answering() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let fixtures = test_corpus::by_category(Category::Hostile);
        assert!(fixtures.len() >= 5, "the corpus has its hostile fixtures");
        let connection = fixture.database.connect().await.expect("a connection");
        let repository = MessageRepository::new(&connection);
        // Newest first in the list, in the corpus's order.
        for (index, hostile) in fixtures.iter().enumerate() {
            let (message, _) = fixture
                .file(
                    ("Sender", "sender@example.com"),
                    hostile.name(),
                    "x",
                    index as i64,
                )
                .await;
            let parsed = postio_model::mime::parse(hostile.bytes());
            repository
                .set_body(
                    message,
                    &StoredBody {
                        text: parsed.body.text.clone(),
                        html: parsed.body.html.clone(),
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::BodyState::Full,
                )
                .await
                .expect("a body");
        }
        drop(connection);
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == fixtures.len()).await,
            "the hostile mail never reached the list: {:?}",
            support::subjects(&window)
        );

        for (at, hostile) in fixtures.iter().enumerate() {
            let name = hostile.name();
            // The next keystroke moves the cursor onto this one.
            let before = cursor_subject(&window);
            support::keys(&window, &["j"]);
            assert!(
                crate::settle_until(async || cursor_subject(&window) != before).await,
                "{name}: `j` was not answered after opening the one before it"
            );
            assert_eq!(
                cursor_subject(&window).as_deref(),
                Some(name),
                "{name}: row {at} is not where the corpus put it"
            );
            support::deliver(&window, "Return");
            let reading = window.reading().expect("Return opened the message");
            assert!(
                crate::settle_until(
                    async || reading.is_open() && reading.reader().view().document().is_some()
                )
                .await,
                "{name}: the pane never drew the message or its fallback"
            );
            let document = reading.reader().view().document().expect("a snapshot");
            assert!(
                matches!(document.outcome, postio_render::Outcome::Rendered)
                    || !document.text.text.trim().is_empty(),
                "{name}: the pane is blank -- neither a drawn body nor the \
                 fallback's text"
            );
            reading.close();
            assert!(
                crate::settle_until(async || window.reading().is_none_or(|r| !r.is_open())).await,
                "{name}: the message did not close"
            );
        }
    });
}
