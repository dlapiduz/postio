//! The unsubscribe notice, wired end to end (#971): activating it in the
//! reader reaches the store, and Settings' Privacy section reads it back.
//!
//! The reader only asks; this proves Focus's window answers. The activation
//! lands in the unsubscribe log stamped with the message's own account, with
//! the sender's domain as the list when the message names none, and opening
//! Settings afterwards lists it. The same two-part shape as the egress log's
//! case in `settings_wiring`.
//!
//! Focus answers `U` (`CommandId::Unsubscribe`) in a digest, on the message
//! the digest's cursor is on. The open message's own unsubscribe notice is
//! not wired in Focus (`Reader::set_unsubscribe` is never called and `act`
//! has no arm for the command), so its case is held out in `IGNORED`.

use postio_model::BodyState;
use postio_storage::repository::{MessageRepository, StoredBody, UnsubscribeRepository};

use crate::support::{self, Fixture};

/// A plain message with no `List-Id`: the sender's domain is the fallback
/// list, so this exercises that path without depending on the model layer's
/// handling of RFC 2919's list headers, which it tests itself.
const NEWSLETTER: &[u8] = b"From: Weekly Digest <weekly@news.example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: This week in Postio\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Nothing much happened\r\n";

/// Personal mail: no `List-Id`, no `List-Unsubscribe`.
const PERSONAL: &[u8] = b"From: Grace Hopper <grace@friends.example.net>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: Lunch on Friday?\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Are you free\r\n";

/// Store `NEWSLETTER` in the fixture's inbox as a message with a body, one
/// that carries a `List-Unsubscribe` (its promoted headers say so, as sync
/// files them).
async fn file_the_newsletter(fixture: &Fixture) {
    file(fixture, NEWSLETTER, true).await;
}

/// Store `raw` in the fixture's inbox with a body; `offered` is what its
/// promoted `List-Unsubscribe` flag says.
async fn file(fixture: &Fixture, raw: &[u8], offered: bool) {
    let connection = fixture.database.connect().await.expect("a connection");
    let repository = MessageRepository::new(&connection);
    let parsed = postio_model::mime::parse(raw);
    let body = parsed.body.clone();
    let encoding_problems = parsed.encoding_problems;
    let mut message = parsed.into_message(fixture.account.id, fixture.inbox, chrono::Utc::now());
    message.sync.body_state = BodyState::Full;
    message.promoted = Some(postio_model::promoted::PromotedHeaders {
        unsubscribe_offered: offered,
        automation: 0,
    });
    let id = repository.create(&mut message).await.expect("a message");
    repository
        .set_body(
            id,
            &StoredBody {
                text: body.text,
                html: body.html,
                headers: None,
                headers_truncated: false,
                encoding_problems,
            },
            BodyState::Full,
        )
        .await
        .expect("a body");
}

async fn logged(fixture: &Fixture) -> Vec<postio_model::UnsubscribeActivation> {
    let connection = fixture.database.connect().await.expect("a connection");
    UnsubscribeRepository::new(&connection)
        .for_account(fixture.account.id)
        .await
        .expect("the log")
}

pub fn the_open_message_s_unsubscribe_notice_logs_it_and_the_privacy_section_lists_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file_the_newsletter(&fixture).await;
        let (window, _client) = fixture.open().await;

        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        assert!(
            crate::settle_until(
                async || reading.is_open() && reading.reader().unsubscribe_banner_visible()
            )
            .await,
            "a message with a sender always has a list to leave (the domain, \
             if nothing else), and the open message offers it"
        );
        assert!(logged(&fixture).await.is_empty(), "nothing is logged yet");

        reading.reader().click_unsubscribe();
        assert!(
            crate::settle_until(async || logged(&fixture).await.len() == 1).await,
            "activating the notice never reached storage: the reader only \
             asks, and nothing in the window answered"
        );
        let activations = logged(&fixture).await;
        assert_eq!(activations[0].account_id, fixture.account.id);
        assert_eq!(
            activations[0].list_identifier, "news.example.org",
            "no List-Id on this message, so the sender's domain is the list"
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        privacy_lists_an_activation(&window).await;
    });
}

/// Open Settings' Privacy section: it lists the activation.
async fn privacy_lists_an_activation(window: &postio_gtk::window::FocusWindow) {
    support::deliver_with(window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
    let dialog = crate::settings::settings_shown(window)
        .await
        .expect("Settings opened");
    let privacy = crate::settings::section_rows(&dialog)
        .into_iter()
        .find(|(said, _)| said == "Privacy")
        .map(|(_, row)| row)
        .expect("Settings lists a Privacy section");
    support::click(window, &privacy, 1);
    assert!(
        crate::settle_until(async || {
            !support::with_class(&dialog, "postio-settings-unsubscribe-row").is_empty()
        })
        .await,
        "Privacy lists no activations over a log that holds one"
    );
}

/// `U` in a digest leaves the list of the message the digest's cursor is on:
/// the log holds one activation stamped with the message's account and the
/// sender's domain (no `List-Id` here), the window says what it left, and
/// Privacy lists it.
pub fn u_in_a_digest_logs_the_activation_and_the_privacy_section_lists_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _delivery, _held) = crate::digest::delivered_holding().await;
        let (window, _client) = fixture.open().await;
        let digest = crate::digest::open_digest(&window).await;
        assert!(
            digest.focused().is_some(),
            "the digest's cursor is on a message"
        );
        assert!(logged(&fixture).await.is_empty(), "nothing is logged yet");

        support::keys(&window, &["U"]);
        assert!(
            crate::settle_until(async || logged(&fixture).await.len() == 1).await,
            "`U` in the digest never reached storage"
        );
        let activations = logged(&fixture).await;
        assert_eq!(activations[0].account_id, fixture.account.id);
        assert_eq!(
            activations[0].list_identifier, "ledger.test",
            "no List-Id on this message, so the sender's domain is the list"
        );
        assert!(
            crate::settle_until(
                async || window.toast_showing().as_deref() == Some("Unsubscribed from ledger.test")
            )
            .await,
            "the window did not say what it left: {:?}",
            window.toast_showing()
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        privacy_lists_an_activation(&window).await;
    });
}

/// Personal mail opens with no unsubscribe band, and `U` still leaves the
/// sender's domain: the band is for list mail, the key is not (T261).
pub fn personal_mail_has_no_unsubscribe_band_but_u_still_works() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file(&fixture, PERSONAL, false).await;
        let (window, _client) = fixture.open().await;

        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        assert!(
            crate::settle_until(async || reading.is_open() && reading.body_text().contains("free"))
                .await,
            "the message never rendered"
        );
        crate::settle();
        assert!(
            !reading.reader().unsubscribe_banner_visible(),
            "personal mail carries no unsubscribe band"
        );

        support::keys(&window, &["U"]);
        assert!(
            crate::settle_until(async || logged(&fixture).await.len() == 1).await,
            "`U` on a message with no band did nothing"
        );
        assert_eq!(
            logged(&fixture).await[0].list_identifier,
            "friends.example.net"
        );
    });
}
