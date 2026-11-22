use adw::prelude::*;
use postio_model::{EmailAddress, Message, MessageId};
use postio_storage::repository::{MessageRepository, StoredBody, ThreadingRepository};

use crate::support::{self, Fixture};

pub const ME: &str = "test@example.com";

impl Fixture {
    pub async fn harbor_thread(&self) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(
            self.account.id,
            self.inbox,
            support::now() - chrono::Duration::minutes(47),
        );
        message.from = vec![EmailAddress::new(Some("Lena Park"), "lena@example.org")];
        message.to = vec![
            EmailAddress::new(Some("Test User"), ME),
            EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net"),
        ];
        message.cc = vec![
            EmailAddress::new(Some("Grace Oyelaran"), "grace@example.org"),
            EmailAddress::new(None::<String>, "harbor-api@example.org"),
        ];
        message.subject = Some("Harbor API draft v3".to_owned());
        message.preview = Some("v3 is up for review.".to_owned());
        message.rfc_message_id = Some(postio_model::RfcMessageId::new("<harbor.v3@example.org>"));
        let messages = MessageRepository::new(&connection);
        let id = messages.create(&mut message).await.expect("a message");
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(
                        "v3 is up for review.\n\nThe rate-limit headers moved to the appendix."
                            .to_owned(),
                    ),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("its body");
        ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        id
    }
}

pub fn field(root: &impl IsA<gtk::Widget>, name: &str) -> Option<String> {
    let row = support::with_class(root, "postio-compose-row")
        .into_iter()
        .filter(|row| row.is_visible())
        .find(|row| {
            row.first_child()
                .and_downcast::<gtk::Label>()
                .is_some_and(|label| label.text() == name)
        })?;
    let mut shown: Vec<String> = support::with_class(&row, "postio-recipient-chip")
        .iter()
        .map(|chip| {
            let words = |class: &str| {
                support::with_class(chip, class)
                    .first()
                    .and_then(|label| label.downcast_ref::<gtk::Label>().map(|label| label.text()))
                    .map(|text| text.to_string())
            };
            match (
                words("postio-recipient-chip-name"),
                words("postio-recipient-chip-address"),
            ) {
                (Some(name), Some(address)) => format!("{name} <{address}>"),
                (None, Some(address)) => address,
                _ => String::new(),
            }
        })
        .collect();
    let mut stack = vec![row];
    while let Some(widget) = stack.pop() {
        if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
            if !entry.text().is_empty() {
                shown.push(entry.text().to_string());
            }
            break;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    Some(shown.join(", "))
}

pub fn reply_all_from_the_open_message_answers_it_and_esc_returns_to_it() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "Numbers.", 5).await;
        let message = fixture.harbor_thread().await;
        fixture.label(message, &["Harbor"]).await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 2).await;
        support::keys(&window, &["j", "j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        assert_eq!(reading.title(), "Harbor API draft v3");

        support::keys(&window, &["a"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "a in the open message opened no composer"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        assert!(
            crate::settle_until(async || {
                field(&dialog, "Subject").as_deref() == Some("Re: Harbor API draft v3")
            })
            .await,
            "the reply answers the message on screen: {:?}",
            field(&dialog, "Subject")
        );
        assert_eq!(
            field(&dialog, "To").as_deref(),
            Some("Lena Park <lena@example.org>")
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Esc did not close the composer"
        );
        assert!(
            reading.is_open() && reading.title() == "Harbor API draft v3",
            "Esc returns to the message the reply was written from"
        );
    });
}

pub fn a_send_says_it_was_queued_and_undo_takes_it_back() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.harbor_thread().await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::keys(&window, &["j", "r"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "r opened no composer"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "mod+Return did not close the composer"
        );
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "a send said nothing"
        );
        let toast = window.toast().expect("the toast");
        assert_eq!(toast.button_label().as_deref(), Some("Undo"));
        assert!(
            toast.title().unwrap_or_default().contains("queued"),
            "the toast says {:?}",
            toast.title()
        );

        support::press(&window, "z", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "Undo did not put the draft back in the composer"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        assert_eq!(
            field(&dialog, "Subject").as_deref(),
            Some("Re: Harbor API draft v3")
        );
    });
}

fn heading_toggle(composer: &postio_widgets::composer::Composer) -> gtk::ToggleButton {
    support::with_class(composer, "postio-toolbar-heading")
        .into_iter()
        .next()
        .and_downcast::<gtk::ToggleButton>()
        .expect("the toolbar carries a Heading toggle")
}

fn block_drawn(composer: &postio_widgets::composer::Composer, nth: usize) -> String {
    composer.test_body_eval(&format!(
        "(() => {{ const block = document.body.children[{nth}]; \
           if (!block) return 'none'; \
           const size = (el) => parseFloat(getComputedStyle(el).fontSize); \
           return block.tagName + ' ' + block.textContent + ' ' \
             + (size(block) > size(document.body) ? 'larger' : 'body-sized'); }})()"
    ))
}

pub fn the_heading_key_and_its_markdown_make_a_heading_the_toolbar_shows() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.composer().is_some_and(|c| c.is_open())).await,
            "c opened no composer"
        );
        let composer = window.composer().expect("the composer");
        let toggle = heading_toggle(&composer);
        assert!(toggle.icon_name().is_some(), "the Heading toggle is drawn");
        composer.test_focus_field(postio_widgets::composer::Field::Body);
        composer.test_set_body("Agenda");
        assert!(
            composer.marks_lit().is_empty(),
            "a paragraph is not a heading"
        );

        let heading = gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK;
        support::press(&window, "1", heading);
        assert!(
            crate::settle_until(async || block_drawn(&composer, 0) == "H1 Agenda larger").await,
            "ctrl+alt+1 made no heading of the line: {}",
            block_drawn(&composer, 0)
        );
        assert!(
            crate::settle_until(async || composer.marks_lit() == ["heading_1"]).await,
            "the toolbar shows {:?} over a heading",
            composer.marks_lit()
        );
        support::press(&window, "1", heading);
        assert!(
            crate::settle_until(async || block_drawn(&composer, 0) == "P Agenda body-sized").await,
            "a second ctrl+alt+1 left the heading: {}",
            block_drawn(&composer, 0)
        );
        assert!(
            crate::settle_until(async || composer.marks_lit().is_empty()).await,
            "the toolbar shows {:?} over a paragraph",
            composer.marks_lit()
        );

        composer.test_body_eval(
            "(() => { const sel = window.getSelection(); \
               sel.selectAllChildren(document.body); sel.collapseToEnd(); \
               document.execCommand('insertParagraph'); return 'broke'; })()",
        );
        for character in "# Next".chars() {
            composer.test_body_eval(&format!(
                "(() => {{ document.execCommand('insertText', false, '{character}'); \
                   return 'typed'; }})()"
            ));
        }
        assert!(
            crate::settle_until(async || block_drawn(&composer, 1) == "H1 Next larger").await,
            "`# ` made no heading of the line: {}",
            block_drawn(&composer, 1)
        );
        assert!(
            crate::settle_until(async || composer.marks_lit() == ["heading_1"]).await,
            "the toolbar shows {:?} over the typed heading",
            composer.marks_lit()
        );
        assert_eq!(
            block_drawn(&composer, 0),
            "P Agenda body-sized",
            "the line above was touched"
        );
        assert_eq!(
            composer.test_body_eval("String(document.body.children.length)"),
            "2",
            "the heading left lines behind it"
        );
    });
}
