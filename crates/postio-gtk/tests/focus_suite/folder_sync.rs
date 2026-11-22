use std::sync::Arc;

use postio_account::backend::{MockBackend, MockMailbox};
use postio_account::secret::MemorySecretStore;

use crate::support::{self, Fixture};

pub fn a_folder_the_server_will_not_open_is_marked_until_it_syncs() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "Numbers.", 5).await;
        fixture.folder("Receipts").await;
        let backend = Arc::new(
            MockBackend::builder()
                .mailbox(MockMailbox::new("INBOX"))
                .mailbox(MockMailbox::new("Archive"))
                .mailbox(MockMailbox::new("Trash"))
                .mailbox(MockMailbox::new("Receipts"))
                .build(),
        );
        backend.refuse_select("Receipts", "Mailbox is locked");
        let host = postio_host::Host::start(fixture.database.clone(), fixture.blob_store(), {
            let backend = backend.clone();
            move |wiring| {
                wiring
                    .with_secrets(Arc::new(MemorySecretStore::new()))
                    .with_mail(postio_session::MailOverride {
                        backend,
                        smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                            postio_smtp::transport::SmtpScript::new("220 ready"),
                        )),
                    })
            }
        })
        .expect("a host");
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        session.start_syncing();

        assert!(
            crate::settle_until(async || window.sync_said() == "Receipts can't sync").await,
            "the label never named the folder: {:?}",
            window.sync_said()
        );
        let detail = window.sync_detail().unwrap_or_default();
        assert!(
            detail.starts_with("Receipts: ") && detail.contains("Mailbox is locked"),
            "the tooltip says why: {detail:?}"
        );

        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
        assert!(
            crate::settle_until(async || !places.warnings().is_empty()).await,
            "no row is marked: {:?}",
            places.names()
        );
        let warnings = places.warnings();
        assert_eq!(warnings.len(), 1, "only the refused folder: {warnings:?}");
        assert_eq!(warnings[0].0, "Receipts");
        assert!(warnings[0].1.contains("Mailbox is locked"), "{warnings:?}");
        places.set_filter("Receipts");
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.place_name() == "Receipts").await,
            "the marked row did not open Receipts: {:?}",
            window.place_name()
        );

        backend.allow_select("Receipts");
        support::press(&window, "F5", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.sync_said().starts_with("Synced ")).await,
            "the mark outlived the folder's completed pass: {:?}",
            window.sync_said()
        );
        assert_eq!(window.sync_detail(), None);
        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Receipts".to_owned())).await,
            "the popover never listed Receipts: {:?}",
            places.names()
        );
        assert_eq!(places.warnings(), Vec::<(String, String)>::new());
        session.stop();
        support::keep(session);
    });
}
