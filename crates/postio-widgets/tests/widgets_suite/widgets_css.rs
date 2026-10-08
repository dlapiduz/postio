//! The shared stylesheet dresses the shared widgets, in whatever colours the
//! app defines (specs/007-postio-focus research R11).
//!
//! The roles here are this test's own, and deliberately unlike any real
//! palette: what is asserted is that a control wears the *role*, so the same
//! sheet serves any palette, Focus's libadwaita colours among them.
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

/// The shared sheet brings the metrics every app lays these widgets out by:
/// spacing, chip sizes and type sizes. An app defines only its colours
/// (research R11), so with nothing but this test's colour roles, each metric
/// must still resolve -- read back as the size it gives a probe.
pub fn the_shared_sheet_brings_the_shared_metrics() {
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
    let sheet = postio_widgets::style::install(&display);
    let probes = gtk::CssProvider::new();
    probes.load_from_string(
        ".probe-space { min-width: var(--postio-space-3); }\n\
         .probe-chip { min-width: var(--postio-chip-height); }\n\
         .probe-role { font-size: var(--postio-text-title); }\n\
         .probe-literal { font-size: 1.3636rem; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &probes,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );

    let probe = |class: &str, text: &str| {
        let label = gtk::Label::new(Some(text));
        label.add_css_class(class);
        label
    };
    let (space, chip) = (probe("probe-space", ""), probe("probe-chip", ""));
    let (role, literal) = (
        probe("probe-role", "Inbox is empty"),
        probe("probe-literal", "Inbox is empty"),
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for label in [&space, &chip, &role, &literal] {
        label.set_halign(gtk::Align::Start);
        column.append(label);
    }
    let window = gtk::Window::new();
    window.set_child(Some(&column));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    let width = |label: &gtk::Label| label.measure(gtk::Orientation::Horizontal, -1).0;
    assert_eq!(
        width(&space),
        10,
        "--postio-space-3 does not resolve to 10px"
    );
    assert_eq!(
        width(&chip),
        22,
        "--postio-chip-height does not resolve to 22px"
    );
    assert_eq!(
        width(&role),
        width(&literal),
        "--postio-text-title does not resolve to its size"
    );

    window.destroy();
    for provider in [&probes, &sheet, &roles] {
        gtk::style_context_remove_provider_for_display(&display, provider);
    }
}

/// Every widget under `root` wearing `class`.
fn wearing(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if root.has_css_class(class) {
        found.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        found.extend(wearing(&current, class));
        child = current.next_sibling();
    }
    found
}

/// The account form is both apps' (T165), so the shared sheet dresses it:
/// in an app that defines only the roles -- Focus, opening it from its
/// sign-in banner -- the refusal still reads as the callout, and the step
/// as the dim line it is.
pub fn the_shared_sheet_dresses_the_account_form() {
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
    let sheet = postio_widgets::style::install(&display);

    let form = postio_widgets::onboarding::Onboarding::new();
    form.set_status(postio_widgets::onboarding::Status::Failed(
        "The server refused the password.".to_owned(),
    ));
    let window = gtk::Window::new();
    window.set_child(Some(&form));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    let root: gtk::Widget = form.clone().upcast();
    let refusal = wearing(&root, "postio-callout");
    assert_eq!(refusal.len(), 1, "one callout on the form");
    assert_eq!(
        rgb(refusal[0].color()),
        (1, 2, 3),
        "the refusal does not wear the callout's --postio-ink"
    );
    let step = wearing(&root, "postio-onboarding-step");
    assert_eq!(
        rgb(step[0].color()),
        (90, 80, 70),
        "the step does not wear --postio-dim"
    );

    window.destroy();
    gtk::style_context_remove_provider_for_display(&display, &sheet);
    gtk::style_context_remove_provider_for_display(&display, &roles);
}
