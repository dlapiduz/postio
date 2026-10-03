//! Sending with no network (US3 scenario 7, FR-055; T082): the message is
//! in the Outbox the moment Send is pressed, and once the link is back it
//! leaves once -- not again on the next drain, and not at all while the
//! link keeps failing.
//!
//! Focus's host in this suite never syncs, which is the machine with no
//! network. The link returning is the drainer the engine runs, driven here
//! over the mock backend and a scripted SMTP server, as `postio-host`'s own
//! RSVP cases drive it. Nothing opens a connection.

use chrono::{Duration, Utc};
use postio_account::backend::{MailBackend as _, MockBackend, MockMailbox};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore as _};
use postio_client::Client;
use postio_model::ListScope;
use postio_model::listing::MailStore as _;
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use postio_storage::BlobStore;

use crate::support::{self, Fixture};

/// An SMTP server that accepts everything, on 587 with STARTTLS.
pub(crate) fn accepting() -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(
            "EHLO",
            "250-mail.example.com\r\n250-STARTTLS\r\n250 AUTH PLAIN",
        )
        .on("STARTTLS", "220 go ahead")
        .on("AUTH PLAIN", "235 authenticated")
        .on("MAIL FROM", "250 ok")
        .on("RCPT TO", "250 ok")
        .on("DATA", "354 go ahead")
        .on("QUIT", "221 bye")
}

/// How many messages a connection carried to the server: each payload
/// ends with the lone dot.
fn delivered(connector: &ScriptedConnector) -> usize {
    String::from_utf8_lossy(&connector.log().written)
        .matches("\r\n.\r\n")
        .count()
}

/// Drain the account's queue over `connector` as of `at`, as the engine
/// does when there is a link.
pub(crate) async fn drain(
    fixture: &Fixture,
    backend: &MockBackend,
    secrets: &std::sync::Arc<MemorySecretStore>,
    blobs: &BlobStore,
    connector: &ScriptedConnector,
    at: chrono::DateTime<Utc>,
) -> postio_sync::DrainReport {
    let tokens = postio_account::auth::StoredPasswordSource::new(secrets.clone());
    let connection = fixture.database.connect().await.expect("a connection");
    postio_sync::Drainer::new(backend)
        .with_smtp(postio_sync::send::SmtpContext {
            connector,
            tokens: &tokens,
            blobs,
        })
        .drain(&connection, fixture.account.id, at)
        .await
        .expect("a drain")
}

async fn in_the_outbox(client: &Client, fixture: &Fixture) -> u32 {
    client
        .list_count(ListScope::Outbox(fixture.account.id))
        .await
        .expect("the Outbox counts")
}

pub fn offline_a_send_is_in_the_outbox_at_once_and_leaves_once() {
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
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session =
            postio_gtk::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
        let client = session.client().clone();
        let blobs = session.host().wiring().blobs.clone();
        support::keep(session);
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "Focus mounted no composer"
        );
        assert_eq!(in_the_outbox(&client, &fixture).await, 0);

        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        let composer = window.composer().expect("the composer");
        composer.test_set_to("Lena Park <lena@example.org>, ");
        composer.test_set_subject("Harbor API draft v3");
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Ctrl+Return did not send: {}",
            composer.status()
        );
        // At once, with no network: nothing here waited on a server.
        assert!(
            crate::settle_until(async || in_the_outbox(&client, &fixture).await == 1).await,
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

        // The link drops before the message goes out: nothing leaves, and
        // it waits in the Outbox for the next try.
        let dropping = ScriptedConnector::new(accepting()).vanishing_at("MAIL FROM");
        drain(&fixture, &backend, &secrets, &blobs, &dropping, Utc::now()).await;
        assert_eq!(delivered(&dropping), 0, "nothing left over a dropped link");
        assert_eq!(
            in_the_outbox(&client, &fixture).await,
            1,
            "still waiting to go"
        );

        // The link is back: it leaves, once.
        let back = ScriptedConnector::new(accepting());
        let later = Utc::now() + Duration::hours(1);
        let report = drain(&fixture, &backend, &secrets, &blobs, &back, later).await;
        assert_eq!(delivered(&back), 1, "it left: {report:?}");
        let written = String::from_utf8_lossy(&back.log().written).into_owned();
        assert!(
            written.contains("RCPT TO:<lena@example.org>")
                && written.contains("Subject: Harbor API draft v3"),
            "what left is the message Focus sent:\n{written}"
        );
        let again = drain(
            &fixture,
            &backend,
            &secrets,
            &blobs,
            &back,
            later + Duration::hours(1),
        )
        .await;
        assert_eq!(delivered(&back), 1, "and never a second time: {again:?}");
        assert!(
            crate::settle_until(async || in_the_outbox(&client, &fixture).await == 0).await,
            "the Outbox still holds it after it left"
        );
    });
}
