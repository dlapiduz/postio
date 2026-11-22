use postio_ui::dwell::DWELL_TO_READ;

use crate::support::{self, Fixture};

fn bold(window: &postio_gtk::window::FocusWindow, subject: &str) -> Option<bool> {
    window
        .pane()?
        .rows_on_screen()
        .into_iter()
        .map(|row| row.drawn())
        .find(|drawn| drawn.texts.iter().any(|text| text == subject))
        .map(|drawn| drawn.bold)
}

fn read_in_pane(window: &postio_gtk::window::FocusWindow) {
    window.set_focus_config(postio_config::FocusConfig {
        reading: postio_config::Reading::Pane,
        ..postio_config::FocusConfig::default()
    });
    crate::settle();
}

fn showing(window: &postio_gtk::window::FocusWindow) -> Option<String> {
    window
        .reading()
        .filter(|reading| reading.is_open())
        .map(|reading| reading.title())
}

async fn all_bold(window: &postio_gtk::window::FocusWindow) {
    for subject in ["First", "Second", "Third"] {
        assert!(
            crate::settle_until(async || bold(window, subject) == Some(true)).await,
            "{subject} was not drawn unread to begin with: {:?}",
            bold(window, subject)
        );
    }
}

pub fn a_message_left_open_in_the_dialog_is_marked_read() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

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

pub fn a_message_the_composer_replaced_is_not_marked_read() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        all_bold(&window).await;

        support::deliver(&window, "Return");
        assert!(crate::settle_until(async || showing(&window).as_deref() == Some("First")).await);
        support::deliver(&window, "r");
        assert!(
            crate::settle_until(async || window.composer().is_some_and(|c| c.is_open())).await,
            "r opened no composer"
        );
        crate::settle_for(DWELL_TO_READ * 2).await;
        assert_eq!(
            bold(&window, "First"),
            Some(true),
            "First was marked read after the composer had replaced it"
        );
    });
}

pub fn stepping_the_pane_past_a_message_leaves_it_unread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        all_bold(&window).await;

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
                support::deliver(&window, "I"),
                "I reached nothing in the open message"
            );
            assert!(
                crate::settle_until(async || bold(&window, subject) == Some(true)).await,
                "I in the open message (pane: {pane}) did not mark {subject} unread again"
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

pub fn a_message_in_a_window_that_lost_focus_stays_unread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || showing(&window).as_deref() == Some("First")).await,
            "Return opened nothing"
        );
        // The headless compositor grants no focus to take away, so say what GTK's `is-active`
        // notification would.
        window.focus_changed(false);
        crate::settle_for(DWELL_TO_READ * 2).await;
        assert_eq!(
            bold(&window, "First"),
            Some(true),
            "a message open in an unfocused window was marked read"
        );

        window.focus_changed(true);
        assert!(
            crate::settle_until(async || bold(&window, "First") == Some(false)).await,
            "First stayed unread after the window was back for the dwell"
        );
    });
}

pub fn refocusing_does_not_take_back_r() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        all_bold(&window).await;

        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || bold(&window, "First") == Some(false)).await,
            "First was never marked read"
        );
        assert!(support::deliver(&window, "I"), "I reached nothing");
        assert!(crate::settle_until(async || bold(&window, "First") == Some(true)).await);
        window.focus_changed(false);
        window.focus_changed(true);
        crate::settle_for(DWELL_TO_READ * 2).await;
        assert_eq!(
            bold(&window, "First"),
            Some(true),
            "refocusing the window marked the message read again"
        );
    });
}
