//! Focus's corrections reach `config.toml` (the commands lane's verbs):
//! dismissing a sender's question three times writes a stop marker to the
//! file Focus was opened with, because Focus tells its host where that file
//! is (`FocusSetup::with_config_path`).

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
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_focus::startup::adopt_at(
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
        let written = || std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            crate::settle_until(async || written().contains("ada@example.com")).await,
            "no stop marker was written to Focus's config: {:?}",
            written()
        );
        support::keep(session);
        drop(directory);
    });
}
