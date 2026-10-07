//! Compose and reply in Focus (US3, screens 05 and 06): the classic app's
//! composer, in a dialog over the list.
//!
//! What is asserted is what the dialog shows -- the heading, the fields as
//! their entries read, the Labels row, the quote in the body -- never what
//! the composer was handed.

use adw::prelude::*;
use postio_model::{EmailAddress, Message, MessageId};
use postio_storage::repository::{MessageRepository, StoredBody, ThreadingRepository};

use crate::support::{self, Fixture};

/// The user's own address in the fixture's account: `reply_all` leaves it
/// out of the recipients.
pub const ME: &str = "test@example.com";

impl Fixture {
    /// File Lena's "Harbor API draft v3" into the inbox, to the user and
    /// Ben, copying Grace and the Harbor list, with a body to quote.
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

/// What the composer field labelled `name` ("To", "Cc", "Subject") shows:
/// each recipient chip as "Name <address>" (T079), then what its entry
/// holds, joined as a list is written.
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

/// US3 scenario 1 (T078): `E` on a conversation opens the composer, in its
/// dialog over the list, with every recipient from the thread, a "Re:"
/// subject, the thread's labels marked as its, and the quote folded; `Esc`
/// returns to where the person was.
pub fn reply_all_starts_with_every_recipient_re_the_labels_and_a_folded_quote() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let message = fixture.harbor_thread().await;
        fixture.label(message, &["Harbor"]).await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["j"]);
        let cursor = window.cursor_row().map(|row| row.id());
        assert_eq!(cursor, Some(message), "the cursor is on the thread");

        support::keys(&window, &["E"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "E opened no composer over the list"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        // The message dialog's size rule, from the window (T221).
        assert_eq!(
            (dialog.content_width(), dialog.content_height()),
            (
                postio_ui::focus_dialog::dialog_width(window.width()),
                postio_ui::focus_dialog::dialog_height(window.height())
            ),
            "the message dialog's frame"
        );
        assert!(
            crate::settle_until(async || {
                support::texts(&dialog)
                    .iter()
                    .any(|text| text == "Reply to all")
            })
            .await,
            "the header names the composition: {:?}",
            support::texts(&dialog)
        );
        assert_eq!(
            field(&dialog, "To").as_deref(),
            Some("Lena Park <lena@example.org>"),
            "the sender, as screen 06 draws it"
        );
        assert_eq!(
            field(&dialog, "Cc").as_deref(),
            Some(
                "Ben Adeyemi <ben@example.net>, Grace Oyelaran <grace@example.org>, \
                 harbor-api@example.org"
            ),
            "everyone else it went to, the user left out"
        );
        assert_eq!(
            field(&dialog, "Subject").as_deref(),
            Some("Re: Harbor API draft v3")
        );
        assert!(
            crate::settle_until(async || {
                let labels = support::with_class(&dialog, "focus-compose-labels");
                labels.len() == 1 && {
                    let said = support::texts(&labels[0]);
                    said.contains(&"Harbor".to_owned())
                        && said.contains(&"from the thread".to_owned())
                }
            })
            .await,
            "the thread's labels, marked as the thread's: {:?}",
            support::with_class(&dialog, "focus-compose-labels")
                .first()
                .map(support::texts)
        );
        let composer = window.composer().expect("the composer");
        let quote = || {
            composer.test_body_eval(
                "(() => { const fold = document.querySelector('details.postio-quote'); \
                 if (!fold) return 'no fold'; \
                 return fold.open ? 'open' : 'folded'; })()",
            )
        };
        // The editing surface loads the reply on its own time.
        assert!(
            crate::settle_until(async || quote() == "folded").await,
            "the quote is folded under the draft: {}",
            quote()
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Esc did not close the composer"
        );
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            cursor,
            "Esc returns to where the person was"
        );
    });
}

/// US3 scenario 1 from the open message (T078's dialog half, screen 04's
/// toolbar): `E` in the message dialog answers the message on screen, and
/// `Esc` returns to that message, still open.
pub fn reply_all_from_the_open_message_answers_it_and_esc_returns_to_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let message = fixture.harbor_thread().await;
        fixture.label(message, &["Harbor"]).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        // The cursor on Budget, then the Harbor thread opened: what is on
        // screen is what a reply answers.
        support::keys(&window, &["j", "j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        assert_eq!(reading.title(), "Harbor API draft v3");

        support::keys(&window, &["E"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "E in the open message opened no composer"
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

/// #1752: a send says it was queued, offers Undo, and Undo takes it
/// off the queue and puts the draft back in the composer.
pub fn a_send_says_it_was_queued_and_undo_takes_it_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.harbor_thread().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j", "e"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "e opened no composer"
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

/// What the row holding the keyboard says: the first line of the focused list
/// item, as the list draws it.
fn focused_row_subject(window: &postio_gtk::window::FocusWindow) -> Option<String> {
    let focus = gtk::prelude::GtkWindowExt::focus(window)?;
    let row = focus.first_child()?.downcast::<postio_gtk::list::RowWidget>().ok()?;
    let item = row.item()?;
    item.as_conversation()
        .and_then(|row| row.summary.representative.subject.clone())
}

/// Leaving the composer in the reading pane returns the keyboard to the row
/// the cursor is on, not to the list's first row.
pub fn escape_from_the_pane_composer_returns_the_keyboard_to_the_cursor_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::deliver(&window, "F8");
        support::keys(&window, &["j", "j", "j"]);
        let cursor = support::subjects(&window);
        let at = window.pane().expect("the list").cursor().selected() as usize;
        let want = cursor[at].clone();
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
            "Return opened nothing beside the list"
        );
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.composer().is_some_and(|c| c.is_open())).await,
            "c opened no composer"
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.composer().is_some_and(|c| !c.is_open())).await,
            "Escape did not close the composer"
        );
        assert!(
            crate::settle_until(async || focused_row_subject(&window).as_deref() == Some(&want))
                .await,
            "the keyboard is on {:?}, the cursor on {want:?}",
            focused_row_subject(&window)
        );
    });
}
