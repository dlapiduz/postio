//! The marker card and its sentence (US2 scenario 6, T073): a message with
//! a marker opens with the card under its headers -- the kind, the quote,
//! the action and its key -- and the quoted sentence is highlighted in the
//! body where it appears (research R2).

use crate::support::{self, Fixture};

const SENTENCE: &str = "Please leave comments by Wednesday";

pub fn a_marked_message_opens_with_its_card_and_its_sentence_highlighted() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor API draft v3",
                "Uploaded v3.",
                5,
            )
            .await;
        fixture
            .write_body(
                message,
                &format!(
                    "Hi all,\n\nUploaded v3 with the pagination changes.\n\n{SENTENCE}; I'd like to freeze it Thursday.\n\nLena\n"
                ),
            )
            .await;
        fixture.ask(message, SENTENCE).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("open");

        assert!(
            crate::settle_until(async || !reading.marker_card_said().is_empty()).await,
            "no marker card"
        );
        let card = reading.marker_card_said();
        for expected in ["Question", SENTENCE, "Reply", "e"] {
            assert!(
                card.iter().any(|said| said.contains(expected)),
                "the card does not say {expected:?}: {card:?}"
            );
        }

        assert!(
            crate::settle_until(async || !reading.reader().view().highlight_rects().is_empty())
                .await,
            "the sentence was not highlighted"
        );
        let document = reading.reader().view().document().expect("rendered");
        let found = document.text.find(SENTENCE);
        assert_eq!(found.len(), 1, "the sentence is drawn once");
        assert_eq!(
            reading.reader().view().highlight_rects(),
            document.text.rects(found[0].clone()),
            "the highlight covers the sentence, and only it"
        );
    });
}

/// T118's surface: the card offers "Dismiss" with `-`'s key, and pressing
/// it takes the marker off -- the card goes, and the list's row with it
/// draws one line again.
pub fn the_card_dismisses_its_marker() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor API draft v3",
                "Uploaded v3.",
                5,
            )
            .await;
        fixture
            .write_body(message, &format!("Hi all,\n\n{SENTENCE}.\n\nLena\n"))
            .await;
        fixture.ask(message, SENTENCE).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || !reading.marker_card_said().is_empty()).await,
            "no marker card"
        );
        let card = reading.marker_card_said();
        assert!(
            card.iter().any(|said| said == "Dismiss") && card.iter().any(|said| said == "-"),
            "the card offers Dismiss with its key: {card:?}"
        );
        assert!(reading.dismiss_marker(), "the card's Dismiss was pressed");
        assert!(
            crate::settle_until(async || reading.marker_card_said().is_empty()).await,
            "the card stayed: {:?}",
            reading.marker_card_said()
        );
        assert!(
            crate::settle_until(async || {
                window.cursor_row().is_some_and(|row| !row.two_lines())
            })
            .await,
            "the row still draws its marker"
        );
    });
}
