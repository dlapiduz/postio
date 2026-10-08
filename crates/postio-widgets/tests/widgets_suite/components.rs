//! The shared components, measured the way a person would see them rather
//! than by what they were handed.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;

/// A presented window to put components in, or `None` with no display.
fn window() -> Option<gtk::Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    crate::support_reader::prepare(&gdk::Display::default().unwrap());
    Some(gtk::Window::new())
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

/// Every widget under `root`, depth first.
fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        out.push(widget.clone());
        out.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    out
}

/// Every settings pane keeps one rhythm: a section heading sits one `S6`
/// below what came before it, or flush at the top. The panes had been
/// written with 18, 20 and 22, so three panes of one window had three.
pub fn every_settings_heading_keeps_one_rhythm() {
    let Some(window) = window() else {
        return;
    };
    let panel = postio_widgets::settings::SettingsPanel::new();
    window.set_child(Some(&panel));
    window.present();
    pump();
    // Panes are built the first time they are shown.
    for section in postio_widgets::settings::Section::ALL {
        panel.show_section(section);
        pump();
    }

    let allowed = [0, postio_widgets::widgets::space::S6];
    let mut seen = 0;
    for widget in descendants(panel.upcast_ref()) {
        // The sidebar's group headings belong to the list's header func,
        // not to a pane's column.
        if !widget.has_css_class("postio-kicker")
            || widget.has_css_class("postio-settings-nav-heading")
        {
            continue;
        }
        seen += 1;
        let label = widget.downcast_ref::<gtk::Label>().unwrap().label();
        assert!(
            allowed.contains(&widget.margin_top()),
            "`{label}` sits {}px below its neighbour; the rhythm is {allowed:?}",
            widget.margin_top()
        );
    }
    assert!(seen >= 8, "found only {seen} section headings");
    window.destroy();
}
