//! `v` shows the raw source (US2 scenario 5, T068): the message as it came,
//! header lines first, from the list or from the open message -- read
//! through `Client::raw_source`, which fetches only when asked.

use gtk::gdk;

use crate::support::{self, Fixture};

const RAW: &[u8] = b"From: Ada Moreno <ada@example.com>\r\n\
To: you@example.com\r\n\
Subject: Budget\r\n\
X-Postio-Fixture: raw source\r\n\
\r\n\
The numbers are attached.\r\n";

pub fn v_shows_the_raw_message_from_the_list_and_from_the_open_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Budget",
                "The numbers.",
                5,
            )
            .await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j", "v"]);
        let shows = |window: &postio_focus::window::FocusWindow| {
            window.source_shown().is_some_and(|text| {
                text.contains("X-Postio-Fixture: raw source") && text.contains("Subject: Budget")
            })
        };
        assert!(
            crate::settle_until(async || shows(&window)).await,
            "v on the row did not show its raw source: {:?}",
            window.source_shown()
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.source_shown().is_none()).await,
            "Escape did not close the source"
        );

        let _ = window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
        assert!(window.reading().is_some_and(|reading| reading.is_open()));
        support::keys(&window, &["v"]);
        assert!(
            crate::settle_until(async || shows(&window)).await,
            "v in the open message did not show its raw source"
        );
    });
}
