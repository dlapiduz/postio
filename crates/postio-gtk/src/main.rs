//! Postio — a local-first, keyboard-first email client.
//!
//! This binary is the GTK4/libadwaita frontend. It must contain no SQL and no
//! IMAP: it talks to `postio-core` over commands and events only. See
//! `CLAUDE.md` for the architectural invariants CI enforces.

fn main() -> glib::ExitCode {
    postio_gtk::app::run()
}
