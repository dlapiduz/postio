//! The outlined frame (spec 008, research R4): the plain frame, with a
//! border round the widget the keyboard is on and a caption naming its
//! region. Written beside the plain frame, never instead of it, because the
//! base-versus-branch comparison hashes the plain one and an outline that
//! moves with focus would otherwise count as a visual change.

use gtk::prelude::*;
use postio_gtk::capture;
use postio_gtk::storyboard::outline::outlined;

use super::storyboard_support::{display, show, until};

struct Pixels {
    bytes: Vec<u8>,
    width: usize,
}

impl Pixels {
    fn of(texture: &gtk::gdk::Texture) -> Self {
        let mut downloader = gtk::gdk::TextureDownloader::new(texture);
        downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8Premultiplied);
        let (bytes, _) = downloader.download_bytes();
        Self {
            bytes: bytes.to_vec(),
            width: texture.width() as usize,
        }
    }

    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let at = (y * self.width + x) * 4;
        self.bytes[at..at + 4].try_into().expect("a pixel")
    }

    /// How many pixels in the rows `top..bottom` differ from `other`'s.
    fn differing(&self, other: &Self, top: usize, bottom: usize) -> usize {
        let row = self.width * 4;
        let (mine, _) = self.bytes[top * row..bottom * row].as_chunks::<4>();
        let (theirs, _) = other.bytes[top * row..bottom * row].as_chunks::<4>();
        mine.iter().zip(theirs).filter(|(a, b)| a != b).count()
    }
}

/// A window with a button low in the content, which holds the keyboard.
fn window() -> (gtk::Window, gtk::Button) {
    let window = gtk::Window::new();
    window.set_decorated(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&gtk::Label::new(Some("above")));
    let button = gtk::Button::with_label("the focused one");
    button.set_margin_top(60);
    button.set_margin_start(40);
    button.set_margin_end(40);
    button.set_vexpand(true);
    button.set_valign(gtk::Align::Start);
    content.append(&button);
    window.set_child(Some(&content));
    window.set_default_size(320, 220);
    show(&window);
    button.grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));
    assert!(until(|| button.width() > 0));
    (window, button)
}

/// Settle on a stable frame to compare against.
fn plain(window: &gtk::Window) -> gtk::gdk::Texture {
    let mut last = None;
    for _ in 0..4 {
        super::storyboard_support::run_for(std::time::Duration::from_millis(40));
        last = Some(capture::texture_now(window).expect("a picture"));
    }
    last.expect("a frame")
}

pub fn the_focused_widgets_bounds_are_drawn_over() {
    if !display() {
        return;
    }
    let (window, button) = window();
    let before = Pixels::of(&plain(&window));

    let after = Pixels::of(&outlined(&window, "list"));

    let bounds = button
        .compute_bounds(&window)
        .expect("the button has bounds in the window");
    let (x, y) = (bounds.x() as usize, bounds.y() as usize);
    let middle = y + bounds.height() as usize / 2;
    assert_ne!(
        before.at(x, middle),
        after.at(x, middle),
        "no border at the focused widget's left edge"
    );
    assert_ne!(
        before.at(x + bounds.width() as usize - 1, middle),
        after.at(x + bounds.width() as usize - 1, middle),
        "no border at the focused widget's right edge"
    );
}

pub fn the_caption_carries_the_region_name() {
    if !display() {
        return;
    }
    let (window, button) = window();
    let before = Pixels::of(&plain(&window));
    let bounds = button.compute_bounds(&window).expect("bounds");
    // Rows well clear of the border, so only a caption can differ there.
    let rows_above = (bounds.y() as usize).saturating_sub(34)..(bounds.y() as usize - 2);

    let short = Pixels::of(&outlined(&window, "list"));
    let long = Pixels::of(&outlined(&window, "a rather longer region name"));

    assert!(
        short.differing(&before, rows_above.start, rows_above.end) > 0,
        "no caption drawn above the outline"
    );
    assert!(
        short.differing(&long, rows_above.start, rows_above.end) > 0,
        "the caption does not depend on the region name"
    );
}

pub fn the_plain_frame_is_untouched() {
    if !display() {
        return;
    }
    let (window, _button) = window();
    let hash = |texture: &gtk::gdk::Texture| blake3::hash(&Pixels::of(texture).bytes);
    let before = hash(&plain(&window));

    let _ = outlined(&window, "list");
    let after = hash(&capture::texture_now(&window).expect("a picture"));

    assert_eq!(before, after, "drawing the outline changed the plain frame");
}
