//! Answering an invitation from its row (US8, FR-100 to FR-103; T113),
//! over the corpus' invitations filed as sync files mail: the body stage
//! Focus runs turns each calendar part into its marker, and the row draws
//! it.
//!
//! - An open invitation's row says Invite, its time, and Accept `y` /
//!   Decline `Y`. `y` answers it; the toast says "Accepted" with Undo, for
//!   as long as the answer can be taken back (`RSVP_WINDOW`), and then
//!   goes. The row says what was answered.
//! - A click on a row's drawn Decline does what `Y` does.
//! - A cancelled invitation, and one whose event is over, offer no answer,
//!   and `y` on them answers nothing.
//!
//! The open invitation is `invite-zone-without-vtimezone` with its day moved
//! to a month from now, so the case does not fall past its own event; the
//! cancelled and past ones are the corpus' own, which stay so.

use chrono::{Datelike as _, Duration, Utc};
use gtk::prelude::*;
use postio_focus::list::RowWidget;
use postio_focus::window::FocusWindow;
use postio_model::{Identity, MessageId};
use postio_storage::BlobStore;
use postio_storage::repository::{IdentityRepository, MessageRepository, StoredBody};

use crate::support::{self, Fixture};

/// The address the corpus' invitations are sent to, which the account
/// sends from, so there is someone to answer as.
const ATTENDEE: &str = "ada.norwood@example.com";

impl Fixture {
    /// The account sends as the invitations' attendee, and has the folders
    /// an answer is queued and filed through.
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

    /// File `raw` into the inbox `minutes` ago, as sync and the body
    /// backfill leave it: the message, its text, and its calendar part's
    /// bytes in `blobs`.
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
        let mut message = parsed.into_message(
            self.account.id,
            self.inbox,
            support::now() - Duration::minutes(minutes),
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

/// `invite-zone-without-vtimezone`, its event moved to a month from today.
fn open_invitation() -> Vec<u8> {
    let ahead = Utc::now() + Duration::days(30);
    let day = format!("{:04}{:02}{:02}", ahead.year(), ahead.month(), ahead.day());
    postio_model::test_corpus::load("invite-zone-without-vtimezone")
        .text_lossy()
        .replace("20261020", &day)
        .into_bytes()
}

/// The row on screen whose subject contains `subject`.
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

/// What the row whose subject contains `subject` drew.
fn said(window: &FocusWindow, subject: &str) -> Vec<String> {
    row(window, subject)
        .map(|row| row.drawn().texts)
        .unwrap_or_default()
}

/// Focus over a store holding `raws`, once each has its Invite marker.
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
    let session = postio_focus::startup::adopt(&window, host, &postio_config::Config::default());
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

/// US8 scenarios 1-3's part on screen: the row offers Accept `y` and
/// Decline `Y`, `y` answers, and the toast lasts the answer's window.
pub fn y_accepts_from_the_row_and_the_toast_lasts_the_window() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let invitation = open_invitation();
        let (_fixture, window) = open_over(&[(&invitation, "Portfolio review")]).await;
        let line = said(&window, "Portfolio review");
        for offered in ["Accept", "y", "Decline", "Y"] {
            assert!(
                line.contains(&offered.to_owned()),
                "the row offers {offered:?}: {line:?}"
            );
        }

        support::keys(&window, &["j", "y"]);
        assert!(
            crate::settle_until(async || window.toast_showing().as_deref() == Some("Accepted"))
                .await,
            "y said nothing: {:?}",
            window.toast_showing()
        );
        let toast = window.toast().expect("the toast on screen");
        assert_eq!(
            toast.button_label().as_deref(),
            Some("Undo"),
            "\"Accepted \u{b7} Undo\""
        );
        let window_lasts = postio_session::actions::RSVP_WINDOW.as_secs() as u32;
        assert_eq!(
            toast.timeout(),
            window_lasts,
            "the toast lasts as long as the answer can be taken back"
        );
        assert!(
            crate::settle_until(async || {
                let line = said(&window, "Portfolio review");
                line.contains(&"Accepted".to_owned()) && !line.contains(&"Accept".to_owned())
            })
            .await,
            "the row says what was answered, and offers no second answer: {:?}",
            said(&window, "Portfolio review")
        );

        // It goes by itself once the window has closed, and not before the
        // ordinary toast's eight seconds would have taken it.
        let shown = std::time::Instant::now();
        let gone = std::time::Duration::from_secs(u64::from(window_lasts) + 5);
        while window.toast_showing().is_some() && shown.elapsed() < gone {
            crate::settle();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            window.toast_showing(),
            None,
            "the toast outlived its window"
        );
        assert!(
            shown.elapsed()
                >= std::time::Duration::from_secs(u64::from(
                    postio_widgets::widgets::toast::TOAST_TIMEOUT
                )),
            "the toast went with an ordinary toast's timeout, not the window's"
        );
    });
}

/// A click on the row's own Decline does what `Y` does.
pub fn a_click_on_the_row_s_decline_declines() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let invitation = open_invitation();
        let (_fixture, window) = open_over(&[(&invitation, "Portfolio review")]).await;
        let target = row(&window, "Portfolio review").expect("the invitation's row");
        let (x, y) = target
            .drawn()
            .action("Decline")
            .expect("the row draws a Decline button");
        target.press_at(x, y);
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

/// US8 scenarios 4 and 5: a cancelled invitation and one whose event is
/// over show no Accept or Decline, and `y` on them answers nothing.
pub fn a_cancelled_or_past_invitation_offers_no_answer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let cancelled = postio_model::test_corpus::load("invite-cancel").bytes();
        let past = postio_model::test_corpus::load("calendar-invite").bytes();
        let (_fixture, window) = open_over(&[(cancelled, "Canceled"), (past, "Invitation")]).await;
        for (subject, status) in [("Canceled", "Cancelled"), ("Invitation", "Past")] {
            let line = said(&window, subject);
            assert!(
                line.contains(&status.to_owned()),
                "{subject:?} says {status:?}: {line:?}"
            );
            for answer in ["Accept", "Decline"] {
                assert!(
                    !line.contains(&answer.to_owned()),
                    "{subject:?} offers {answer:?}: {line:?}"
                );
            }
        }
        // `y` on each: nothing is answered.
        for _ in 0..2 {
            support::keys(&window, &["j", "y"]);
        }
        assert!(
            crate::settle_while(async || { window.toast_showing().as_deref() != Some("Accepted") })
                .await,
            "an invitation with nothing to answer was answered"
        );
    });
}

/// The open message's Invite card (T113's dialog half, screen 04): it
/// offers Accept `y` and Decline `Y`, and `Y` in the dialog declines the
/// invitation on screen.
pub fn the_open_invitation_s_card_answers_with_its_keys() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let invitation = open_invitation();
        let (_fixture, window) = open_over(&[(&invitation, "Portfolio review")]).await;
        support::keys(&window, &["j"]);
        let _ = window.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty());
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
