//! Focus's colours are libadwaita's, through Focus's own roles, and follow
//! the system's light and dark at once (T040; research R11, FR-090).
//!
//! What is read is the colour GTK resolved for a widget on screen: the row's
//! ink (`--postio-ink`) and the strip's counts (`--postio-dim`). A role Focus
//! did not define resolves to nothing, and the widget falls back to the ink
//! it inherits -- which is what the first assertion tells apart.

use gtk::prelude::*;

use crate::support::{self, only};

fn luminance(colour: &gtk::gdk::RGBA) -> f32 {
    0.2126 * colour.red() + 0.7152 * colour.green() + 0.0722 * colour.blue()
}

pub fn the_roles_resolve_and_follow_the_system_into_dark() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let (_fixture, window) = support::three_in_the_inbox().await;
        crate::settle();

        let row = window
            .pane()
            .expect("the inbox is showing")
            .rows_on_screen()
            .into_iter()
            .next()
            .expect("a row on screen");
        let counts = only(&window, "focus-counts")
            .first_child()
            .expect("the counts label");
        let (ink, dim) = (row.color(), counts.color());
        assert!(
            luminance(&ink) < 0.5,
            "light ink is dark on a light view: {ink:?}"
        );
        assert!(
            dim.alpha() < ink.alpha() * 0.8,
            "the counts wear Focus's dim role, clearly fainter than the ink \
             they would inherit: dim {dim:?}, ink {ink:?}"
        );

        manager.set_color_scheme(adw::ColorScheme::ForceDark);
        let followed = crate::settle_until(async || row.color() != ink).await;
        let (dark_ink, dark_dim) = (row.color(), counts.color());
        manager.set_color_scheme(adw::ColorScheme::Default);
        assert!(followed, "the row's ink did not follow the switch to dark");
        assert!(
            luminance(&dark_ink) > 0.5,
            "dark ink is light on a dark view: {dark_ink:?}"
        );
        assert_ne!(dim, dark_dim, "the dim role follows the switch too");
    });
}
