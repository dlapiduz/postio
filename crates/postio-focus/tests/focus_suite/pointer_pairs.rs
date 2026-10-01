//! T194: the mouse beside the keyboard. Enter opens the cursor's message
//! and so does a double-click on a row; a press in a row's gutter toggles
//! its selection as `x` does; Escape closes the dialogs.

use gtk::gdk;
use gtk::prelude::*;

use crate::support::{self, Fixture};

/// A double-click on a row is GTK's `activate` on the list (the list is not
/// single-click-activate, so a single click only moves the cursor). It must
/// open that row's message through the one `OpenMessage` command.
pub fn a_double_click_on_a_row_opens_its_message_as_enter_does() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            !pane.view().is_single_click_activate(),
            "a single click must only move the cursor"
        );
        assert!(window.reading().is_none(), "nothing is open yet");
        pane.view().emit_by_name::<()>("activate", &[&2u32]);
        let reading = window.reading().expect("a double-click opened a message");
        assert!(
            crate::settle_until(async || !reading.body_text().is_empty()
                || reading.reader().view().document().is_some())
            .await,
            "the opened message never drew"
        );
        assert_eq!(
            pane.cursor().selected(),
            2,
            "the cursor went to the row that was double-clicked"
        );
        let subject = support::subjects(&window)[2].clone();
        assert!(
            crate::settle_until(async || support::texts(&reading.dialog())
                .iter()
                .any(|text| text.contains(&subject)))
            .await,
            "the dialog shows the double-clicked row's message ({subject:?}): {:?}",
            support::texts(&reading.dialog())
        );
    });
}

/// A press in a row's gutter toggles that row's selection, as `x` does.
pub fn a_press_in_a_rows_gutter_toggles_its_selection() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        let rows = pane.rows_on_screen();
        let row = rows.first().expect("a row on screen");
        let bar = support::only(&window, "focus-bulk-bar");
        assert!(!bar.is_mapped());
        assert!(row.press_at(30.0, 20.0), "the gutter took the press");
        assert!(
            crate::settle_until(async || bar.is_mapped()).await,
            "a gutter press selects the row"
        );
        assert!(
            support::texts(&bar).iter().any(|text| text == "1 selected"),
            "{:?}",
            support::texts(&bar)
        );
        assert!(row.press_at(30.0, 20.0));
        assert!(
            crate::settle_until(async || !bar.is_mapped()).await,
            "a second press unselects it"
        );
    });
}

/// Escape closes the dialogs a person opens over the list.
pub fn escape_closes_the_message_and_the_key_map() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j"]);
        let _ = window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
        assert!(window.reading().is_some());
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.reading().is_none_or(|reading| !reading.is_open()))
                .await,
            "Escape closes the open message"
        );
        support::press(&window, "question", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || window.key_map().is_some()).await,
            "? opens the key map"
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.key_map().is_none()).await,
            "Escape closes the key map"
        );
    });
}
