//! Keys in the open message (T195, T196), pressed the way GTK delivers
//! them: to the widget the keyboard is really on once the dialog is up,
//! through every key controller between it and the window, in GTK's order.
//!
//! The earlier cases called `FocusWindow::handle_key` directly, which is
//! the window's handler and not a key press: they passed while `j` and `k`
//! did nothing in the running app.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// A window over three messages, the cursor on the first, its message open
/// and drawn.
async fn opened(fixture: &Fixture) -> postio_focus::window::FocusWindow {
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 3).await,
        "the inbox never reached the screen"
    );
    support::keys(&window, &["j"]);
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
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                subject,
                "A line.",
                minutes,
            )
            .await;
        let body = format!("{subject}: a line long enough to scroll. ").repeat(400);
        fixture.write_body(message, &body).await;
    }
}

/// T195: `j` and `k`, pressed into the open message wherever opening it
/// left the keyboard, step the dialog to the next and previous message.
pub fn j_and_k_step_the_open_message_from_where_the_keyboard_is() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened(&fixture).await;
        let reading = window.reading().expect("open");
        assert_eq!(reading.title(), "First");

        support::deliver(&window, "j");
        assert!(
            crate::settle_until(async || reading.title() == "Second").await,
            "j did not step to the next message: {:?}, keyboard on {}",
            reading.title(),
            support::focus_path(&window)
        );
        support::deliver(&window, "k");
        assert!(
            crate::settle_until(async || reading.title() == "First").await,
            "k did not step back: {:?}, keyboard on {}",
            reading.title(),
            support::focus_path(&window)
        );
    });
}

/// The column's vertical adjustment: what the open message scrolls by.
fn column(reading: &postio_focus::open::OpenMessage) -> gtk::Adjustment {
    reading
        .reader()
        .view()
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("the body is in the column's scroller")
        .vadjustment()
}

/// T196: Down and Up move the open message a line, Page Down and Page Up a
/// page, wherever the keyboard is; never to an end, and never by moving the
/// keyboard to a control further down the column.
pub fn the_arrows_and_paging_keys_scroll_the_open_message_in_steps() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
