//! The outlined frame: the plain frame with the keyboard's place marked.
//!
//! The reviewer and the maintainer read this one, because it says where the
//! keyboard is and which region of the app that is. It is drawn as an
//! overlay on the window's render node, never into the widgets, so the
//! plain frame — the one a base-versus-branch comparison hashes — is not
//! changed by being outlined, and an outline that moves with focus is not a
//! visual change.

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};

use super::deliver::keyboard_target;

/// The colour of the outline and the caption's plate: loud on purpose, and
/// not one the design uses.
const OUTLINE: gdk::RGBA = gdk::RGBA::new(1.0, 0.17, 0.84, 1.0);
const BORDER: f32 = 3.0;
const CAPTION_PAD: f32 = 4.0;

/// The window's picture with a border drawn round the widget the keyboard is
/// on and a caption naming `region_name`.
///
/// Falls back to a picture of the window alone, with the caption in its
/// corner, when the keyboard is on nothing with bounds; an outline that
/// cannot be drawn is itself information. Panics only where a plain capture
/// would: a window the compositor never showed.
pub fn outlined(window: &gtk::Window, region_name: &str) -> gdk::Texture {
    let bounds = keyboard_target(window).compute_bounds(window);
    let layout = window.create_pango_layout(Some(region_name));
    let (_, text) = layout.pixel_extents();
    let plate_height = text.height() as f32 + 2.0 * CAPTION_PAD;
    let plate_width = text.width() as f32 + 2.0 * CAPTION_PAD;

    crate::capture::texture_with(window, |snapshot| {
        let (x, top) = match &bounds {
            Some(bounds) => {
                let rect = gsk::RoundedRect::from_rect(*bounds, 0.0);
                snapshot.append_border(&rect, &[BORDER; 4], &[OUTLINE; 4]);
                // Above the border when there is room, inside it when not.
                let above = bounds.y() - plate_height;
                (bounds.x(), if above >= 0.0 { above } else { bounds.y() })
            }
            None => (0.0, 0.0),
        };
        snapshot.append_color(
            &OUTLINE,
            &graphene::Rect::new(x, top, plate_width, plate_height),
        );
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x + CAPTION_PAD, top + CAPTION_PAD));
        snapshot.append_layout(&layout, &gdk::RGBA::WHITE);
        snapshot.restore();
    })
    .expect("a picture of the window")
}
