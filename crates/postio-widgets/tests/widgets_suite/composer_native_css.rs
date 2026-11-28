use gtk::prelude::*;
use postio_body::{DocPosition, Document, EditIntent};
use postio_widgets::composer::surface::ComposerSurface;

use super::composer_native_input::shown;
use crate::support_compose::{settle, style};

pub fn a_scheme_change_restyles_and_keeps_the_caret_and_the_history() {
    let display = gtk::gdk::Display::default().unwrap();
    style::install(&display);
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    settle();
    let shown = shown(Document::from_text("hello"));
    shown.at(&[0], 5);
    assert!(shown.surface.dispatch(EditIntent::Insert("!".into())));
    let light = shown.surface.view().color();

    manager.set_color_scheme(adw::ColorScheme::ForceDark);
    settle();
    let dark = shown.surface.view().color();
    assert_ne!(light, dark, "the ink follows the scheme");
    assert_eq!(
        shown.surface.selection().head,
        DocPosition::new([0], 6),
        "the caret stays"
    );
    assert!(shown.surface.can_undo(), "and so does the history");
    shown.surface.undo();
    assert_eq!(shown.surface.document(), Document::from_text("hello"));
    manager.set_color_scheme(adw::ColorScheme::Default);
}
