use gtk::prelude::*;
use postio_core::Event;

use crate::support::{self, Fixture};

pub fn a_body_that_lands_while_its_message_is_open_is_drawn_in_place() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Tide gate", "", 5)
            .await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Older", "", 50)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Return opened the message");
        assert_eq!(reading.shown(), Some(message));
        assert!(
            crate::settle_until(async || reading.body_text().contains("Downloading")).await,
            "the plate never drew, so this proves nothing: {:?}",
            reading.body_text()
        );
        // The dialog hands the keyboard to its content when it maps, a turn after the plate draws;
        // a baseline read before then is "nothing".
        assert!(
            crate::settle_until(async || gtk::prelude::GtkWindowExt::focus(&window).is_some())
                .await,
            "the message opened and the keyboard never went into it"
        );
        crate::settle();
        let cursor = window.cursor_row().map(|row| row.id());
        let focus = support::focus_path(&window);

        sink.emit(Event::BodyLoaded {
            account: fixture.account.id,
            message: postio_model::MessageId::new(message.get() + 1),
            sections: postio_core::event::Sections::Text,
        });
        fixture
            .write_body(message, "The tide gate opens at nine.")
            .await;
        assert!(
            sink.emit(Event::BodyLoaded {
                account: fixture.account.id,
                message,
                sections: postio_core::event::Sections::Text,
            }),
            "the hub took the event"
        );
        assert!(
            crate::settle_until(async || reading
                .body_text()
                .contains("The tide gate opens at nine."))
            .await,
            "the body landed and the reader still says: {:?}",
            reading.body_text()
        );
        assert!(reading.is_open(), "the message stayed open");
        assert_eq!(reading.shown(), Some(message));
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            cursor,
            "the list moved"
        );
        assert_eq!(support::focus_path(&window), focus, "the focus moved");
    });
}

pub fn a_body_the_server_refuses_stops_saying_it_is_downloading() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Tide gate", "", 5)
            .await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Older", "", 50)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Return opened the message");
        assert!(
            crate::settle_until(async || reading.body_text().contains("Downloading")).await,
            "the plate never drew: {:?}",
            reading.body_text()
        );

        fixture.refuse_body(message).await;
        assert!(sink.emit(Event::BodyFailed {
            account: fixture.account.id,
            message,
            sections: postio_core::event::Sections::Text,
        }));
        assert!(
            crate::settle_until(async || reading.body_text().contains("could not be downloaded"))
                .await,
            "the refusal never reached the open message: {:?}",
            reading.body_text()
        );
        assert!(reading.is_open());
    });
}
