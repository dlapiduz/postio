//! The unsubscribe notice, wired end to end (#971): activating it in the
//! reader reaches the store, and Settings' Privacy section reads it back.
//!
//! The reader only asks; this proves Focus's window answers. The activation
//! lands in the unsubscribe log stamped with the message's own account, with
//! the sender's domain as the list when the message names none, and opening
//! Settings afterwards lists it. The same two-part shape as the egress log's
//! case in `settings_wiring`.
//!
//! Focus answers `M` (`CommandId::Unsubscribe`) in a digest, on the message
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

        ignore_the_offer(&window).await;
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        privacy_lists_an_activation(&window).await;
    });
}

/// The offer to clear what the list sent is a toast with one button, not a
/// dialog (#1952); these cases are about the log, so they ignore it.
async fn ignore_the_offer(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || {
            window
                .toast()
                .and_then(|toast| toast.button_label())
                .is_some_and(|label| label.starts_with("Archive"))
        })
        .await,
        "the offer to clear the list never came as a toast: {:?}",
        window.toast_showing()
    );
    window.dismiss_toast();
    crate::settle();
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

/// `M` in a digest leaves the list of the message the digest's cursor is on:
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

        support::keys(&window, &["M"]);
        assert!(
            crate::settle_until(async || logged(&fixture).await.len() == 1).await,
            "`M` in the digest never reached storage"
        );
        let activations = logged(&fixture).await;
        assert_eq!(activations[0].account_id, fixture.account.id);
        assert_eq!(
            activations[0].list_identifier, "ledger.test",
            "no List-Id on this message, so the sender's domain is the list"
        );
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said.starts_with("Unsubscribed from "))
            })
            .await,
            "the window did not say what it left: {:?}",
            window.toast_showing()
        );
        assert!(
            window
                .toast_showing()
                .is_some_and(|said| !said.starts_with("Unsubscribed from ledger.test")),
            "the confirmation names the sender's address, not the domain the log keeps: {:?}",
            window.toast_showing()
        );

        ignore_the_offer(&window).await;
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        privacy_lists_an_activation(&window).await;
    });
}

/// Personal mail opens with no unsubscribe band, and `M` still leaves the
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

        support::keys(&window, &["M"]);
        assert!(
            crate::settle_until(async || logged(&fixture).await.len() == 1).await,
            "`M` on a message with no band did nothing"
        );
        assert_eq!(
            logged(&fixture).await[0].list_identifier,
            "friends.example.net"
        );
    });
}

/// The open message's dialog has finished opening, so a toast raised now is
/// drawn over it (a toast raised during the opening is under the dialog).
async fn the_dialog_is_open(reading: &postio_gtk::open::OpenMessage) {
    use gtk::prelude::*;
    assert!(
        crate::settle_until(async || reading.is_open() && reading.dialog().is_mapped()).await,
        "the message never finished opening"
    );
    crate::settle_for(std::time::Duration::from_millis(800)).await;
}

/// What the toast offers for the newsletter filed twice: the confirmation
/// naming the sender's exact address, the count, and one button.
async fn the_offer_is_up(window: &postio_gtk::window::FocusWindow, button: &str) {
    assert!(
        crate::settle_until(async || {
            window
                .toast()
                .and_then(|toast| toast.button_label())
                .is_some_and(|label| label == button)
        })
        .await,
        "`M` offered no `{button}` toast (toast: {:?})",
        window.toast_showing()
    );
    if let Some(reading) = window.reading().filter(|reading| reading.is_open()) {
        use gtk::prelude::*;
        assert!(
            window
                .toast_host()
                .is_some_and(|host| host.is_ancestor(&reading.dialog())),
            "the offer is drawn under the open message, out of reach"
        );
    }
    let said = window.toast_showing().expect("the toast's words");
    assert!(
        said.starts_with("Unsubscribed from weekly@news.example.org"),
        "the confirmation names the sender's address, not the domain: {said}"
    );
    assert!(said.contains("2 messages"), "it counts them: {said}");
}

async fn inbox_is_empty(fixture: &Fixture) -> bool {
    let connection = fixture.database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .page(&postio_storage::repository::ListQuery::mailbox(
            fixture.inbox,
        ))
        .await
        .expect("a page")
        .is_empty()
}

/// After `M` the window offers to clear what the sender left (#1952): one
/// toast, one button, never a dialog. Ignoring it or pressing Esc keeps
/// everything; the button archives the lot as one undoable command.
pub fn leaving_a_list_offers_one_action_on_a_toast_and_ignoring_it_keeps_the_mail() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file_the_newsletter(&fixture).await;
        file_the_newsletter(&fixture).await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        the_dialog_is_open(&reading).await;

        support::keys(&window, &["M"]);
        the_offer_is_up(&window, "Archive 2").await;

        // Esc declines: nothing is archived, and nothing is announced.
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        window.dismiss_toast();
        // The dismissed toast animates out, still mapped, for a moment: the
        // next offer's button is the one to press, not the leaving one. Wait
        // for it to be gone, not for a time a loaded machine can outrun.
        assert!(
            crate::settle_until(async || {
                !support::descendants(&window).iter().any(|widget| {
                    use gtk::prelude::*;
                    widget.is::<gtk::Button>()
                        && widget.is_mapped()
                        && support::texts(widget)
                            .iter()
                            .any(|text| text == "Archive 2")
                })
            })
            .await,
            "the declined offer's button never left the window"
        );
        assert!(!inbox_is_empty(&fixture).await, "declining archived them");
        assert_ne!(
            window.toast_showing().as_deref(),
            Some("Archived 2 messages"),
            "declining archived them"
        );

        // Again, and take it up: the button does it as one command.
        support::keys(&window, &["Return"]);
        let reading = window.reading().expect("Return opened the message");
        the_dialog_is_open(&reading).await;
        support::keys(&window, &["M"]);
        the_offer_is_up(&window, "Archive 2").await;
        let archive = support::button_labelled(&window, "Archive 2");
        support::click(&window, &archive, 1);
        assert!(
            crate::settle_until(async || inbox_is_empty(&fixture).await).await,
            "the button archived nothing (toast {:?})",
            window.toast_showing()
        );
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said.starts_with("Archived 2 messages"))
            })
            .await,
            "the archive did not say it was one undoable command: {:?}",
            window.toast_showing()
        );
    });
}

/// `[focus] after_unsubscribe = "delete"` makes the one action Delete.
pub fn the_setting_makes_the_one_action_delete() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file_the_newsletter(&fixture).await;
        file_the_newsletter(&fixture).await;
        let (window, _client) = fixture.open().await;
        window.set_focus_config(postio_config::FocusConfig {
            after_unsubscribe: postio_config::AfterUnsubscribe::Delete,
            ..postio_config::FocusConfig::default()
        });
        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        the_dialog_is_open(&reading).await;

        support::keys(&window, &["M"]);
        the_offer_is_up(&window, "Delete 2").await;
        let delete = support::button_labelled(&window, "Delete 2");
        support::click(&window, &delete, 1);
        assert!(
            crate::settle_until(async || inbox_is_empty(&fixture).await).await,
            "the button deleted nothing (toast {:?})",
            window.toast_showing()
        );
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said.starts_with("Deleted 2 messages"))
            })
            .await,
            "{:?}",
            window.toast_showing()
        );
    });
}

/// The cap drawn on the toast's button, if it has one.
fn caps_on_the_toast(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    use gtk::prelude::*;
    let Some(host) = window.toast_host() else {
        return Vec::new();
    };
    support::descendants(&host)
        .iter()
        .filter(|widget| widget.type_().name() == "AdwToastWidget")
        .flat_map(support::descendants)
        .filter(|widget| widget.has_css_class("postio-keyhint") && widget.is_mapped())
        .filter_map(|widget| {
            widget
                .downcast_ref::<gtk::Label>()
                .map(|l| l.text().to_string())
        })
        .collect()
}

/// The offer's one action has a key (#1953): `C` takes it up from the
/// keyboard exactly as the button does, the button shows the key, and the
/// same key with nothing on offer says so rather than doing nothing.
pub fn the_offer_is_taken_up_from_the_keyboard() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file_the_newsletter(&fixture).await;
        file_the_newsletter(&fixture).await;
        let (window, _client) = fixture.open().await;

        // Nothing on offer: the key answers, in words.
        support::keys(&window, &["C"]);
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said == postio_ui::unsubscribe::NO_OFFER)
            })
            .await,
            "`C` with no offer on screen said nothing: {:?}",
            window.toast_showing()
        );
        assert!(!inbox_is_empty(&fixture).await);

        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        the_dialog_is_open(&reading).await;
        support::keys(&window, &["M"]);
        the_offer_is_up(&window, "Archive 2").await;
        assert!(
            crate::settle_until(async || caps_on_the_toast(&window) == ["C"]).await,
            "the button does not show its key: {:?}",
            caps_on_the_toast(&window)
        );

        support::keys(&window, &["C"]);
        assert!(
            crate::settle_until(async || inbox_is_empty(&fixture).await).await,
            "`C` archived nothing (toast {:?})",
            window.toast_showing()
        );
        assert!(
            crate::settle_until(async || {
                window
                    .toast()
                    .is_some_and(|toast| toast.title().is_some_and(|t| t.starts_with("Archived 2")))
            })
            .await,
            "the archive did not say it could be undone: {:?}",
            window.toast_showing()
        );
        // Once taken the offer is spent: a second press has nothing to take.
        support::keys(&window, &["C"]);
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said == postio_ui::unsubscribe::NO_OFFER)
            })
            .await,
            "{:?}",
            window.toast_showing()
        );
    });
}

/// An offer the message dialog opens over is gone, not hidden: `C` must not
/// archive a sender's mail on the strength of a toast nobody can see.
pub fn opening_a_message_ends_the_offer_it_would_hide() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        file_the_newsletter(&fixture).await;
        file_the_newsletter(&fixture).await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["j", "Return"]);
        let reading = window.reading().expect("Return opened the message");
        the_dialog_is_open(&reading).await;
        support::keys(&window, &["M"]);
        the_offer_is_up(&window, "Archive 2").await;
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        crate::settle();
        support::keys(&window, &["Return"]);
        the_dialog_is_open(&reading).await;
        support::keys(&window, &["C"]);
        assert!(
            crate::settle_until(async || {
                window
                    .toast_showing()
                    .is_some_and(|said| said == postio_ui::unsubscribe::NO_OFFER)
            })
            .await,
            "the hidden offer answered a key: {:?}",
            window.toast_showing()
        );
        assert!(
            !inbox_is_empty(&fixture).await,
            "it archived under the dialog"
        );
    });
}
