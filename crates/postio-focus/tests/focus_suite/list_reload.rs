//! Mail arriving while a person is in the list (#1177): the cursor stays on
//! the message it was on and the marks stay on the messages they were put
//! on, whatever a sync pass does to the order underneath them.
//!
//! The engine announces that a mailbox moved; it does not say how, because
//! it does not know. A list that kept the cursor on the *row number* would,
//! after a batch or a re-sort, put the reading pane and the next verb on
//! whichever message now sits at that index.

use gtk::prelude::*;
use postio_core::Event;
use postio_core::state::Selection;

use crate::support::{self, Fixture};

/// A folder of `count` conversations, newest first, subjects `Mail 1`...
async fn folder(fixture: &Fixture, count: i64) {
    for number in 1..=count {
        fixture
            .file_in(fixture.inbox, &format!("Mail {number}"), number + 10)
            .await;
    }
}

/// Tell the window the way sync does: this mailbox changed.
fn moved(sink: &postio_core::bridge::EventSink, fixture: &Fixture) {
    assert!(
        sink.emit(Event::MessageListChanged {
            account: fixture.account.id,
            mailbox: fixture.inbox,
        }),
        "the hub took the event"
    );
}

pub fn a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        folder(&fixture, 120).await;
        let (host, sink) = fixture.host_telling();
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_focus::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || window
                .pane()
                .is_some_and(|pane| pane.feed().total() == 120))
            .await,
            "the folder as it stands never arrived"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window).len() >= 10).await,
            "the first page never drew"
        );

        // Where the person is: a few rows down, with three messages marked.
        support::keys(&window, &["j", "x", "j", "x", "j", "x", "j", "j", "j"]);
        crate::settle();
        let cursor = window.cursor_row().expect("the cursor is on a row").id();
        let selection = window.selection();
        assert_ne!(
            selection,
            Selection::default(),
            "three rows are marked, or this proves nothing"
        );

        // Batches commit, as a sync does for minutes rather than once: older
        // mail filed behind the person's place.
        for batch in 0..3 {
            for number in 0..40 {
                fixture
                    .file_in(
                        fixture.inbox,
                        &format!("Older {batch}.{number}"),
                        10_000 + batch * 100 + number,
                    )
                    .await;
            }
            moved(&sink, &fixture);
            let expected = 160 + batch as u32 * 40;
            assert!(
                crate::settle_until(async || window
                    .pane()
                    .is_some_and(|pane| pane.feed().total() == expected))
                .await,
                "the batch never arrived, so this proves nothing: {} rows",
                window.pane().map_or(0, |pane| pane.feed().total())
            );
            assert_eq!(
                window.cursor_row().map(|row| row.id()),
                Some(cursor),
                "the cursor moved to a different message when batch {batch} landed"
            );
            assert_eq!(
                window.selection(),
                selection,
                "marked messages were lost when batch {batch} landed: the next \
                 verb would have hit the wrong mail"
            );
        }
    });
}

pub fn a_reordering_sync_leaves_the_cursor_on_the_same_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        folder(&fixture, 120).await;
        let (host, sink) = fixture.host_telling();
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_focus::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || window
                .pane()
                .is_some_and(|pane| pane.feed().total() == 120))
            .await,
            "the folder never arrived"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window).len() >= 10).await,
            "the first page never drew"
        );

        // Reading something a few rows down.
        support::keys(&window, &["j", "j", "j", "j", "j", "j"]);
        crate::settle();
        let reading = window.cursor_row().expect("the cursor is on a row");
        let reading_id = reading.id();
        let at = window.pane().expect("the list").cursor().selected();

        // A sync pass files newer mail above it: every row moves down one.
        fixture.file_in(fixture.inbox, "Just arrived", 1).await;
        moved(&sink, &fixture);
        assert!(
            crate::settle_until(async || window
                .pane()
                .is_some_and(|pane| pane.feed().total() == 121))
            .await,
            "the new mail never arrived, so this proves nothing"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window)
                .first()
                .is_some_and(|subject| subject == "Just arrived"))
            .await,
            "the list never re-sorted: {:?}",
            support::subjects(&window)
        );
        crate::settle();
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            Some(reading_id),
            "the order moved and the cursor stayed on row {at}, so the next \
             verb would act on a different message than the one being read"
        );
    });
}
