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

/// The frame of a window with a second one open beside it: `window`'s picture
/// with `other` laid over it, centred, and the outline round the keyboard
/// inside `other`.
///
/// A camera on one window cannot show a composition detached into a window
/// of its own, and the frame that does not show it cannot say the keyboard
/// went with it. The outline is never drawn on `window` here: the keyboard
/// is not in it.
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
    let left = ((window.width() - other.width()) as f32 / 2.0).max(0.0);
    let top_of_other = ((window.height() - other.height()) as f32 / 2.0).max(0.0);

    crate::capture::texture_with(window, |snapshot| {
        // Over the whole picture: the main window's own is replaced, so no
        // outline of its focus shows through.
        let whole = graphene::Rect::new(0.0, 0.0, window.width() as f32, window.height() as f32);
        snapshot.append_texture(&main, &whole);
        snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.25), &whole);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(left, top_of_other));
        let own = graphene::Rect::new(0.0, 0.0, other.width() as f32, other.height() as f32);
        // The window's own paper: a window's picture is its content, and
        // under it the main window would show through.
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
