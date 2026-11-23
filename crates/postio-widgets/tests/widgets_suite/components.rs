use gtk::gdk;
use gtk::prelude::*;

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

pub fn every_settings_heading_keeps_one_rhythm() {
    let Some(window) = window() else {
        return;
    };
    let panel = postio_widgets::settings::SettingsPanel::new();
    window.set_child(Some(&panel));
    window.present();
    pump();
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
