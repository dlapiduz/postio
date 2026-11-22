//! The open invitation is `invite-zone-without-vtimezone` with its day moved a month ahead, so it
//! does not fall past its own event.

use chrono::{Datelike as _, Duration, Utc};
use gtk::prelude::*;
use postio_gtk::list::RowWidget;
use postio_gtk::window::FocusWindow;
use postio_model::{Identity, MessageId};
use postio_storage::BlobStore;
use postio_storage::repository::{IdentityRepository, MessageRepository, StoredBody};

use crate::support::{self, Fixture};

const ATTENDEE: &str = "ada.norwood@example.com";

impl Fixture {
    pub async fn invited(&self) {
        let connection = self.database.connect().await.expect("a connection");
        let mut identity = Identity::new(
            self.account.id,
            postio_model::EmailAddress::new(Some("Ada Norwood"), ATTENDEE),
        );
        identity.display_name = "Ada Norwood".to_owned();
        IdentityRepository::new(&connection)
            .create(&mut identity)
            .await
            .expect("an identity");
        for folder in ["Drafts", "Sent"] {
            postio_storage::test_support::mailbox(&connection, &self.account, folder).await;
        }
    }

    pub async fn file_raw(&self, blobs: &BlobStore, raw: &[u8], minutes: i64) -> MessageId {
        let parsed = postio_model::mime::parse(raw);
        let calendar: Vec<(Option<String>, Vec<u8>)> = parsed
            .parts
            .iter()
            .filter(|part| {
                part.attachment
                    .mime_type
                    .eq_ignore_ascii_case("text/calendar")
            })
            .map(|part| (part.attachment.part_id.clone(), part.content.clone()))
            .collect();
        let body = parsed.body.clone();
        // By the real clock: Focus reads only the last 30 days of what was filed, so a fixed date
        // ages out.
        let mut message = parsed.into_message(
            self.account.id,
            self.inbox,
            Utc::now() - Duration::minutes(minutes),
        );
        let connection = self.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let id = messages.create(&mut message).await.expect("a message");
        for (part, bytes) in calendar {
            let blob = blobs.put(&bytes).expect("the part stored");
            messages
                .set_attachment_blob(id, part.as_deref().unwrap_or("2"), &blob)
                .await
                .expect("the part linked");
        }
        messages
            .set_body(
                id,
                &StoredBody {
                    text: body.text,
                    html: body.html,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("its body");
        postio_storage::repository::ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        id
    }
}

fn open_invitation() -> Vec<u8> {
    let ahead = Utc::now() + Duration::days(30);
    let day = format!("{:04}{:02}{:02}", ahead.year(), ahead.month(), ahead.day());
    postio_model::test_corpus::load("invite-zone-without-vtimezone")
        .text_lossy()
        .replace("20261020", &day)
        .into_bytes()
}

fn row(window: &FocusWindow, subject: &str) -> Option<RowWidget> {
    window.pane()?.rows_on_screen().into_iter().find(|row| {
        row.item().is_some_and(|item| {
            item.as_conversation().is_some_and(|row| {
                row.summary
                    .representative
                    .subject
                    .as_deref()
                    .is_some_and(|said| said.contains(subject))
            })
        })
    })
}

fn said(window: &FocusWindow, subject: &str) -> Vec<String> {
    row(window, subject)
        .map(|row| row.drawn().texts)
        .unwrap_or_default()
}

async fn open_over(raws: &[(&[u8], &str)]) -> (Fixture, FocusWindow) {
    let fixture = Fixture::empty().await;
    fixture.invited().await;
    let (host, _sink) = fixture.host_telling();
    let blobs = host.wiring().blobs.clone();
    for (index, (raw, _)) in raws.iter().enumerate() {
        fixture.file_raw(&blobs, raw, 10 + index as i64).await;
    }
    let window = FocusWindow::new(None);
    window.present();
    let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
    support::keep(session);
    for (_, subject) in raws {
        assert!(
            crate::settle_until(async || said(&window, subject).contains(&"Invite".to_owned()))
                .await,
            "{subject:?} never drew its Invite marker: {:?}",
            said(&window, subject)
        );
    }
    (fixture, window)
}

pub fn a_click_on_the_row_s_decline_declines() {
    crate::gtk_case(async {
        let invitation = open_invitation();
        let (_fixture, window) = open_over(&[(&invitation, "Portfolio review")]).await;
        let target = row(&window, "Portfolio review").expect("the invitation's row");
        let (x, y) = target
            .drawn()
            .action("Decline")
            .expect("the row draws a Decline button");
        support::click_in(&window, &target, x, y, 1);
        assert!(
            crate::settle_until(async || window.toast_showing().as_deref() == Some("Declined"))
                .await,
            "the click said nothing: {:?}",
            window.toast_showing()
        );
        assert!(
            crate::settle_until(
                async || said(&window, "Portfolio review").contains(&"Declined".to_owned())
            )
            .await,
            "the row says what was answered: {:?}",
            said(&window, "Portfolio review")
        );
    });
}

pub fn the_open_invitation_s_card_answers_with_its_keys() {
    crate::gtk_case(async {
        let invitation = open_invitation();
        let (_fixture, window) = open_over(&[(&invitation, "Portfolio review")]).await;
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the invitation");
        assert!(
            crate::settle_until(async || !reading.marker_card_said().is_empty()).await,
            "the dialog drew no marker card"
        );
        let card = reading.marker_card_said();
        for offered in ["Invite", "Accept", "y", "Decline", "Y"] {
            assert!(
                card.contains(&offered.to_owned()),
                "the card offers {offered:?}: {card:?}"
            );
        }

        support::keys(&window, &["Y"]);
        assert!(
            crate::settle_until(async || window.toast_showing().as_deref() == Some("Declined"))
                .await,
            "Y in the open message said nothing: {:?}",
            window.toast_showing()
        );
        assert!(reading.is_open(), "answering leaves the message open");
    });
}
