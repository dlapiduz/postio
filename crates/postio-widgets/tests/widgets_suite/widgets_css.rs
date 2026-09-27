//! The shared stylesheet dresses the shared widgets, in whatever colours the
//! app defines (specs/007-postio-focus research R11).
//!
//! The roles here are this test's own, and deliberately unlike any real
//! palette: what is asserted is that a control wears the *role*, so the same
//! sheet serves the classic app's tokens and Focus's libadwaita colours.
//! Asserted on the colour each widget resolves, which is what gets drawn.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint::KeyLine;

/// The roles an app defines, as colours no theme has.
const ROLES: &str = ":root {
  --postio-accent: rgb(10, 20, 30);
  --postio-accent-fg: rgb(200, 100, 50);
  --postio-accent-hover: rgb(11, 21, 31);
  --postio-ink: rgb(1, 2, 3);
  --postio-dim: rgb(90, 80, 70);
  --postio-faint: rgb(60, 110, 160);
  --postio-surface: rgb(250, 250, 250);
  --postio-hover-bg: rgb(240, 240, 240);
  --postio-hairline: rgb(220, 220, 220);
  --postio-hairline-strong: rgb(210, 210, 210);
}";

fn rgb(color: gdk::RGBA) -> (u8, u8, u8) {
    let byte = |channel: f32| (channel * 255.0).round() as u8;
    (byte(color.red()), byte(color.green()), byte(color.blue()))
}

pub fn the_shared_sheet_dresses_the_shared_widgets() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    let roles = gtk::CssProvider::new();
    roles.load_from_string(ROLES);
    gtk::style_context_add_provider_for_display(
        &display,
        &roles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    postio_widgets::style::register();
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    let sheet = gtk::CssProvider::new();
    sheet.connect_parsing_error({
        let errors = errors.clone();
        move |_, section, error| {
            errors
                .borrow_mut()
                .push(format!("{}: {error}", section.to_str()));
        }
    });
    sheet.load_from_resource(postio_widgets::style::WIDGETS_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &sheet,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    assert!(
        errors.borrow().is_empty(),
        "widgets.css does not parse in GTK's CSS subset: {:#?}",
        errors.borrow()
    );

    let primary = button::button("Send", Kind::Primary, Size::Regular);
    let ghost = button::button("Later", Kind::Ghost, Size::Regular);
    let keys = KeyLine::new("probe-keys");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&primary);
    column.append(&ghost);
    column.append(keys.widget());
    let window = gtk::Window::new();
    window.set_child(Some(&column));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    assert_eq!(
        rgb(primary.color()),
        (200, 100, 50),
        "a primary button does not wear --postio-accent-fg"
    );
    assert_eq!(
        rgb(ghost.color()),
        (90, 80, 70),
        "a ghost button does not wear --postio-dim"
    );
    assert_eq!(
        rgb(keys.widget().color()),
        (60, 110, 160),
        "a line of key hints does not wear --postio-faint"
    );

    window.destroy();
    gtk::style_context_remove_provider_for_display(&display, &sheet);
    gtk::style_context_remove_provider_for_display(&display, &roles);
}
