//! A digest read as a summary that cites its mail (US13, T154; screens 22
//! and 23). The summary is stored with the delivery as the summariser
//! would have stored it, so no model runs: what is tested is what the
//! window makes of it.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::digest::DigestPage;

use crate::digest::{delivered_holding, open_digest};
use crate::support;

const RATE: &str = "The committee held the rate at four percent for a third month.";
const WEEKLY: &str = "Retail numbers were flat this week, and freight was down.";
/// A statement whose words carry markup and a link, as a model might write
/// them (scenario 3).
const MARKED: &str = "The rate held <a href=\"https://rates.example/\">here</a>; see https://rates.example/decision.";

/// The two held messages with bodies, and a summary of them: two
/// statements on one topic, one on another, and one citing a message the
/// digest does not hold -- which must not be shown (FR-173).
async fn summarised() -> (crate::support::Fixture, [postio_model::MessageId; 2]) {
    let (fixture, delivery, held) = delivered_holding().await;
    let (weekly, rate) = (held[0], held[1]);
    fixture
        .write_body(rate, &format!("Good morning.\n\n{RATE}\n\nThe Ledger\n"))
        .await;
    fixture
        .write_body(weekly, &format!("This week:\n\n{WEEKLY}\n\nThe Ledger\n"))
        .await;
    // Filed away from the inbox, so the inbox is what `open_digest` expects.
    let archive = fixture.folder("Kept").await;
    let stray = fixture.file_in(archive, "Not held", 90).await;
    let statement = |topic: &str, text: &str, number: u32, message, excerpt: &str| {
        postio_model::summary::SummaryStatement {
            topic: topic.to_owned(),
            text: text.to_owned(),
            reference: postio_model::summary::SummaryReference {
                number,
                message,
                excerpt: excerpt.to_owned(),
            },
        }
    };
    let summary = postio_model::summary::DigestSummary {
        statements: vec![
            statement("Rates", MARKED, 1, rate, "held the rate at four percent"),
            statement(
                "Rates",
                "Nothing moves until spring.",
                1,
                rate,
                "for a third month",
            ),
            statement(
                "Trade",
                "Shops had a flat week.",
                2,
                weekly,
                "numbers were flat",
            ),
            statement(
                "Elsewhere",
                "A statement citing nothing held.",
                3,
                stray,
                "About Not held",
            ),
        ],
        messages: 2,
        senders: 1,
    };
    let connection = fixture.database.connect().await.expect("a connection");
    postio_storage::repository::DigestRepository::new(&connection)
        .set_summary(
            delivery,
            &serde_json::to_string(&summary).expect("it serialises"),
            support::now(),
        )
        .await
        .expect("stored");
    drop(connection);
    (fixture, [weekly, rate])
}

/// US13 scenarios 1, 3 and 6, and SC-014: a digest with a summary opens on
/// it; every statement shown ends in its numbered reference, a statement
/// whose reference is not the digest's is not shown, markup and links read
/// as plain characters, and `Tab` switches to the plain list and back.
pub fn a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _) = summarised().await;
        let (window, _client) = fixture.open().await;
        let digest = open_digest(&window).await;
        assert!(
            crate::settle_until(async || digest.showing() == DigestPage::Summary).await,
            "the digest did not open on its summary: {:?}",
            digest.showing()
        );
        assert_eq!(
            digest.references(),
            [1, 1, 2],
            "every statement shown ends in its reference, in reading order"
        );
        let said = digest.texts();
        for wanted in [
            "Rates",
            "Trade",
            "Written on this computer by the local model from these 2 messages only. \
             Every statement links to the email it came from.",
        ] {
            assert!(
                said.iter().any(|text| text.contains(wanted)),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            said.iter()
                .all(|text| !text.contains("A statement citing nothing held")),
            "a statement citing mail the digest does not hold was shown: {said:?}"
        );
        let paragraphs = digest.paragraphs();
        assert!(
            paragraphs.iter().any(|label| label.text().contains(MARKED)),
            "the model's markup is not shown as its characters: {:?}",
            paragraphs
                .iter()
                .map(|label| label.text())
                .collect::<Vec<_>>()
        );
        assert!(
            paragraphs.iter().all(|label| !label.uses_markup()),
            "a summary paragraph reads its text as markup"
        );

        support::press(&window, "Tab", gdk::ModifierType::empty());
        assert_eq!(digest.showing(), DigestPage::List, "Tab shows the messages");
        assert!(
            digest.focused().is_some(),
            "the list has its cursor for j and k"
        );
        support::press(&window, "Tab", gdk::ModifierType::empty());
        assert_eq!(
            digest.showing(),
            DigestPage::Summary,
            "Tab again, the summary"
        );
    });
}

/// US13 scenario 2 and SC-014: `]` and `[` move between references, and
/// the focused one shows its message under the paragraph; `Enter` opens
/// that email in the same window with the cited passage highlighted, byte
/// for byte, and a banner naming the reference; `j` steps to the digest's
/// next source; `Escape` goes back to the summary at the same reference.
pub fn a_reference_opens_its_email_in_place_with_the_passage_highlighted() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, [weekly, rate]) = summarised().await;
        let (window, _client) = fixture.open().await;
        let digest = open_digest(&window).await;
        assert!(
            crate::settle_until(async || digest.showing() == DigestPage::Summary).await,
            "no summary"
        );
        assert_eq!(digest.focused_reference(), Some(0), "the first is focused");
        support::press(&window, "bracketright", gdk::ModifierType::empty());
        support::press(&window, "bracketright", gdk::ModifierType::empty());
        assert_eq!(digest.focused_reference(), Some(2), "] moves on");
        support::press(&window, "bracketright", gdk::ModifierType::empty());
        assert_eq!(digest.focused_reference(), Some(2), "not past the last");
        support::press(&window, "bracketleft", gdk::ModifierType::empty());
        support::press(&window, "bracketright", gdk::ModifierType::empty());
        assert_eq!(digest.focused_reference(), Some(2), "[ moves back");
        assert!(
            digest
                .texts()
                .iter()
                .any(|text| text.contains("The weekly numbers")),
            "the focused reference does not show its message: {:?}",
            digest.texts()
        );
        assert_eq!(
            digest.focused().map(|row| row.id),
            Some(weekly),
            "D and U act on the referenced message"
        );

        support::press(&window, "Return", gdk::ModifierType::empty());
        assert_eq!(digest.showing(), DigestPage::Email, "Enter opens the email");
        assert!(
            window.reading().is_none() && window.digest().is_some(),
            "the email opened in a second window"
        );
        assert_eq!(digest.shown(), Some(weekly));
        let said = digest.texts();
        assert!(
            said.iter()
                .any(|text| text.contains("Cited as 2 in the summary; the passage is highlighted")),
            "no banner naming the reference: {said:?}"
        );
        assert!(
            crate::settle_until(async || !digest.reader().view().highlight_rects().is_empty())
                .await,
            "the cited passage was not highlighted"
        );
        let document = digest.reader().view().document().expect("rendered");
        let found = document.text.find("numbers were flat");
        assert_eq!(found.len(), 1, "the passage is drawn once");
        assert_eq!(
            digest.reader().view().highlight_rects(),
            document.text.rects(found[0].clone()),
            "the highlight is the cited passage, and only it"
        );

        support::press(&window, "j", gdk::ModifierType::empty());
        assert_eq!(digest.shown(), Some(rate), "j steps to the next source");
        assert!(
            crate::settle_until(async || {
                digest
                    .texts()
                    .iter()
                    .any(|text| text.contains("Cited as 1 in the summary"))
            })
            .await,
            "the next source's banner names its own reference: {:?}",
            digest.texts()
        );

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert_eq!(digest.showing(), DigestPage::Summary, "Esc, the summary");
        assert!(window.digest().is_some(), "Esc did not close the window");
        assert_eq!(
            digest.focused_reference(),
            Some(2),
            "back at the same reference"
        );
    });
}

/// The digest window's summary takes `j` and `k` as it takes `]` and `[`,
/// and shows where the keyboard is: the focused statement wears the ring,
/// the keyboard is on it, and a key pressed now is heard (the storyboard
/// review of 2026-10: `j` did nothing, and the outline sat on an invisible
/// row of the list behind the summary).
pub fn the_summary_takes_j_and_k_and_rings_the_focused_statement() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _) = summarised().await;
        let (window, _client) = fixture.open().await;
        let digest = open_digest(&window).await;
        assert!(
            crate::settle_until(async || digest.showing() == DigestPage::Summary).await,
            "no summary"
        );
        assert!(
            crate::settle_until(async || digest.keyboard_on_statement(0)).await,
            "the keyboard never reached the first statement: {}",
            support::focus_path(&window)
        );
        let ringed = |digest: &postio_gtk::digest::DigestWindow| -> Vec<usize> {
            digest
                .statement_rows()
                .iter()
                .enumerate()
                .filter(|(_, row)| row.has_css_class("focus-digest-summary-focused"))
                .map(|(index, _)| index)
                .collect()
        };
        assert_eq!(ringed(&digest), [0], "the first statement wears the ring");
        assert!(
            crate::settle_until(async || window.observe().keyboard.reachable).await,
            "a key pressed now is heard: the keyboard is on {}",
            support::focus_path(&window)
        );
        assert!(
            digest.keyboard_on_statement(0),
            "the keyboard is on the focused statement: {}",
            support::focus_path(&window)
        );

        support::keys(&window, &["j"]);
        assert_eq!(digest.focused_reference(), Some(1), "j moves to the next");
        assert_eq!(ringed(&digest), [1], "and the ring moves with it");
        assert!(
            crate::settle_until(async || {
                digest.keyboard_on_statement(1) && window.observe().keyboard.reachable
            })
            .await,
            "the keyboard moved with it: {}",
            support::focus_path(&window)
        );
        support::keys(&window, &["j", "j"]);
        assert_eq!(digest.focused_reference(), Some(2), "not past the last");
        support::keys(&window, &["k"]);
        assert_eq!(digest.focused_reference(), Some(1), "k moves back");
        assert_eq!(ringed(&digest), [1]);

        let said = digest.texts();
        assert!(
            said.iter()
                .any(|text| text == "Rates \u{b7} 2 statements from 1 message"),
            "a group says what it holds and from where: {said:?}"
        );
        assert!(
            said.iter().any(|text| text.starts_with("Reference ")),
            "the card names which reference is focused: {said:?}"
        );
    });
}
