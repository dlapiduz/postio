use adw::prelude::*;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_model::{Draft, EmailAddress};

use crate::compose::field;
use crate::support::{self, Fixture};

async fn both_apps() -> (Fixture, postio_gtk::window::FocusWindow, Client) {
    let fixture = Fixture::empty().await;
    {
        let connection = fixture.database.connect().await.expect("a connection");
        postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
    }
    fixture.file_from_ada("Budget", "Numbers.", 5).await;
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    let session =
        postio_gtk::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
    let terminal = session.host().connect(ClientKind::Tui);
    support::keep(session);
    assert!(
        crate::settle_until(async || support::subjects(&window) == ["Budget"]).await,
        "the inbox never reached the screen"
    );
    assert!(
        crate::settle_until(async || window.composer().is_some()).await,
        "Focus mounted no composer"
    );
    (fixture, window, terminal)
}

pub fn g_t_lists_drafts_and_enter_opens_one_to_edit() {
    crate::gtk_case(async {
        let (fixture, window, terminal) = both_apps().await;
        let mut kept = Draft::new(fixture.account.id);
        kept.to = vec![EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net")];
        kept.subject = "Harbor notes".to_owned();
        kept.body.text = Some("Two comments on the headers.".to_owned());
        terminal
            .save_draft(1, kept)
            .await
            .expect("the terminal saves its draft");

        support::keys(&window, &["g", "d"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Harbor notes"]).await,
            "g d did not list Drafts: {:?} under {:?}",
            support::subjects(&window),
            window.place_name()
        );
        assert_eq!(window.place_name(), "Drafts");

        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "Enter on a draft did not open it to edit"
        );
        assert!(
            window.reading().is_none_or(|reading| !reading.is_open()),
            "a draft opens in the composer, not the reader"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        assert!(
            crate::settle_until(async || {
                field(&dialog, "Subject").as_deref() == Some("Harbor notes")
            })
            .await,
            "the draft's subject: {:?}",
            field(&dialog, "Subject")
        );
        let composer = window.composer().expect("the composer");
        assert!(
            crate::settle_until(async || {
                composer.focused_field() == Some(postio_widgets::composer::Field::Body)
            })
            .await,
            "the keyboard is in {:?}, not the body of a draft already addressed",
            composer.focused_field()
        );
        assert!(
            crate::settle_until(async || {
                composer.test_body_eval(
                    "(() => { const s = window.getSelection(); \
                       if (!s.rangeCount) return 'none'; \
                       const end = document.createRange(); \
                       end.selectNodeContents(document.body); end.collapse(false); \
                       return s.getRangeAt(0).compareBoundaryPoints(Range.START_TO_START, end) === 0 \
                         ? 'end' : 'elsewhere'; })()",
                ) == "end"
            })
            .await,
            "the caret is not at the end of the resumed body: {}",
            composer.test_body_eval(
                "(() => { const s = window.getSelection(); \
                   return s.rangeCount ? s.anchorNode.nodeName + ':' + s.anchorOffset \
                     + ' of ' + document.body.innerText.length : 'none'; })()"
            )
        );
    });
}
