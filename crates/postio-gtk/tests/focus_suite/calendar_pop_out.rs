use gtk::prelude::*;

use crate::support::{self, Fixture};

async fn agenda_open() -> (Fixture, postio_gtk::window::FocusWindow) {
    let fixture = Fixture::empty().await;
    fixture
        .file(("Ada Moreno", "ada@example.test"), "Hello", "Hi.", 5)
        .await;
    crate::calendar::with_events(&fixture).await;
    let (window, _client) = fixture.open().await;
    support::keys(&window, &["g", "e"]);
    assert!(
        crate::settle_until(async || window.calendar().and_then(|view| view.ringed()).is_some())
            .await
    );
    support::keys(&window, &["j"]);
    (fixture, window)
}

fn pop(window: &postio_gtk::window::FocusWindow) {
    support::press(
        window,
        "o",
        gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
    );
}

fn ringed_out(window: &postio_gtk::window::FocusWindow) -> Option<String> {
    window
        .calendar_window_agenda()
        .and_then(|agenda| agenda.ringed())
        .map(|row| row.title)
}

pub fn mod_shift_o_moves_calendar_into_its_own_window_and_back_with_its_ring() {
    crate::gtk_case(async {
        let (_fixture, window) = agenda_open().await;
        assert!(
            crate::settle_until(async || window
                .calendar()
                .and_then(|view| view.ringed())
                .is_some_and(|row| row.title == "After that"))
            .await
        );

        pop(&window);
        assert!(
            crate::settle_until(async || ringed_out(&window).as_deref() == Some("After that"))
                .await,
            "Calendar is in its own window with the same ring"
        );
        assert!(
            window.calendar().is_none(),
            "the main window is back on mail"
        );

        support::keys(&window, &["Escape"]);
        crate::settle();
        assert!(
            window.calendar_window_open(),
            "Escape at its root closes nothing"
        );

        pop(&window);
        assert!(
            crate::settle_until(async || !window.calendar_window_open()
                && window
                    .calendar()
                    .and_then(|view| view.ringed())
                    .is_some_and(|row| row.title == "After that"))
            .await,
            "the same chord brings Calendar back as the main window's mode"
        );
    });
}

pub fn destinations_present_the_window_that_shows_them() {
    crate::gtk_case(async {
        let (_fixture, window) = agenda_open().await;
        pop(&window);
        assert!(crate::settle_until(async || window.calendar_window_open()).await);

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || window
                .pane()
                .is_some_and(|pane| pane.feed().focused_window() == postio_focus::Window::Main))
            .await,
            "g i in the Calendar window goes to mail in the main window"
        );
        support::keys(&window, &["g", "e"]);
        assert!(
            crate::settle_until(async || {
                window.pane().is_some_and(|pane| {
                    pane.feed().focused_window() == postio_focus::Window::Calendar
                })
            })
            .await,
            "g e from mail goes to the Calendar window"
        );
        assert!(
            window.calendar().is_none(),
            "the main window shows no Calendar mode"
        );
    });
}

pub fn closing_the_calendar_window_or_mod_w_pops_calendar_back_in() {
    crate::gtk_case(async {
        let (_fixture, window) = agenda_open().await;
        pop(&window);
        assert!(crate::settle_until(async || window.calendar_window_open()).await);

        support::press(&window, "w", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(
                async || !window.calendar_window_open() && window.calendar().is_some()
            )
            .await,
            "mod+w in the Calendar window pops it in rather than quitting"
        );

        pop(&window);
        assert!(crate::settle_until(async || window.calendar_window_open()).await);
        if let Some(host) = window
            .calendar_window_agenda()
            .and_then(|agenda| agenda.widget().root())
        {
            host.downcast::<gtk::Window>().expect("a window").close();
        }
        assert!(
            crate::settle_until(
                async || !window.calendar_window_open() && window.calendar().is_some()
            )
            .await,
            "its X pops it in"
        );
    });
}

pub fn calendar_popped_out_at_quit_reopens_in_its_own_window() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.test"), "Hello", "Hi.", 5)
            .await;
        crate::calendar::with_events(&fixture).await;
        postio_session::calendar::set_calendar_window(
            &fixture.database,
            postio_model::calendar::CalendarWindowState {
                popped: true,
                width: 800,
                height: 600,
            },
        )
        .await
        .expect("saved");

        let (window, _client) = fixture.open().await;

        assert!(
            crate::settle_until(async || ringed_out(&window).is_some()).await,
            "Calendar opens in its own window once mail is up"
        );
        assert!(
            !support::subjects(&window).is_empty(),
            "the main window shows mail"
        );
    });
}
