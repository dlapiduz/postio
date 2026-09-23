//! A new-mail notification reads the store on the runtime, never on the GTK
//! main thread (#1608).
//!
//! The notifier used to await two fresh store connections -- the mailbox's
//! role and the account's label -- inline in the single event drain, so
//! every event queued behind a `NewMail` waited for them on the thread that
//! draws. Counted, not timed: the counting seam is thread-local, so what it
//! sees here is exactly what ran on this thread.

use gtk::gdk;
use postio_app::notifications::Notifier;
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::window::Window;
use postio_model::MailboxRole;
use postio_runtime::store::LocalStore;
use postio_storage::seed::seed_small;
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;
use postio_ui::notify::Attention;

pub fn a_new_mail_notification_reads_nothing_on_the_main_thread() {
    crate::gtk_case(async {
        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let database = test_support::memory().await;
        let report = seed_small(&database, 12).await;
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
        let message = {
            let connection = database.connect().await.expect("a connection");
            postio_storage::repository::MessageRepository::new(&connection)
                .page(&postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(inbox),
                    limit: 1,
                    after: None,
                })
                .await
                .expect("a page")
                .first()
                .expect("the inbox has mail")
                .id
        };
        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let notifier = Notifier::new(
            database.clone(),
            std::sync::Arc::new(LocalStore::new(&database)),
            bridge.handle(),
            Default::default(),
        );
        let window = Window::default();

        let counts = counted_async(async || {
            notifier
                .notify(&window, inbox, &[message], Attention::default())
                .await;
        })
        .await;
        assert_eq!(
            counts.statements, 0,
            "notifying about one message ran {} statements on the GTK thread; \
             every event queued behind it waited for them",
            counts.statements
        );
    });
}
