//! T193: the window's title-bar buttons hover in their own shape. The top
//! bar is a `CenterBox` whose ends fill its height, so an icon button left
//! at the default `valign` was laid out 26x46 and hovered as a tall pill,
//! whatever its `border-radius`; the buttons are centred at their own size.

use gtk::prelude::*;

use crate::support;

/// The populated inbox keeps GTK's native titlebar gestures, just as the
/// waiting page does: drag, double-click maximize and the window menu.
pub fn the_inbox_top_bar_is_a_window_handle() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = support::Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        crate::settle();
        let top = support::only(&window, "focus-top-bar");
        assert!(
            top.ancestor(gtk::WindowHandle::static_type()).is_some(),
            "the visible inbox header must support dragging and double-click maximize"
        );
    });
}

async fn maximize_and_restore(window: &postio_gtk::window::FocusWindow) {
    window.set_default_size(1024, 700);
    window.unmaximize();
    assert!(
        crate::settle_until(async || !window.is_maximized()
            && window.width() == 1024
            && window.height() == 700)
        .await,
        "the window never settled at its restored size"
    );
    let button = support::only(window, "focus-maximize")
        .downcast::<gtk::Button>()
        .expect("a button");
    assert_eq!(button.tooltip_text().as_deref(), Some("Maximize window"));
    support::click(window, &button, 1);
    assert!(
        crate::settle_until(async || window.is_maximized() && window.width() > 1024).await,
        "clicking the visible maximize button never maximized the window"
    );
    assert_eq!(button.tooltip_text().as_deref(), Some("Restore window"));
    assert_eq!(
        button.icon_name().as_deref(),
        Some("window-restore-symbolic")
    );
    support::click(window, &button, 1);
    assert!(
        crate::settle_until(async || !window.is_maximized()).await,
        "clicking restore never restored the window"
    );
    assert_eq!(button.tooltip_text().as_deref(), Some("Maximize window"));
}

/// Window controls remain available while opening, then on the inbox.
pub fn maximize_and_restore_work_before_and_after_mail_opens() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = postio_gtk::window::FocusWindow::new(None);
        window.set_default_size(1024, 700);
        window.present();
        maximize_and_restore(&window).await;
        window.destroy();

        let fixture = support::Fixture::empty().await;
        fixture
            .file(("Ada", "ada@example.com"), "Mail", "x", 10)
            .await;
        let (window, _client) = fixture.open_sized(Some((1024, 700))).await;
        maximize_and_restore(&window).await;
    });
}

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
