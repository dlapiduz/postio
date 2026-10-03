//! A new-mail notification reads the store on the runtime, never on the GTK
//! main thread (#1608).
//!
//! Focus's notifier is the host's (`Host::focus_notification`), which spawns
//! the decision on the host's runtime. The event drain runs on the thread
//! that draws, so a store read inline there stalls every event queued behind
//! a `NewMail`. Counted, not timed: the counting seam is thread-local, so
//! what it sees here is exactly what ran on this thread, from the event
//! being told to the notification being delivered.

use std::cell::RefCell;
use std::rc::Rc;

use postio_storage::test_support::counting::counted_async;

use crate::support::{self, Fixture};

pub fn a_new_mail_notification_reads_nothing_on_the_main_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "The numbers.",
                5,
            )
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let told: Rc<RefCell<Vec<String>>> = Rc::default();
        window.set_notification_sink({
            let told = Rc::clone(&told);
            move |notification| told.borrow_mut().push(notification.body.clone())
        });
        // Looking at Filtered, not the inbox: an arrival there is news.
        support::keys(&window, &["g", "f"]);
        crate::settle();

        let counts = counted_async(async || {
            assert!(sink.emit(postio_core::Event::NewMail {
                account: fixture.account.id,
                mailbox: fixture.inbox,
                messages: vec![message],
            }));
            assert!(
                crate::settle_until(async || !told.borrow().is_empty()).await,
                "no notification for mail that stayed in the inbox"
            );
        })
        .await;
        assert!(
            told.borrow()[0].contains("Atlas budget"),
            "it names the mail: {:?}",
            told.borrow()
        );
        assert_eq!(
            counts.statements, 0,
            "notifying about one message ran {} statements on the GTK thread; \
             every event queued behind it waited for them",
            counts.statements
        );
    });
}
