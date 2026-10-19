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
async fn opened(fixture: &Fixture) -> postio_gtk::window::FocusWindow {
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 3).await,
        "the inbox never reached the screen"
    );
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
        let body = format!("{subject}: a line long enough to scroll. ").repeat(1200);
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
fn column(reading: &postio_gtk::open::OpenMessage) -> gtk::Adjustment {
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

/// A message opens with the keyboard on the message, not on a button of
/// the header: Return or Space after opening scrolls it rather than
/// stepping away.
pub fn the_keyboard_lands_on_the_message_not_a_button() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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

/// The toast an act under the open message raises is drawn over the dialog,
/// where its words and its Undo can be read and pressed.
pub fn a_toast_raised_under_the_open_message_is_drawn_over_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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

/// `d` in the open message digests its sender, as it does beside the list
/// (#1754): the dialog used to drop every key `reading_key` had no arm for.
pub fn d_in_the_open_message_opens_the_digest_rule() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened(&fixture).await;
        support::deliver(&window, "d");
        assert!(
            crate::settle_until(async || window.rule_dialog().is_some()).await,
            "d in the open message opened nothing"
        );
        let dialog = window.rule_dialog().expect("the rule dialog");
        let said = dialog.texts();
        for wanted in ["Digest this sender", "ada@example.com"] {
            assert!(
                said.iter().any(|text| text == wanted),
                "no {wanted:?} in {said:?}"
            );
        }
    });
}

/// A window over three messages with a vault configured, the cursor on the
/// first.
async fn with_a_vault(
    fixture: &Fixture,
    vault: &tempfile::TempDir,
) -> postio_gtk::window::FocusWindow {
    let config = postio_config::Config::from_toml_str(&format!(
        "[focus.vault]\npath = \"{}\"\n",
        vault.path().display()
    ))
    .expect("a config with a vault");
    let window = postio_gtk::window::FocusWindow::new(None);
    gtk::prelude::GtkWindowExt::present(&window);
    support::keep(postio_gtk::startup::adopt(&window, fixture.host(), &config));
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 3).await,
        "the inbox never reached the screen"
    );
    window
}

/// As [`with_a_vault`], with the first message open.
async fn opened_with_a_vault(
    fixture: &Fixture,
    vault: &tempfile::TempDir,
) -> postio_gtk::window::FocusWindow {
    let window = with_a_vault(fixture, vault).await;
    support::deliver(&window, "Return");
    let reading = window.reading().expect("Return opened the message");
    assert!(
        crate::settle_until(async || reading.is_open()).await,
        "the message never opened"
    );
    window
}

/// `T` and `N` in the open message open the capture sheet for it, as a
/// task and as a note, as they do beside the list (#1754, T056).
pub fn t_in_the_open_message_opens_capture() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let vault = tempfile::tempdir().expect("a vault folder");
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = opened_with_a_vault(&fixture, &vault).await;

        support::deliver(&window, "T");
        assert!(
            crate::settle_until(async || window.capture().is_some_and(|sheet| sheet.is_shown()))
                .await,
            "T in the open message opened no capture sheet"
        );
        let sheet = window.capture().expect("the capture sheet");
        let said = sheet.texts();
        assert!(
            said.iter().any(|text| text == "Add task"),
            "the sheet is not a task: {said:?}"
        );
        // Escape puts the sheet away and leaves the message open.
        support::deliver(&window, "Escape");
        assert!(
            crate::settle_until(async || !sheet.is_open()).await,
            "Escape did not close the sheet"
        );
        assert!(
            window.reading().is_some_and(|reading| reading.is_open()),
            "closing the sheet closed the message under it"
        );

        support::deliver(&window, "N");
        assert!(
            crate::settle_until(async || sheet.is_shown()).await,
            "N in the open message opened no capture sheet"
        );
        let said = sheet.texts();
        assert!(
            said.iter().any(|text| text == "Add note"),
            "the sheet is not a note: {said:?}"
        );
    });
}

/// The capture sheet closed, the list's keys are the keyboard's again: a
/// sheet put away must not leave the window in the sheet's key context.
pub fn the_list_keys_work_again_once_the_capture_sheet_is_closed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let vault = tempfile::tempdir().expect("a vault folder");
        let fixture = Fixture::empty().await;
        three(&fixture).await;
        let window = with_a_vault(&fixture, &vault).await;
        let pane = window.pane().expect("the list");
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "the cursor never reached the first row"
        );

        support::deliver(&window, "T");
        assert!(
            crate::settle_until(async || window.capture().is_some_and(|sheet| sheet.is_shown()))
                .await,
            "T opened no capture sheet"
        );
        let sheet = window.capture().expect("the capture sheet");
        support::deliver(&window, "Escape");
        assert!(
            crate::settle_until(async || !sheet.is_open()).await,
            "Escape did not close the sheet"
        );

        support::deliver(&window, "j");
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 1).await,
            "j did not move the cursor after the sheet closed: on {}, context {:?}",
            pane.cursor().selected(),
            window.key_context()
        );
    });
}
