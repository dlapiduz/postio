//! Postio — a local-first, keyboard-first email client.
//!
//! This binary is the GTK4/libadwaita frontend. It must contain no SQL and no
//! IMAP: it talks to `postio-core` over commands and events only. See
//! `CLAUDE.md` for the architectural invariants CI enforces.
//!
//! The window itself is not built yet. What runs today is the foundation the
//! rest of the UI hangs off: the embedded fonts and the design tokens
//! generated from the Industry design system.

use gtk::gdk;

fn main() {
    if adw::init().is_err() {
        eprintln!("postio: no display; the UI needs a Wayland or X11 session");
        return;
    }

    // Fonts first: a PangoContext keeps the family it has already resolved, so
    // faces registered after the first widget would not reach it.
    match postio_gtk::fonts::install() {
        Ok(faces) => println!("postio: {} embedded font faces registered", faces.len()),
        Err(error) => eprintln!("postio: {error}"),
    }

    if let Some(display) = gdk::Display::default() {
        postio_gtk::style::install(&display);
        println!("postio: design tokens loaded");
    }

    println!("postio: UI not implemented yet");
}
