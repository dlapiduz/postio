use gtk::gdk;
use postio_gtk::digest::DigestPage;

use crate::digest::{delivered_holding, open_digest};
use crate::support;

const RATE: &str = "The committee held the rate at four percent for a third month.";
const WEEKLY: &str = "Retail numbers were flat this week, and freight was down.";
const MARKED: &str = "The rate held <a href=\"https://rates.example/\">here</a>; see https://rates.example/decision.";

async fn summarised() -> (crate::support::Fixture, [postio_model::MessageId; 2]) {
    let (fixture, delivery, held) = delivered_holding().await;
    let (weekly, rate) = (held[0], held[1]);
    fixture
        .write_body(rate, &format!("Good morning.\n\n{RATE}\n\nThe Ledger\n"))
        .await;
    fixture
        .write_body(weekly, &format!("This week:\n\n{WEEKLY}\n\nThe Ledger\n"))
        .await;
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

pub fn a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail() {
    crate::gtk_case(async {
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
