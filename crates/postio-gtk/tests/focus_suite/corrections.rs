//! Focus's corrections reach the engine (the commands lane's verbs):
//! dismissing a sender's question three times turns that marker off for the
//! sender, kept on the person they are (ADR 0048).

use postio_core::{Command, MessageTarget};

use crate::support::{self, Fixture};

pub fn three_dismissals_write_a_stop_marker_to_focus_s_config() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "").expect("an empty config");
        let fixture = Fixture::empty().await;
        let mut asked = Vec::new();
        for (subject, minutes) in [("One", 10), ("Two", 20), ("Three", 30)] {
            let (message, _) = fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    subject,
                    "A question.",
                    minutes,
                )
                .await;
            fixture.ask(message, "Can you approve it?").await;
            asked.push(message);
        }
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_gtk::startup::adopt_at(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
            Some(&path),
        );
        for message in asked {
            session
                .client()
                .send(Command::DismissMarker {
                    target: MessageTarget::Messages(vec![message]),
                    dismissed: true,
                })
                .await
                .expect("the host takes it");
        }
        // The correction is the person's (ADR 0048): the engine reads it, and
        // the file is not written.
        let ada = postio_model::EmailAddress::new(None::<String>, "ada@example.com");
        let stopped = || {
            session
                .host()
                .wiring()
                .focus
                .config()
                .is_some_and(|focus| focus.filter.stops(&ada, "question"))
        };
        assert!(
            crate::settle_until(async || stopped()).await,
            "no stop marker reached Focus's engine"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "",
            "an address is the person's, not the file's"
        );
        support::keep(session);
        drop(directory);
    });
}
