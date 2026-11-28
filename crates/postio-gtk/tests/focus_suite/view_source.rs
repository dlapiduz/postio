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
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::keys(&window, &["j", "V"]);
        let shows = |window: &postio_gtk::window::FocusWindow| {
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

        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(window.reading().is_some_and(|reading| reading.is_open()));
        support::keys(&window, &["V"]);
        assert!(
            crate::settle_until(async || shows(&window)).await,
            "V in the open message did not show its raw source"
        );
    });
}
