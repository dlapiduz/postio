//! One store, either desktop app (US11 scenario 2, the archive half, T064):
//! what Focus archives, the classic app sees archived -- at once, through
//! its own client of the same host.
//!
//! The classic app's window is `postio-gtk`'s, which Focus may not depend
//! on even for a test (`check-crate-boundaries.py`), so its side is the
//! client it reads through: `ClientKind::Gtk`, connected to the host Focus
//! opened, reading and listening as that window does.

use postio_client::protocol::ClientKind;
use postio_model::listing::MailStore as _;
use postio_model::{ListScope, MailboxRole};

use crate::support::{self, Fixture};

pub fn what_focus_archives_the_classic_app_sees_archived() {
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
        let classic = session.host().connect(ClientKind::Gtk);
        let heard = classic.events();
        support::keep(session);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Budget"]).await,
            "the inbox never reached the screen"
        );
        let folders = classic
            .mailboxes(fixture.account.id)
            .await
            .expect("the classic app reads the folders");
        let archive = folders
            .iter()
            .find(|folder| folder.role == MailboxRole::Archive)
            .expect("an Archive folder")
            .id;
        assert_eq!(
            classic.list_count(ListScope::Mailbox(archive)).await,
            Ok(0),
            "nothing archived yet"
        );

        support::keys(&window, &["j", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).is_empty()).await,
            "Focus did not archive it"
        );

        // The classic app hears it leave the inbox, as its window would.
        let mut removed = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !removed && std::time::Instant::now() < deadline {
            crate::settle();
            while let Ok(envelope) = heard.try_recv() {
                if let postio_core::Event::MessagesRemoved {
                    mailbox, messages, ..
                } = envelope.event
                {
                    removed |= mailbox == fixture.inbox && messages.contains(&message);
                }
            }
        }
        assert!(
            removed,
            "the classic app was not told the message left the inbox"
        );
        assert_eq!(
            classic.list_count(ListScope::Mailbox(archive)).await,
            Ok(1),
            "the classic app lists it in Archive"
        );
        assert_eq!(
            classic.list_count(ListScope::Mailbox(fixture.inbox)).await,
            Ok(0),
            "and not in the inbox"
        );
    });
}
