//! The pickers (specs/007-postio-focus T091, US5; screens 11-14): a small
//! popover anchored to the row, whose presets are chosen by number, whose
//! date field `Tab` reaches, and whose typed date is read on this machine.

use std::cell::Cell;
use std::rc::Rc;

use chrono::{DateTime, Local, TimeZone};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Keymap;
use postio_widgets::widgets::pickers::{When, WhenPicker};

use crate::support::until;

fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("an unambiguous local time")
}

/// A window with a row-sized button for the picker to hang from.
fn anchored() -> (gtk::Window, gtk::Button) {
    let anchor = gtk::Button::with_label("Ada Moreno \u{b7} Atlas Q3 budget");
    anchor.set_valign(gtk::Align::Start);
    let window = gtk::Window::builder()
        .default_width(700)
        .default_height(500)
        .child(&anchor)
        .build();
    window.present();
    (window, anchor)
}

/// US5 scenario 1 and T091: at Saturday 16:09 the snooze picker offers the
/// four presets with their times; `2` picks the second, and `Tab` reaches
/// the date field, where "tue 9am" is the coming Tuesday (scenario 2).
pub fn two_picks_the_second_preset_and_tab_focuses_the_date_entry() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, anchor) = anchored();
    let now = local_at(2026, 9, 26, 16, 9);
    let picker = WhenPicker::new(Keymap::defaults(), When::Snooze);
    let chosen: Rc<Cell<Option<DateTime<Local>>>> = Rc::default();
    picker.connect_chosen({
        let chosen = Rc::clone(&chosen);
        move |at| chosen.set(Some(at))
    });

    picker.open(&anchor, None, "Ada Moreno \u{b7} Atlas Q3 budget", now);
    assert!(
        until(|| picker.picker().is_shown()),
        "the picker never opened"
    );
    let said = picker.picker().texts();
    for wanted in [
        "Snooze until",
        "Ada Moreno \u{b7} Atlas Q3 budget",
        "Later today",
        "18:00",
        "Tomorrow morning",
        "Sun 27 Sep, 08:00",
        "Monday morning",
        "Mon 28 Sep, 08:00",
        "Next week",
        "Sat 3 Oct, 08:00",
        "1",
        "4",
    ] {
        assert!(
            said.iter().any(|line| line == wanted),
            "no {wanted:?} in {said:?}"
        );
    }

    assert!(
        picker
            .picker()
            .press(gdk::Key::_2, gdk::ModifierType::empty()),
        "2 is the picker's"
    );
    assert_eq!(
        chosen.get(),
        Some(local_at(2026, 9, 27, 8, 0)),
        "the second preset"
    );
    assert!(
        until(|| !picker.picker().is_open()),
        "choosing closes the picker"
    );

    chosen.set(None);
    picker.open(&anchor, None, "Ada Moreno \u{b7} Atlas Q3 budget", now);
    assert!(
        until(|| picker.picker().is_shown()),
        "the picker never reopened"
    );
    assert!(
        !picker.picker().field_has_keyboard(),
        "the presets have the keyboard first"
    );
    assert!(
        picker
            .picker()
            .press(gdk::Key::Tab, gdk::ModifierType::empty())
    );
    assert!(
        until(|| picker.picker().field_has_keyboard()),
        "Tab did not reach the date field"
    );
    picker.picker().entry().set_text("tue 9am");
    picker.picker().entry().emit_activate();
    assert_eq!(
        chosen.get(),
        Some(local_at(2026, 9, 29, 9, 0)),
        "tue 9am is the coming Tuesday"
    );
    window.destroy();
}
