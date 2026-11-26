use gtk::prelude::*;
use postio_widgets::capture;
use postio_widgets::storyboard::outline::outlined;

use super::storyboard_support::{show, until};

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

    fn differing(&self, other: &Self, top: usize, bottom: usize) -> usize {
        let row = self.width * 4;
        let (mine, _) = self.bytes[top * row..bottom * row].as_chunks::<4>();
        let (theirs, _) = other.bytes[top * row..bottom * row].as_chunks::<4>();
        mine.iter().zip(theirs).filter(|(a, b)| a != b).count()
    }
}

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

fn plain(window: &gtk::Window) -> gtk::gdk::Texture {
    let mut last = None;
    for _ in 0..4 {
        super::storyboard_support::run_for(std::time::Duration::from_millis(40));
        last = Some(capture::texture_now(window).expect("a picture"));
    }
    last.expect("a frame")
}

pub fn the_focused_widgets_bounds_are_drawn_over() {
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
    let (window, button) = window();
    let before = Pixels::of(&plain(&window));
    let bounds = button.compute_bounds(&window).expect("bounds");
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
    let (window, _button) = window();
    let hash = |texture: &gtk::gdk::Texture| blake3::hash(&Pixels::of(texture).bytes);
    let before = hash(&plain(&window));

    let _ = outlined(&window, "list");
    let after = hash(&capture::texture_now(&window).expect("a picture"));

    assert_eq!(before, after, "drawing the outline changed the plain frame");
}

const PROBE: [u8; 4] = [0, 255, 0, 255];
const MAGENTA: [u8; 4] = [255, 43, 214, 255];

fn a_green_field_in_a_popover() -> gtk::Window {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".outline-probe { background: #00ff00; border: none; box-shadow: none; outline: none; }",
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("a display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );
    let window = gtk::Window::new();
    window.set_decorated(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let anchor = gtk::MenuButton::new();
    anchor.set_label("places");
    anchor.set_margin_top(40);
    anchor.set_halign(gtk::Align::Center);
    let probe = gtk::Button::new();
    probe.add_css_class("outline-probe");
    probe.set_size_request(160, 40);
    let inside = gtk::Box::new(gtk::Orientation::Vertical, 12);
    inside.append(&gtk::Label::new(Some("above the field")));
    inside.append(&probe);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&inside));
    anchor.set_popover(Some(&popover));
    content.append(&anchor);
    window.set_child(Some(&content));
    window.set_default_size(480, 360);
    show(&window);
    popover.popup();
    assert!(until(|| popover.is_mapped() && probe.width() > 0));
    probe.grab_focus();
    assert!(until(
        || GtkWindowExt::focus(&window).as_ref() == Some(probe.upcast_ref())
    ));
    window
}

pub fn the_outline_sits_on_a_focused_widget_inside_a_popover() {
    let window = a_green_field_in_a_popover();
    let before = Pixels::of(&plain(&window));
    let height = before.bytes.len() / 4 / before.width;
    let green: Vec<(usize, usize)> = (0..height)
        .flat_map(|y| (0..before.width).map(move |x| (x, y)))
        .filter(|&(x, y)| before.at(x, y) == PROBE)
        .collect();
    assert!(!green.is_empty(), "the popover's probe is not in the frame");
    let left = green.iter().map(|p| p.0).min().expect("a pixel");
    let right = green.iter().map(|p| p.0).max().expect("a pixel");
    let top = green.iter().map(|p| p.1).min().expect("a pixel");
    let bottom = green.iter().map(|p| p.1).max().expect("a pixel");

    let after = Pixels::of(&outlined(&window, "places"));

    let (middle_x, middle_y) = ((left + right) / 2, (top + bottom) / 2);
    for (edge, x, y) in [
        ("left", left, middle_y),
        ("right", right, middle_y),
        ("bottom", middle_x, bottom),
    ] {
        assert_eq!(
            after.at(x, y),
            MAGENTA,
            "the outline misses the focused widget's {edge} edge at ({x}, {y}); \
             the widget is drawn at x {left}..={right}, y {top}..={bottom}"
        );
    }
    assert_eq!(
        after.at(middle_x, bottom + 6),
        before.at(middle_x, bottom + 6),
        "the outline runs on below the focused widget"
    );
}
