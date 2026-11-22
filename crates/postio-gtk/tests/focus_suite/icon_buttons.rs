//! Every icon-only button is the one `icon_button()` builds and is laid out at its own size, never
//! stretched to its row or column.

use adw::prelude::AdwDialogExt;
use gtk::prelude::*;

use crate::support::{self, Fixture};

fn is_icon_only(widget: &gtk::Widget) -> bool {
    if !(widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>()) {
        return false;
    }
    if widget
        .parent()
        .is_some_and(|parent| parent.is::<gtk::MenuButton>())
    {
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

fn icon_buttons(surface: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    support::descendants(surface)
        .into_iter()
        .filter(|widget| widget.is_mapped() && is_icon_only(widget))
        .collect()
}

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

        assert_icon_buttons_keep_their_shape("the window", &window);

        let keys = postio_gtk::keymap_dialog::build(&window.keymap());
        keys.present(Some(&window));
        laid_out(&keys).await;
        assert_icon_buttons_keep_their_shape("the key map", &keys);
        keys.close();
        crate::settle();

        let source = postio_gtk::source::dialog(b"From: a@example.com\r\n\r\nx", &window.keymap());
        source.present(Some(&window));
        laid_out(&source).await;
        assert_icon_buttons_keep_their_shape("the raw source", &source);
        source.close();
        crate::settle();

        support::keys(&window, &["j"]);
        let _ = window.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        let dialog = reading.dialog();
        laid_out(&dialog).await;
        assert_icon_buttons_keep_their_shape("the open message", &dialog);
        reading.close();
        crate::settle();

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

pub fn the_settings_icon_buttons_keep_their_own_shape() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, _path) = crate::settings::one_message_under(
            "[saved_searches.receipts]\nquery = \"in:Receipts\"\npinned = true\n",
        )
        .await;
        support::deliver_with(&window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
        let dialog = crate::settings::settings_shown(&window)
            .await
            .expect("Settings opened");
        laid_out(&dialog).await;
        assert_icon_buttons_keep_their_shape("Settings", &dialog);
        let filters = crate::settings::section_rows(&dialog)
            .into_iter()
            .find(|(name, _)| name == "Saved searches")
            .map(|(_, row)| row)
            .expect("Filters is listed");
        support::click(&window, &filters, 1);
        assert!(
            crate::settle_until(async || support::with_class(&dialog, "postio-icon-button")
                .iter()
                .filter(|button| button.is_mapped() && button.width() > 0)
                .count()
                > 1)
            .await,
            "the saved search's own icon buttons are drawn"
        );
        assert_icon_buttons_keep_their_shape("Settings' Saved searches", &dialog);
    });
}
