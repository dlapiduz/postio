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

/// US1 scenario 6: the rows, which draw themselves rather than wear CSS,
/// follow the switch at once too -- their ink, and a marker's accent, the
/// system's accent as libadwaita gives it for the scheme (FR-090, FR-091).
pub fn the_rows_repaint_in_dark_at_once_and_the_marker_keeps_the_accent() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let fixture = support::Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Budget",
                "Can you approve it?",
                5,
            )
            .await;
        fixture.ask(message, "Can you approve it?").await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        let drawn = || pane.rows_on_screen()[0].drawn();
        assert!(
            crate::settle_until(async || drawn().accent.is_some()).await,
            "the marked row was never drawn"
        );
        let light = drawn();
        assert_eq!(
            light.accent,
            Some(manager.accent_color().to_standalone_rgba(false)),
            "the marker is drawn in the system's accent"
        );

        manager.set_color_scheme(adw::ColorScheme::ForceDark);
        let followed = crate::settle_until(async || drawn().ink != light.ink).await;
        let dark = drawn();
        manager.set_color_scheme(adw::ColorScheme::Default);
        assert!(
            followed,
            "the row was not repainted when the scheme changed"
        );
        assert!(
            luminance(&dark.ink) > 0.5 && luminance(&light.ink) < 0.5,
            "light ink on dark, dark ink on light: {:?} then {:?}",
            light.ink,
            dark.ink
        );
        assert_eq!(
            dark.accent,
            Some(manager.accent_color().to_standalone_rgba(true)),
            "and the accent is libadwaita's for the dark scheme"
        );
    });
}
