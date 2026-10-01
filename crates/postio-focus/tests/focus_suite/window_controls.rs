//! T193: the window's title-bar buttons hover in their own shape. The top
//! bar is a `CenterBox` whose ends fill its height, so an icon button left
//! at the default `valign` was laid out 26x46 and hovered as a tall pill,
//! whatever its `border-radius`; the buttons are centred at their own size.

use gtk::prelude::*;

use crate::support;

/// The top bar's icon buttons (compose, the main menu, close) are laid out
/// at their own size, not stretched to the bar's height; close is square,
/// which `circular` then makes a circle.
pub fn the_top_bars_icon_buttons_hover_in_their_own_shape() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = support::Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        crate::settle();
        let top = support::only(&window, "focus-top-bar");
        let find = |class: &str| support::with_class(&top, class);
        let compose = find("focus-compose");
        let menu = find("focus-menu");
        let close = find("focus-close");
        for (name, widget) in [
            ("compose", compose.first()),
            ("menu", menu.first()),
            ("close", close.first()),
        ] {
            let widget = widget.unwrap_or_else(|| panic!("{name} is in the top bar"));
            assert!(
                widget.height() <= widget.width() + 2,
                "{name} is {}x{}: stretched to the bar's height it hovers as a tall pill",
                widget.width(),
                widget.height()
            );
        }
        let close = &close[0];
        assert_eq!(
            close.width(),
            close.height(),
            "close is a square hit target"
        );
    });
}
