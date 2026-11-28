//! The window is driven by what the opening thread says (`Progress::Stage`, `Progress::Done`),
//! never by a wait the case sleeps through.
//! A key before the store opens is not a case: Focus answers none but Quit until there is mail.

use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_gtk::startup::{self, Opener, Progress};
use postio_gtk::window::FocusWindow;
use postio_ui::list_state::{OPENING_THRESHOLD, Waiting, describe_wait};

use crate::support::{self, Fixture};

fn opening() -> (FocusWindow, async_channel::Sender<Progress>) {
    let window = FocusWindow::new(None);
    window.present();
    let (sender, receiver) = async_channel::unbounded();
    startup::open(
        &window,
        receiver,
        Rc::new(postio_config::Config::default()),
        Opener::new(
            None,
            Arc::new(postio_account::secret::MemorySecretStore::default()),
        ),
        Rc::new(|session: startup::Session| support::keep(session)),
    );
    (window, sender)
}

pub fn a_start_past_its_budget_says_what_it_is_waiting_on() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let started = std::time::Instant::now();
        let (window, progress) = opening();
        progress
            .send(Progress::Stage(Waiting::Migrating))
            .await
            .expect("the window listens");
        crate::settle();
        // On a machine loaded past the threshold the plate is rightly up; there is nothing to say
        // then.
        if started.elapsed() < OPENING_THRESHOLD {
            assert_eq!(
                window.waiting_text(),
                "",
                "the plate is due after {OPENING_THRESHOLD:?}, not at once"
            );
        }

        let (title, description) = describe_wait(Waiting::Migrating);
        assert!(
            crate::settle_until(async || window.waiting_text() == format!("{title} {description}"))
                .await,
            "past the threshold with the store still opening, the window owes \
             the reader a sentence naming the wait: {:?}",
            window.waiting_text()
        );

        progress
            .send(Progress::Stage(Waiting::Indexing))
            .await
            .expect("the window listens");
        let (title, description) = describe_wait(Waiting::Indexing);
        assert!(
            crate::settle_until(async || window.waiting_text() == format!("{title} {description}"))
                .await,
            "the plate kept naming the earlier wait: {:?}",
            window.waiting_text()
        );

        fixture.file_from_ada("Budget", "Numbers.", 5).await;
        progress
            .send(Progress::Done(Ok(fixture.host())))
            .await
            .expect("the window listens");
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never replaced the plate"
        );
        assert_eq!(window.waiting_text(), "", "the plate outlived its wait");
    });
}

pub fn an_ordinary_start_draws_nothing_that_is_then_removed() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "Numbers.", 5).await;
        let started = std::time::Instant::now();
        let (window, progress) = opening();
        progress
            .send(Progress::Stage(Waiting::Keyring))
            .await
            .expect("the window listens");
        crate::settle();
        if started.elapsed() < OPENING_THRESHOLD {
            assert_eq!(
                window.waiting_text(),
                "",
                "the window said something about a wait of a few milliseconds, \
                 and whatever it said will be replaced the moment the store lands"
            );
        }
        assert!(
            window.rows_on_screen().is_empty(),
            "there is no mail to draw before the store opens"
        );

        progress
            .send(Progress::Done(Ok(fixture.host())))
            .await
            .expect("the window listens");
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );

        crate::settle_for(OPENING_THRESHOLD + std::time::Duration::from_millis(500)).await;
        assert_eq!(
            window.waiting_text(),
            "",
            "the plate arrived over an inbox that had opened"
        );
        assert!(
            !window.rows_on_screen().is_empty(),
            "the plate covered the rows it was a stand-in for"
        );
    });
}
