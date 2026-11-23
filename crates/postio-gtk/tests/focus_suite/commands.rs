use chrono::Duration;
use gtk::gdk;
use postio_core::{Command, MessageTarget};
use postio_model::{Flag, MessageId};
use postio_storage::repository::MessageRepository;

use crate::support::{self, Fixture};

type Window = postio_gtk::window::FocusWindow;

async fn stored(fixture: &Fixture, message: MessageId) -> postio_model::Message {
    let connection = fixture.database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .await
        .expect("a read")
        .expect("the message")
}

async fn showing(window: &Window, place: &str, subjects: &[&str]) {
    assert!(
        crate::settle_until(async || {
            window.place_name() == place && support::subjects(window) == subjects
        })
        .await,
        "expected {place:?} listing {subjects:?}; the window says {:?} listing {:?}",
        window.place_name(),
        support::subjects(window)
    );
}

pub async fn snooze(fixture: &Fixture, message: MessageId) {
    let connection = fixture.database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .snooze(&[message], chrono::Utc::now() + Duration::days(2))
        .await
        .expect("snoozed");
}

pub async fn flag(client: &postio_client::Client, message: MessageId) {
    client
        .send(Command::Flag {
            target: MessageTarget::Messages(vec![message]),
            flagged: Some(true),
        })
        .await
        .expect("flagged");
}

pub fn star_flags_the_row_and_undo_clears_it() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture.file_from_ada("Atlas budget", "x", 30).await;
        fixture
            .file(
                ("Tomas Reyes", "tomas@example.net"),
                "Staffing plan",
                "x",
                10,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let before = window.rows_on_screen();
        support::keys(&window, &["j", "j", "s"]);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "`s` did not flag it"
        );
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "no undo toast"
        );
        assert_eq!(
            window.rows_on_screen().len(),
            before.len(),
            "the row left the inbox"
        );
        assert!(
            !window.rows_on_screen()[1].contains("lagged"),
            "the row carries a flag mark: {:?}",
            window.rows_on_screen()
        );
        support::keys(&window, &["g", "s"]);
        showing(&window, "Flagged", &["Atlas budget"]).await;
        support::keys(&window, &["g", "i"]);
        showing(&window, "Inbox", &["Staffing plan", "Atlas budget"]).await;

        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "Undo did not clear the flag"
        );

        support::keys(&window, &["j", "j", "s"]);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await
        );
        support::keys(&window, &["s"]);
        assert!(
            crate::settle_until(async || !stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "a second `s` did not unflag it"
        );
    });
}
