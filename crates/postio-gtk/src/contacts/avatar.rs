//! A person's avatar: the controller's initials or glyph on its colour, a
//! circle for a person and a rounded square for everything else.

use gtk::prelude::*;
use postio_ui::contact_popover::Avatar;

/// `avatar`, `size` pixels square.
pub fn widget(avatar: &Avatar, size: i32) -> gtk::Overlay {
    let ground = gtk::DrawingArea::new();
    ground.set_content_width(size);
    ground.set_content_height(size);
    ground.set_accessible_role(gtk::AccessibleRole::Presentation);
    let (r, g, b) = (
        f64::from((avatar.rgb >> 16) & 0xff) / 255.0,
        f64::from((avatar.rgb >> 8) & 0xff) / 255.0,
        f64::from(avatar.rgb & 0xff) / 255.0,
    );
    let round = avatar.round;
    ground.set_draw_func(move |_, cairo, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let radius = if round { w / 2.0 } else { w * 0.22 };
        cairo.set_source_rgb(r, g, b);
        cairo.new_sub_path();
        cairo.arc(
            w - radius,
            radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        cairo.arc(
            w - radius,
            h - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        cairo.arc(
            radius,
            h - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cairo.arc(
            radius,
            radius,
            radius,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
        cairo.close_path();
        let _ = cairo.fill();
    });
    let letters = gtk::Label::new(Some(&avatar.text));
    letters.add_css_class("focus-contact-avatar-text");
    letters.set_halign(gtk::Align::Center);
    letters.set_valign(gtk::Align::Center);
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("focus-contact-avatar");
    if size >= 48 {
        overlay.add_css_class("focus-contact-avatar-large");
    }
    overlay.set_child(Some(&ground));
    overlay.add_overlay(&letters);
    overlay.set_halign(gtk::Align::Start);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_accessible_role(gtk::AccessibleRole::Presentation);
    overlay
}
