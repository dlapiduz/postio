//! The classic app dresses the shared widgets in its own tokens (ADR 0043;
//! specs/007-postio-focus T016).
//!
//! Their rules moved from `shell.css` into postio-widgets' `widgets.css`.
//! What a person sees must not move with them: a primary button still wears
//! the accent's foreground, a ghost the dim ink, a line of key hints the
//! faint ink -- each compared with the token itself, resolved on a probe in
//! the same window, so the test holds whatever the design system says.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::widgets::button::{self, Kind, Size};
use postio_gtk::widgets::keyhint::KeyLine;
use postio_gtk::{fonts, style};

pub fn the_classic_app_dresses_the_shared_widgets() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    let _ = fonts::install();
    style::install(&display);
    let probes = gtk::CssProvider::new();
    probes.load_from_string(
        ".probe-accent-fg { color: var(--postio-accent-fg); }\n\
         .probe-dim { color: var(--postio-dim); }\n\
         .probe-faint { color: var(--postio-faint); }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &probes,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );

    let primary = button::button("Compose", Kind::Primary, Size::Regular);
    let ghost = button::button("Keys", Kind::Ghost, Size::Regular);
    let keys = KeyLine::new("probe-keys");
    let probe = |class: &str| {
        let label = gtk::Label::new(Some("x"));
        label.add_css_class(class);
        label
    };
    let (accent_fg, dim, faint) = (
        probe("probe-accent-fg"),
        probe("probe-dim"),
        probe("probe-faint"),
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for widget in [
        primary.upcast_ref::<gtk::Widget>(),
        ghost.upcast_ref(),
        keys.widget().upcast_ref(),
        accent_fg.upcast_ref(),
        dim.upcast_ref(),
        faint.upcast_ref(),
    ] {
        column.append(widget);
    }
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&column));
    window.present();
    crate::pump();

    assert_eq!(
        primary.color(),
        accent_fg.color(),
        "a primary button no longer wears the accent's foreground"
    );
    assert_eq!(
        ghost.color(),
        dim.color(),
        "a ghost button no longer wears the dim ink"
    );
    assert_eq!(
        keys.widget().color(),
        faint.color(),
        "a line of key hints no longer wears the faint ink"
    );

    window.destroy();
    gtk::style_context_remove_provider_for_display(&display, &probes);
}
