use std::sync::Arc;

use gtk::prelude::*;
use postio_account::backend::{
    BodyStructure, Held, MockBackend, MockMailbox, MockMessage, PartNode,
};
use postio_account::secret::MemorySecretStore;
use postio_model::ListScope;
use postio_model::listing::MailStore as _;

use crate::support::{self, Fixture};

const PAYLOAD: &str = "%PDF carried";
const TEXT: &str = "See attached.";

fn report() -> MockMessage {
    let raw = format!(
        "Message-ID: <report@example.com>\r\nFrom: Ada Moreno <ada@example.com>\r\n\
         To: test@example.com\r\nSubject: Report\r\n\
         Content-Type: multipart/mixed; boundary=parts\r\n\r\n\
         --parts\r\nContent-Type: text/plain\r\n\r\n{TEXT}\r\n\
         --parts\r\nContent-Type: application/pdf\r\n\
         Content-Disposition: attachment; filename=report.pdf\r\n\
         Content-Transfer-Encoding: 7bit\r\n\r\n{PAYLOAD}\r\n--parts--\r\n"
    );
    MockMessage::new(raw.into_bytes())
        .with_structure(BodyStructure::from_parts(
            "multipart/mixed",
            [
                PartNode::new("1", "text/plain", TEXT.len() as u64).with_encoding("7bit"),
                PartNode::new("2", "application/pdf", PAYLOAD.len() as u64)
                    .with_encoding("7bit")
                    .with_filename("report.pdf"),
            ],
        ))
        .with_part("1", TEXT.as_bytes())
        .with_part("2", PAYLOAD.as_bytes())
}

fn attachment_notes(composer: &postio_widgets::composer::Composer) -> Vec<String> {
    support::with_class(composer, "postio-compose-label")
        .into_iter()
        .filter(|widget| widget.is_visible() && widget.has_css_class("dim-label"))
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

pub fn a_forwarded_part_reads_downloading_until_its_bytes_land() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let backend = MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX").message(report()))
            .build();
        backend.hold(Held::Bodies, "INBOX", 1);
        let host = postio_host::Host::start(fixture.database.clone(), fixture.blob_store(), {
            let backend = backend.clone();
            move |wiring| {
                wiring
                    .with_secrets(Arc::new(MemorySecretStore::new()))
                    .with_mail(postio_session::MailOverride {
                        backend: Arc::new(backend),
                        smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                            postio_smtp::transport::SmtpScript::new("220 ready"),
                        )),
                    })
            }
        })
        .expect("a host");
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
        let client = session.client().clone();
        session.start_syncing();
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Report"]).await,
            "the report never synced: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "Focus mounted no composer"
        );
        // A row drawn is not yet a row the cursor is on; a press before it lands forwards nothing.
        assert!(
            crate::settle_until(async || window.observe().cursor.index == Some(0)).await,
            "the cursor never landed on the report: {:?}",
            window.observe().cursor
        );

        support::keys(&window, &["f"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "f opened no composer"
        );
        let composer = window.composer().expect("the composer");
        assert!(
            crate::settle_until(async || composer.draft().attachments.len() == 1).await,
            "the forward carries no attachment"
        );
        assert_eq!(composer.draft().kind, postio_model::DraftKind::Forward);
        assert!(
            crate::settle_until(async || attachment_notes(&composer)
                .iter()
                .any(|note| note.contains("downloading")))
            .await,
            "the carried part never read as downloading: {:?}",
            attachment_notes(&composer)
        );
        assert!(
            crate::settle_until(async || backend.parked() == 1).await,
            "the part was never asked for"
        );
        assert_eq!(client.counts().of("FetchCarried"), 1);
        assert_eq!(
            client
                .list_count(ListScope::Outbox(fixture.account.id))
                .await
                .expect("the Outbox counts"),
            0,
            "opening a forward sends nothing"
        );
        crate::settle();
        assert!(
            attachment_notes(&composer)
                .iter()
                .any(|note| note.contains("downloading")),
            "{:?}",
            attachment_notes(&composer)
        );

        backend.release();
        assert!(
            crate::settle_until(async || {
                let notes = attachment_notes(&composer);
                !notes.is_empty() && notes.iter().all(|note| !note.contains("downloading"))
            })
            .await,
            "the part landed and still reads: {:?}",
            attachment_notes(&composer)
        );
        // `BodyLoaded` comes only after the write, but the drawing also ends on any answer but
        // `Queued`, and `Unreachable` still fetches once the account is up:
        // read the store until the part is there, not once (#2206).
        let original = composer
            .draft()
            .forwarded_from
            .expect("a forward names its original");
        assert!(
            crate::settle_until(async || client
                .parts(original)
                .await
                .expect("the original's parts")
                .iter()
                .any(|part| part.part_id.as_deref() == Some("2") && part.blob_id.is_some()))
            .await,
            "the part's bytes are not on this machine; the server served {} body \
             fetches and holds {} parked",
            backend.body_fetches().len(),
            backend.parked()
        );
        assert!(composer.is_visible(), "the forward stayed open");
        session.stop();
        support::keep(session);
    });
}
