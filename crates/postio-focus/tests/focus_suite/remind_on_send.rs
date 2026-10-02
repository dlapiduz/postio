//! "Remind if no reply", set in the composer (US3 scenario 5, T096): `mod+h`
//! in the composer opens the remind picker at its action row, the row says
//! the time chosen, and sending the message sets that reminder on the
//! conversation it is filed into. What the reminder does when it falls due
//! is US5's, proven by `surfaced.rs` and the host's due timer.

use chrono::Utc;
use postio_account::backend::{MailBackend as _, MockBackend, MockMailbox};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore as _};
use postio_smtp::transport::ScriptedConnector;

use crate::offline_send::{accepting, drain};
use crate::support::{self, Fixture};

pub fn mod_h_in_the_composer_sets_a_reminder_that_sending_keeps() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
            postio_storage::test_support::mailbox(&connection, &fixture.account, "Sent").await;
            let mut identity =
                postio_model::Identity::new(fixture.account.id, fixture.account.address.clone());
            identity.display_name = "Test User".to_owned();
            identity.is_default = true;
            postio_storage::repository::IdentityRepository::new(&connection)
                .create(&mut identity)
                .await
                .expect("an identity");
        }
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        );
        let blobs = session.host().wiring().blobs.clone();
        let client = session.client().clone();
        support::keep(session);
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "Focus mounted no composer"
        );
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        let composer = window.composer().expect("the composer");
        composer.test_set_to("Lena Park <lena@example.org>, ");
        composer.test_set_subject("Harbor API draft v3");
        // The composer's verbs are its action row (T221).
        let verbs = || {
            support::texts(&support::only(
                &window.compose_dialog().expect("the dialog"),
                "focus-compose-actions",
            ))
        };
        assert!(
            crate::settle_until(async || verbs().iter().any(|text| text == "Attach")).await,
            "the action row never drew: {:?}",
            verbs()
        );
        assert!(
            verbs().iter().any(|text| text == "Remind"),
            "the action row offers the reminder: {:?}",
            verbs()
        );

        // mod+h: the remind picker, and `1` for its first preset.
        support::press(&window, "h", gtk::gdk::ModifierType::CONTROL_MASK);
        let picker = window
            .open_picker()
            .expect("mod+h opened the remind picker in the composer");
        assert!(crate::settle_until(async || picker.is_shown()).await);
        assert!(
            picker
                .texts()
                .iter()
                .any(|line| line == "Remind me if no one replies by"),
            "{:?}",
            picker.texts()
        );
        let (_, due) = postio_ui::schedule::remind_presets(chrono::Local::now())[0];
        support::press(&window, "1", gtk::gdk::ModifierType::empty());
        assert!(!picker.is_open(), "choosing closed the picker");
        let said = format!("Remind \u{b7} {}", due.format("%a %-d %b"));
        assert!(
            crate::settle_until(async || verbs().contains(&said)).await,
            "the action row does not say the reminder: {:?}",
            verbs()
        );
        assert!(
            window.compose_dialog().is_some(),
            "the composer stays open while the reminder is chosen"
        );

        // Sent, and delivered once the link is back: the conversation the
        // message is filed into has the reminder, due then.
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Ctrl+Return did not send: {}",
            composer.status()
        );
        let outbox = || async {
            use postio_model::listing::MailStore as _;
            client
                .list_count(postio_model::ListScope::Outbox(fixture.account.id))
                .await
                .expect("the Outbox counts")
        };
        assert!(
            crate::settle_until(async || outbox().await == 1).await,
            "the message is not in the Outbox"
        );
        let backend = MockBackend::builder()
            .mailbox(MockMailbox::new("Sent"))
            .build();
        backend.connect().await.expect("the mock connects");
        let secrets = std::sync::Arc::new(MemorySecretStore::new());
        secrets
            .store(
                &AccountKey::new(fixture.account.address.address.clone()),
                &Password::new("password"),
            )
            .await
            .expect("the password");
        let connector = ScriptedConnector::new(accepting());
        // After the undo-send window, as the drainer would find it.
        let later = Utc::now() + chrono::Duration::hours(1);
        let report = drain(&fixture, &backend, &secrets, &blobs, &connector, later).await;
        assert_eq!(report.applied, 1, "it left: {report:?}");

        let connection = fixture.database.connect().await.expect("a connection");
        let reminders = postio_storage::repository::ReminderRepository::new(&connection);
        let standing = reminders.due(due.to_utc()).await.expect("a read");
        assert_eq!(
            standing.len(),
            1,
            "sending set one reminder, due when the composer said: {standing:?}"
        );
        assert_eq!(standing[0].due_at, due.to_utc());
    });
}
