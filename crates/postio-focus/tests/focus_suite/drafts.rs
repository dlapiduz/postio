//! Drafts, across apps (US3 scenario 3, US11 scenario 3; T080, and T064's
//! draft half): `Esc` closes Focus's composer and keeps the draft locally,
//! and a draft left in Focus or the terminal opens for editing in the other,
//! over one store.
//!
//! The terminal's side is the client it reads and writes through --
//! `ClientKind::Tui`, connected to the host Focus opened (`across_apps.rs`
//! does the same). What it does with a Drafts row is what the terminal
//! does: list the Drafts folder, and open the draft behind the row.

use adw::prelude::*;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_model::listing::{ListPage, MailStore as _, PageRequest};
use postio_model::{Draft, EmailAddress, ListScope, MailboxId, MailboxRole, MessageId};

use crate::compose::field;
use crate::support::{self, Fixture};

/// The Drafts folder's id, as the terminal finds it.
async fn drafts_folder(terminal: &Client, fixture: &Fixture) -> MailboxId {
    terminal
        .mailboxes(fixture.account.id)
        .await
        .expect("the terminal reads the folders")
        .iter()
        .find(|folder| folder.role == MailboxRole::Drafts)
        .expect("a Drafts folder")
        .id
}

/// The rows in Drafts, as the terminal lists them: each message and its
/// subject.
async fn drafts_listed(terminal: &Client, drafts: MailboxId) -> Vec<(MessageId, String)> {
    match terminal
        .list_page(PageRequest {
            scope: ListScope::Mailbox(drafts),
            offset: 0,
            limit: 20,
        })
        .await
    {
        Ok(ListPage::Messages(page)) => page
            .rows
            .into_iter()
            .map(|row| (row.id, row.subject.unwrap_or_default()))
            .collect(),
        Ok(ListPage::Threads(page)) => page
            .rows
            .into_iter()
            .map(|row| {
                (
                    row.representative.id,
                    row.representative.subject.unwrap_or_default(),
                )
            })
            .collect(),
        Err(error) => panic!("the terminal could not list Drafts: {error}"),
    }
}

/// A fixture with a Drafts folder and one conversation, Focus open over it,
/// and the terminal's client beside it.
async fn both_apps() -> (Fixture, postio_focus::window::FocusWindow, Client) {
    let fixture = Fixture::empty().await;
    {
        let connection = fixture.database.connect().await.expect("a connection");
        postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
    }
    fixture
        .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
        .await;
    let window = postio_focus::window::FocusWindow::new(None);
    window.present();
    let session =
        postio_focus::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
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

/// US3 scenario 3, and US11 scenario 3 from Focus to the terminal.
pub fn escape_keeps_the_draft_and_the_terminal_opens_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, terminal) = both_apps().await;
        let drafts = drafts_folder(&terminal, &fixture).await;
        assert!(drafts_listed(&terminal, drafts).await.is_empty());

        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        let composer = window.composer().expect("the composer");
        composer.test_set_to("Lena Park <lena@example.org>, ");
        composer.test_set_subject("Q4 headcount");

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Esc did not close the composer"
        );

        // Kept locally: the terminal lists it in Drafts at once.
        let listed = std::cell::RefCell::new(Vec::new());
        assert!(
            crate::settle_until(async || {
                *listed.borrow_mut() = drafts_listed(&terminal, drafts).await;
                listed.borrow().len() == 1
            })
            .await,
            "Esc kept no draft: {:?}",
            listed.borrow()
        );
        let (row, subject) = listed.borrow()[0].clone();
        assert_eq!(subject, "Q4 headcount");

        // And opens it for editing, as the terminal does with the row.
        let draft = terminal
            .draft_behind(row)
            .await
            .expect("the terminal reads the draft")
            .expect("a local draft behind the row, to edit");
        assert_eq!(draft.subject, "Q4 headcount");
        assert_eq!(
            draft.to,
            [EmailAddress::new(Some("Lena Park"), "lena@example.org")]
        );
    });
}

/// US11 scenario 3, from the terminal to Focus: a draft the terminal
/// kept opens in Focus's composer, and closing it keeps the one draft.
pub fn a_draft_the_terminal_kept_opens_in_focus() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, terminal) = both_apps().await;
        let drafts = drafts_folder(&terminal, &fixture).await;

        // The terminal composer's autosave, through its client.
        let mut kept = Draft::new(fixture.account.id);
        kept.to = vec![EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net")];
        kept.subject = "Harbor notes".to_owned();
        kept.body.text = Some("Two comments on the headers.".to_owned());
        terminal
            .save_draft(1, kept)
            .await
            .expect("the terminal saves its draft");
        let listed = std::cell::RefCell::new(Vec::new());
        assert!(
            crate::settle_until(async || {
                *listed.borrow_mut() = drafts_listed(&terminal, drafts).await;
                listed.borrow().len() == 1
            })
            .await,
            "the terminal's draft never reached Drafts"
        );
        let (row, _) = listed.borrow()[0].clone();

        // What Focus's Drafts row does when opened.
        window.open_draft(row);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "the draft did not open in Focus's composer"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        assert_eq!(field(&dialog, "Subject").as_deref(), Some("Harbor notes"));
        assert_eq!(
            field(&dialog, "To").as_deref(),
            Some("Ben Adeyemi <ben@example.net>")
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Esc did not close the composer"
        );
        assert!(
            crate::settle_while(async || drafts_listed(&terminal, drafts).await.len() == 1).await,
            "closing a resumed draft must leave the one draft, not a second: {:?}",
            drafts_listed(&terminal, drafts).await
        );
    });
}

/// US11 scenario 3 by the keyboard (T080): `g t` lists Drafts, and `Enter`
/// on a draft opens it in the composer rather than in the reader.
pub fn g_t_lists_drafts_and_enter_opens_one_to_edit() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, terminal) = both_apps().await;
        let mut kept = Draft::new(fixture.account.id);
        kept.to = vec![EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net")];
        kept.subject = "Harbor notes".to_owned();
        kept.body.text = Some("Two comments on the headers.".to_owned());
        terminal
            .save_draft(1, kept)
            .await
            .expect("the terminal saves its draft");

        support::keys(&window, &["g", "t"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Harbor notes"]).await,
            "g t did not list Drafts: {:?} under {:?}",
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
    });
}
