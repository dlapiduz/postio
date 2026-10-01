//! T202: one rule for icon buttons, as T192 is for close. Every icon-only
//! button on every surface is the one `icon_button()` builds, and is laid
//! out at its own size -- centred where it stands, never stretched to its
//! row's height or its column's width -- so it hovers in its own shape. A
//! stretched one (T193's 26x46 pill in the top bar) cannot come back on a
//! surface this walks.

use adw::prelude::AdwDialogExt;
use gtk::prelude::*;

use crate::support::{self, Fixture};

/// Whether `widget` is a button that shows an icon and no words: a
/// `GtkButton` or a `GtkMenuButton`, not the toggle a menu button keeps
/// inside it.
fn is_icon_only(widget: &gtk::Widget) -> bool {
    if !(widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>()) {
        return false;
    }
    if widget.parent().is_some_and(|parent| parent.is::<gtk::MenuButton>()) {
        return false;
    }
    let inside = support::descendants(widget);
    let icon = inside
        .iter()
        .any(|child| child.is::<gtk::Image>() && child.is_visible());
    let words = inside.iter().any(|child| {
        child
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.is_visible() && !label.text().is_empty())
    });
    icon && !words
}

/// Every icon button on `surface` that is on screen.
fn icon_buttons(surface: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    support::descendants(surface)
        .into_iter()
        .filter(|widget| widget.is_mapped() && is_icon_only(widget))
        .collect()
}

/// What a button is called, for a failure to name it.
fn name_of(widget: &gtk::Widget) -> String {
    let tooltip = widget.tooltip_text().map(|text| text.to_string());
    let icon = widget
        .downcast_ref::<gtk::Button>()
        .and_then(|button| button.icon_name())
        .or_else(|| {
            widget
                .downcast_ref::<gtk::MenuButton>()
                .and_then(|button| button.icon_name())
        })
        .map(|icon| icon.to_string());
    format!("{tooltip:?} ({icon:?}, {:?})", widget.css_classes())
}

/// The rule, on one surface: at least one icon button (so a surface that
/// lost them all does not pass by finding none), each built by the shared
/// constructor and allocated exactly what it asks for.
fn assert_icon_buttons_keep_their_shape(name: &str, surface: &impl IsA<gtk::Widget>) {
    let found = icon_buttons(surface);
    assert!(!found.is_empty(), "{name}: no icon buttons on screen");
    for button in &found {
        let what = name_of(button);
        assert!(
            button.has_css_class("postio-icon-button"),
            "{name}: {what} is not the shared icon button"
        );
        let (_, high, _, _) = button.measure(gtk::Orientation::Vertical, button.width());
        let (_, wide, _, _) = button.measure(gtk::Orientation::Horizontal, -1);
        assert!(
            button.height() <= high + 1 && button.width() <= wide + 1,
            "{name}: {what} is laid out {}x{} where it asks for {wide}x{high}: \
             stretched, it hovers in its row's shape rather than its own",
            button.width(),
            button.height()
        );
    }
}

/// Wait for `surface` to lay its icon buttons out.
async fn laid_out(surface: &impl IsA<gtk::Widget>) {
    assert!(
        crate::settle_until(async || icon_buttons(surface)
            .first()
            .is_some_and(|button| button.width() > 0))
        .await,
        "the surface never laid out an icon button"
    );
}

pub fn every_surfaces_icon_buttons_keep_their_own_shape() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        fixture.label_in(message, "Harbor", "#3a7d44").await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || support::subjects(&window).len() == 1).await);
        window.set_default_size(1000, 640);
        crate::settle();

        // The window's top bar: compose, the main menu, close.
        assert_icon_buttons_keep_their_shape("the window", &window);

        // The key map.
        let keys = postio_focus::keymap_dialog::build(&window.keymap());
        keys.present(Some(&window));
        laid_out(&keys).await;
        assert_icon_buttons_keep_their_shape("the key map", &keys);
        keys.close();
        crate::settle();

        // The raw source.
        let source =
            postio_focus::source::dialog(b"From: a@example.com\r\n\r\nx", &window.keymap());
        source.present(Some(&window));
        laid_out(&source).await;
        assert_icon_buttons_keep_their_shape("the raw source", &source);
        source.close();
        crate::settle();

        // The open message: its steps and its close.
        support::keys(&window, &["j"]);
        let _ = window.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        let dialog = reading.dialog();
        laid_out(&dialog).await;
        assert_icon_buttons_keep_their_shape("the open message", &dialog);
        reading.close();
        crate::settle();

        // The composer, replying: close, detach, the recipient's and the
        // label's removes, and the formatting toolbar.
        window.act(postio_core::CommandId::Reply);
        assert!(crate::settle_until(async || window.compose_dialog().is_some()).await);
        let compose = window.compose_dialog().expect("the compose dialog");
        laid_out(&compose).await;
        assert_icon_buttons_keep_their_shape("the composer", &compose);
    });
}

pub fn the_digests_icon_buttons_keep_their_own_shape() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _, _) = crate::digest::delivered_holding().await;
        let (window, _client) = fixture.open().await;
        let digest = crate::digest::open_digest(&window).await;
        let dialog = digest.dialog().clone();
        laid_out(&dialog).await;
        assert_icon_buttons_keep_their_shape("the digest", &dialog);
    });
}
