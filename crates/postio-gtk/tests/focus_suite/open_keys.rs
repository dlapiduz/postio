//! Keys are pressed as GTK delivers them, through every controller between the focused widget and
//! the window:
//! calling `FocusWindow::handle_key` directly passed while `j` and `k` did nothing in the running
//! app.

use gtk::prelude::*;

use crate::support::{self, Fixture};

async fn opened(fixture: &Fixture) -> postio_gtk::window::FocusWindow {
    let (window, _client) = fixture.open().await;
    support::rows_shown(&window, 3).await;
    support::deliver(&window, "Return");
    let reading = window.reading().expect("Return opened the message");
    assert!(
        crate::settle_until(async || reading.is_open()
            && reading.reader().view().document().is_some()
            && !reading.body_text().is_empty())
        .await,
        "the body never arrived"
    );
    assert!(
        crate::settle_until(async || reading.reader().view().tiles_settled()).await,
        "the body was never drawn"
    );
    window
}

async fn three(fixture: &Fixture) {
    for (minutes, subject) in [(10, "First"), (20, "Second"), (30, "Third")] {
        let (message, _) = fixture.file_from_ada(subject, "A line.", minutes).await;
        let body = format!("{subject}: a line long enough to scroll. ").repeat(1200);
        fixture.write_body(message, &body).await;
    }
}

fn column(reading: &postio_gtk::open::OpenMessage) -> gtk::Adjustment {
    reading
        .reader()
        .view()
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("the body is in the column's scroller")
        .vadjustment()
}

pub fn the_arrows_and_paging_keys_scroll_the_open_message_in_steps() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened(&fixture).await;
        let reading = window.reading().expect("open");
        let adjustment = column(&reading);
        assert!(
            crate::settle_until(
                async || adjustment.upper() - adjustment.page_size() > 4.0 * adjustment.page_size()
            )
            .await,
            "the body is not long enough to scroll: {} of {}",
            adjustment.page_size(),
            adjustment.upper()
        );
        let bottom = adjustment.upper() - adjustment.page_size();
        let keyboard = support::focus_path(&window);

        let start = adjustment.value();
        support::deliver(&window, "Down");
        let line = adjustment.value() - start;
        assert!(
            line > 0.0 && (line - adjustment.step_increment()).abs() < 1.0,
            "Down moved {line} (a line is {}, the end is {bottom})",
            adjustment.step_increment()
        );
        support::deliver(&window, "Page_Down");
        let page = adjustment.value() - start - line;
        assert!(
            page > 0.0 && (page - adjustment.page_size()).abs() < 1.0,
            "Page Down moved {page} (a page is {}, the end is {bottom})",
            adjustment.page_size()
        );
        support::deliver(&window, "Up");
        assert!(
            (adjustment.value() - (start + page)).abs() < 1.0,
            "Up did not move back a line: at {}",
            adjustment.value()
        );
        assert_eq!(
            support::focus_path(&window),
            keyboard,
            "the keys moved the keyboard rather than the column"
        );
    });
}

pub fn the_keyboard_lands_on_the_message_not_a_button() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened(&fixture).await;
        let focus = gtk::prelude::GtkWindowExt::focus(&window);
        let on_the_message = focus.as_ref().is_some_and(|focus| {
            focus.is::<gtk::ScrolledWindow>()
                || focus.ancestor(gtk::ScrolledWindow::static_type()).is_some()
        });
        assert!(
            on_the_message,
            "the keyboard is on {}",
            support::focus_path(&window)
        );
    });
}

pub fn a_toast_raised_under_the_open_message_is_drawn_over_it() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened(&fixture).await;
        let reading = window.reading().expect("open");
        support::deliver(&window, "e");
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "archiving raised no toast"
        );
        let host = window.toast_host().expect("a toast has a host");
        assert!(
            host.is_ancestor(&reading.dialog()),
            "the toast is drawn in the window, under the dialog"
        );
    });
}
