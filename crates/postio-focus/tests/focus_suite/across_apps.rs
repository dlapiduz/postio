//! One store, every app (US11 scenario 2, the archive half, T064): what
//! Focus archives, another app sees archived -- at once, through its own
//! client of the same host.
//!
//! The other app is the terminal, by the client it reads through:
//! `ClientKind::Tui`, connected to the host Focus opened, reading and
//! listening as the terminal does.

use postio_client::protocol::ClientKind;
use postio_model::listing::MailStore as _;
use postio_model::{ListScope, MailboxRole};

use crate::support::{self, Fixture};

pub fn what_focus_archives_the_terminal_sees_archived() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        );
        let terminal = session.host().connect(ClientKind::Tui);
        let heard = terminal.events();
        support::keep(session);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Budget"]).await,
            "the inbox never reached the screen"
        );
        let folders = terminal
            .mailboxes(fixture.account.id)
            .await
            .expect("the terminal reads the folders");
        let archive = folders
            .iter()
            .find(|folder| folder.role == MailboxRole::Archive)
            .expect("an Archive folder")
            .id;
        assert_eq!(
            terminal.list_count(ListScope::Mailbox(archive)).await,
            Ok(0),
            "nothing archived yet"
        );

        support::keys(&window, &["j", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).is_empty()).await,
            "Focus did not archive it"
        );

        // The terminal hears it leave the inbox, as its list would.
        let removed = std::cell::Cell::new(false);
        let heard_it = crate::settle_until(async || {
            while let Ok(envelope) = heard.try_recv() {
                if let postio_core::Event::MessagesRemoved {
                    mailbox, messages, ..
                } = envelope.event
                    && mailbox == fixture.inbox
                    && messages.contains(&message)
                {
                    removed.set(true);
                }
            }
            removed.get()
        })
        .await;
        assert!(
            heard_it,
            "the terminal was not told the message left the inbox"
        );
        assert_eq!(
            terminal.list_count(ListScope::Mailbox(archive)).await,
            Ok(1),
            "the terminal lists it in Archive"
        );
        assert_eq!(
            terminal.list_count(ListScope::Mailbox(fixture.inbox)).await,
            Ok(0),
            "and not in the inbox"
        );
    });
}
