//! The plain frame with the keyboard's place marked. It is drawn as an overlay on the render node,
//! so the plain frame that comparisons hash is unchanged.

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};

use super::deliver::keyboard_target;

const OUTLINE: gdk::RGBA = gdk::RGBA::new(1.0, 0.17, 0.84, 1.0);
const BORDER: f32 = 3.0;
const CAPTION_PAD: f32 = 4.0;

/// Falls back to the window alone, caption in its corner, when the keyboard is on nothing with
/// bounds.
/// Panics where a plain capture would.
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

/// The outline goes round the keyboard inside `other`; none is drawn on `window`.
pub fn outlined_with_window(
    window: &gtk::Window,
    other: &gtk::Window,
    region_name: &str,
) -> gdk::Texture {
    let main = crate::capture::texture_now(window).expect("a picture of the window");
    let second = crate::capture::texture_now(other).expect("a picture of the second window");
    let bounds = keyboard_target(other).compute_bounds(other);
    let layout = window.create_pango_layout(Some(region_name));
    let (_, text) = layout.pixel_extents();
    let plate_height = text.height() as f32 + 2.0 * CAPTION_PAD;
    let plate_width = text.width() as f32 + 2.0 * CAPTION_PAD;
    // Off to the right and down like a second window on a desktop, not centred like a dialog.
    let left = (window.width() - other.width() - 24).max(0) as f32;
    let top_of_other = ((window.height() - other.height()) as f32 - 24.0).max(48.0);
    let label = window.create_pango_layout(Some("a window of its own"));
    let (_, label_text) = label.pixel_extents();

    crate::capture::texture_with(window, |snapshot| {
        let whole = graphene::Rect::new(0.0, 0.0, window.width() as f32, window.height() as f32);
        snapshot.append_texture(&main, &whole);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(left, top_of_other));
        let own = graphene::Rect::new(0.0, 0.0, other.width() as f32, other.height() as f32);
        snapshot.append_outset_shadow(
            &gsk::RoundedRect::from_rect(own, 8.0),
            &gdk::RGBA::new(0.0, 0.0, 0.0, 0.35),
            0.0,
            6.0,
            4.0,
            18.0,
        );
        let tag_height = label_text.height() as f32 + 2.0 * CAPTION_PAD;
        let tag = graphene::Rect::new(
            0.0,
            -tag_height,
            label_text.width() as f32 + 2.0 * CAPTION_PAD,
            tag_height,
        );
        snapshot.append_color(&gdk::RGBA::new(0.2, 0.2, 0.2, 0.9), &tag);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(
            CAPTION_PAD,
            -tag_height + CAPTION_PAD,
        ));
        snapshot.append_layout(&label, &gdk::RGBA::WHITE);
        snapshot.restore();
        // The window's own paper, so the main window does not show through.
        let paper = if adw::StyleManager::default().is_dark() {
            gdk::RGBA::new(0.14, 0.14, 0.14, 1.0)
        } else {
            gdk::RGBA::new(0.98, 0.98, 0.98, 1.0)
        };
        snapshot.append_color(&paper, &own);
        snapshot.append_texture(&second, &own);
        snapshot.append_border(
            &gsk::RoundedRect::from_rect(own, 0.0),
            &[1.0; 4],
            &[gdk::RGBA::new(0.0, 0.0, 0.0, 0.6); 4],
        );
        let (x, top) = match &bounds {
            Some(bounds) => {
                snapshot.append_border(
                    &gsk::RoundedRect::from_rect(*bounds, 0.0),
                    &[BORDER; 4],
                    &[OUTLINE; 4],
                );
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
        snapshot.restore();
    })
    .expect("a picture of the windows")
}
