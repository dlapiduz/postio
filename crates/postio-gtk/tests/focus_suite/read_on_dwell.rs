//! Reading a message marks it read (T237; screens.md, "Reading marks it
//! read"): a message that stays open, in the dialog or in the pane beside the
//! list, for `postio_ui::dwell::DWELL_TO_READ` stops being bold; one stepped
//! past, or closed, sooner stays unread; and `r` in the open message gives
//! the bold back. Each read off what the rows drew.

use postio_ui::dwell::DWELL_TO_READ;

use crate::support::{self, Fixture};

/// Whether the row saying `subject` was drawn bold, once it is drawn.
fn bold(window: &postio_gtk::window::FocusWindow, subject: &str) -> Option<bool> {
    window
        .pane()?
        .rows_on_screen()
        .into_iter()
        .map(|row| row.drawn())
        .find(|drawn| drawn.texts.iter().any(|text| text == subject))
        .map(|drawn| drawn.bold)
}

/// Put `window` in pane mode as the setting does.
fn read_in_pane(window: &postio_gtk::window::FocusWindow) {
    window.set_focus_config(postio_config::FocusConfig {
        reading: postio_config::Reading::Pane,
        ..postio_config::FocusConfig::default()
    });
    crate::settle();
}

/// The open message's title, wherever it is placed.
fn showing(window: &postio_gtk::window::FocusWindow) -> Option<String> {
    window
        .reading()
        .filter(|reading| reading.is_open())
        .map(|reading| reading.title())
}

/// Every row starts bold: the fixture's mail is unread.
async fn all_bold(window: &postio_gtk::window::FocusWindow) {
    for subject in ["First", "Second", "Third"] {
        assert!(
            crate::settle_until(async || bold(window, subject) == Some(true)).await,
            "{subject} was not drawn unread to begin with: {:?}",
            bold(window, subject)
        );
    }
}

/// Open in the dialog and stay: past the dwell the row is no longer bold,
/// and the rows nobody opened still are.
pub fn a_message_left_open_in_the_dialog_is_marked_read() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

        support::deliver(&window, "j");
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || showing(&window).as_deref() == Some("First")).await,
            "Return opened nothing"
        );
        assert!(
            crate::settle_until(async || bold(&window, "First") == Some(false)).await,
            "First stayed bold after it was open for the dwell"
        );
        assert_eq!(
            bold(&window, "Second"),
            Some(true),
            "a row nobody opened was marked read"
        );
    });
}

/// `j` and `k` with a message open beside the list step the pane through
/// the rows; a message passed faster than the dwell stays unread, and only
/// the one the pane rests on is marked.
pub fn stepping_the_pane_past_a_message_leaves_it_unread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        all_bold(&window).await;

        support::deliver(&window, "j");
        support::deliver(&window, "Return");
        support::deliver(&window, "j");
        support::deliver(&window, "j");
        assert!(
            crate::settle_until(async || showing(&window).as_deref() == Some("Third")).await,
            "j did not step the pane to Third: {:?}",
            showing(&window)
        );
        assert!(
            crate::settle_until(async || bold(&window, "Third") == Some(false)).await,
            "the message the pane rested on stayed bold"
        );
        // Longer than a dwell started on either of them would have taken.
        crate::settle_for(DWELL_TO_READ * 2).await;
        for passed in ["First", "Second"] {
            assert_eq!(
                bold(&window, passed),
                Some(true),
                "{passed} was marked read by the pane passing over it"
            );
        }
    });
}

/// Closing the message before the dwell leaves it unread.
pub fn a_message_closed_before_the_dwell_stays_unread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

        support::deliver(&window, "j");
        support::deliver(&window, "Return");
        assert!(crate::settle_until(async || showing(&window).is_some()).await);
        support::deliver(&window, "Escape");
        assert!(crate::settle_until(async || showing(&window).is_none()).await);
        crate::settle_for(DWELL_TO_READ * 2).await;
        assert_eq!(
            bold(&window, "First"),
            Some(true),
            "a message glanced at and closed was marked read"
        );
    });
}

/// `r` in the open message, once the dwell has marked it read, marks it
/// unread again, and the dwell does not take that back while it stays open:
/// the person's choice wins over the clock. In the dialog and in the pane.
pub fn r_in_the_open_message_marks_it_unread_again() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

        for (pane, subject) in [(false, "First"), (true, "Second")] {
            if pane {
                support::deliver(&window, "Escape");
                assert!(crate::settle_until(async || showing(&window).is_none()).await);
                read_in_pane(&window);
                support::deliver(&window, "j");
            } else {
                support::deliver(&window, "j");
            }
            support::deliver(&window, "Return");
            assert!(
                crate::settle_until(async || showing(&window).as_deref() == Some(subject)).await,
                "Return did not open {subject}: {:?}",
                showing(&window)
            );
            assert!(
                crate::settle_until(async || bold(&window, subject) == Some(false)).await,
                "{subject} was never marked read"
            );
            assert!(
                support::deliver(&window, "r"),
                "r reached nothing in the open message"
            );
            assert!(
                crate::settle_until(async || bold(&window, subject) == Some(true)).await,
                "r in the open message (pane: {pane}) did not mark {subject} unread again"
            );
            crate::settle_for(DWELL_TO_READ * 2).await;
            assert_eq!(
                bold(&window, subject),
                Some(true),
                "the dwell marked {subject} read again after r kept it unread"
            );
            assert_eq!(showing(&window).as_deref(), Some(subject), "r closed it");
        }
    });
}
