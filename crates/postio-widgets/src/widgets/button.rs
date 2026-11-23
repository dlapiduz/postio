//! Buttons: four `Kind`s (primary, secondary, ghost, destructive) and two `Size`s, one look each.
//! `icon_button` takes the accessible name as a required argument and uses it for the tooltip.

use adw::prelude::*;
use gtk::gio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Regular,
}

impl Kind {
    fn class(self) -> &'static str {
        match self {
            Kind::Primary => "postio-button-primary",
            Kind::Secondary => "postio-button-secondary",
            Kind::Ghost => "postio-button-ghost",
            Kind::Destructive => "postio-button-destructive",
        }
    }
}

impl Size {
    fn class(self) -> &'static str {
        match self {
            Size::Small => "postio-button-small",
            Size::Regular => "postio-button-regular",
        }
    }
}

const KINDS: [Kind; 4] = [
    Kind::Primary,
    Kind::Secondary,
    Kind::Ghost,
    Kind::Destructive,
];
const SIZES: [Size; 2] = [Size::Small, Size::Regular];

/// Works on any button-shaped widget; the look is classes, so a surface can promote a button by
/// calling this again.
pub fn style(button: &impl IsA<gtk::Widget>, kind: Kind, size: Size) {
    let widget: &gtk::Widget = button.upcast_ref();
    widget.add_css_class("postio-button");
    for other in KINDS {
        widget.remove_css_class(other.class());
    }
    for other in SIZES {
        widget.remove_css_class(other.class());
    }
    widget.add_css_class(kind.class());
    widget.add_css_class(size.class());
}

pub fn button(label: &str, kind: Kind, size: Size) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    style(&button, kind, size);
    button
}

/// `name` is the tooltip and accessible name. Centred at its own size: at the default `Fill` a
/// taller bar stretches it into a tall pill.
pub fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    dress_icon(&button, name);
    button
}

pub fn icon_menu_button(icon: &str, name: &str, menu: &gio::MenuModel) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .icon_name(icon)
        .menu_model(menu)
        .build();
    dress_icon(&button, name);
    button
}

/// For a button whose icon is set elsewhere, such as a toolbar's, not built by `icon_button`.
pub fn dress_icon(button: &impl IsA<gtk::Widget>, name: &str) {
    let widget: &gtk::Widget = button.upcast_ref();
    widget.add_css_class("postio-icon-button");
    widget.set_tooltip_text(Some(name));
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
    widget.set_valign(gtk::Align::Center);
    widget.set_halign(gtk::Align::Center);
}

/// One constructor so no surface draws its own; the class is what the surface-walking test finds.
pub fn close_button() -> gtk::Button {
    let button = icon_button("window-close-symbolic", "Close");
    button.add_css_class("postio-close-button");
    button
}
