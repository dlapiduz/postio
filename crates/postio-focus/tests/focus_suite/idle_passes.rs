//! The store's idle passes wait for the first frame (#1604).
//!
//! The body indexer's catch-up, the header repair, the header index catch-up
//! and the disk reclaim are catch-up work nobody is waiting on; the first
//! frame is. They used to start at the same instant as the first page read,
//! on a runtime of two workers. In Focus they start with
//! `Session::start_syncing`, which `app.rs` calls from the frame the stored
//! mail is drawn in -- so adopting a store starts none of them, and starting
//! sync does start them, a moment later.
//!
//! Nothing here dials anything: the host reads and files mail through a mock
//! backend, so `start_syncing` opens no socket.

use std::sync::Arc;

use chrono::Utc;
use postio_account::backend::MockBackend;
use postio_account::secret::MemorySecretStore;
use postio_index::{SearchRequest, search};
use postio_model::{AccountId, AccountScope, BodyState, MessageId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::{MessageRepository, StoredBody};

use crate::support::{self, Fixture};

/// The messages matching `query`, as search answers.
async fn hits(fixture: &Fixture, account: AccountId, query: &str) -> Vec<MessageId> {
    let connection = fixture.database.connect().await.expect("a connection");
    let parsed = parse(query, Utc::now().date_naive());
    search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 50,
            order: postio_search::ResultOrder::Relevance,
        },
        Utc::now(),
    )
    .await
    .expect("the search runs")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

pub fn the_idle_passes_wait_for_the_first_frame() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (target, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 5)
            .await;
        // A body already on this machine that the index has not seen.
        {
            let connection = fixture.database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .set_body(
                    target,
                    &StoredBody {
                        text: Some(
                            "A word no header in the corpus carries: photogrammetry.".to_owned(),
                        ),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .await
                .expect("store the body");
        }
        fixture.index().await;
        assert!(
            hits(&fixture, fixture.account.id, "photogrammetry")
                .await
                .is_empty(),
            "the body is not indexed yet, so this cannot be measuring the wiring"
        );

        let host = postio_host::Host::start(fixture.database.clone(), fixture.blob_store(), {
            |wiring| {
                wiring
                    .with_secrets(Arc::new(MemorySecretStore::new()))
                    .with_mail(postio_session::MailOverride {
                        backend: Arc::new(MockBackend::builder().build()),
                        smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                            postio_smtp::transport::SmtpScript::new("220 ready"),
                        )),
                    })
            }
        })
        .expect("a host");
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session =
            postio_focus::startup::adopt(&window, host, &postio_config::Config::default());
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        // The first frame is up, and the idle passes have not been started:
        // a second of the loop turning gives them every chance.
        crate::settle_for(std::time::Duration::from_secs(1)).await;
        assert!(
            hits(&fixture, fixture.account.id, "photogrammetry")
                .await
                .is_empty(),
            "the body index caught up before anything started the idle passes"
        );

        session.start_syncing();
        let found = crate::settle_until(async || {
            hits(&fixture, fixture.account.id, "photogrammetry")
                .await
                .contains(&target)
        })
        .await;
        assert!(found, "the idle passes never ran once sync had started");
        session.stop();
        support::keep(session);
    });
}
