//! Buttons: four kinds, two sizes, one look each.
//!
//! Postio had six looks for a button and no rule for which to use. A primary
//! was libadwaita's bare `.suggested-action` pill in the header, the composer,
//! onboarding and search; a pale accent tint inside an action bar; and a
//! solid square fill in settings -- three answers to "this is the one", on
//! one screen at a time. Secondaries were three bordered variants of slightly
//! different radius and weight, plus the parts panel's own.
//!
//! The design system has one answer (`Design/_ds`, `.btn-*`), and this is
//! it:
//!
//! * [`Kind::Primary`] -- the one verb a surface exists for. Solid accent,
//!   square-cornered, identical wherever it appears.
//! * [`Kind::Secondary`] -- a verb beside it. Hairline-bordered, no fill.
//! * [`Kind::Ghost`] -- chrome: the header's `Keys`, the bulk bar. No border,
//!   quiet ink, a tint under the pointer.
//! * [`Kind::Destructive`] -- a verb that loses something. Bordered in the
//!   destructive colour, never filled: it should be findable, not inviting.
//!
//! [`Size::Regular`] is the canvas' 12.5px label; [`Size::Small`] the 11.5px
//! one rows and panels use. Corners are `radius-sm`, as every control's are.
//!
//! [`icon_button`] is the icon-only case, which is where the accessible name
//! is most often forgotten: it takes the name as a required argument and uses
//! it for the tooltip too.

use adw::prelude::*;

/// What a button is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one verb a surface exists for.
    Primary,
    /// A verb beside the primary.
    Secondary,
    /// Chrome: present, quiet, a tint under the pointer.
    Ghost,
    /// A verb that loses something.
    Destructive,
}

/// How much room a button takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// A panel's or a row's button.
    Small,
    /// A surface's own actions.
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

/// Give `button` a kind and a size, replacing any it had.
///
/// Works on anything button-shaped -- a `GtkButton`, a `GtkMenuButton`, a
/// `GtkToggleButton` -- because the look is classes, and a surface that
/// promotes one of its buttons to primary calls this again.
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

/// A labelled button of `kind` and `size`.
pub fn button(label: &str, kind: Kind, size: Size) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    style(&button, kind, size);
    button
}

/// An icon-only button. `name` is required: it is the tooltip and the
/// accessible name, and an icon button without one is a button a screen
/// reader calls "button".
pub fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("postio-icon-button");
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}
