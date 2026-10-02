//! The window's size across a restart (row 10, T246): Focus reopens at the
//! size it was closed at, and a state file it cannot use opens it at the
//! default.
//!
//! The file is `$XDG_STATE_HOME/postio/window.ini`; the suite runs with
//! `XDG_STATE_HOME` in a directory of its own (`hermetic`), so these cases
//! write the real path without touching the developer's. Each removes what
//! it wrote, because a later case in the same process opens a window too.

use adw::prelude::*;
use postio_focus::window::FocusWindow;
use postio_widgets::state::path;

use crate::support;

/// Focus's first-run size: what a window with nothing saved opens at.
const DEFAULT: (i32, i32) = (1440, 900);

/// Runs `body`, then clears the state file whatever it did.
async fn with_clean_state(body: impl std::future::Future<Output = ()>) {
    let _ = std::fs::remove_file(path());
    body.await;
    let _ = std::fs::remove_file(path());
}

fn write_state(text: &str) {
    std::fs::create_dir_all(path().parent().expect("a state directory")).expect("created");
    std::fs::write(path(), text).expect("a state file");
}

pub fn a_restarted_window_opens_at_the_size_it_was_closed_at() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        with_clean_state(async {
            let first = FocusWindow::new(None);
            assert_eq!(
                first.default_size(),
                DEFAULT,
                "nothing saved opens at the default"
            );
            first.set_default_size(1111, 703);
            first.present();
            crate::settle();
            first.close();
            crate::settle();

            let second = FocusWindow::new(None);
            assert_eq!(
                second.default_size(),
                (1111, 703),
                "the window did not come back at the size it closed at"
            );
        })
        .await;
    });
}

pub fn a_state_file_that_cannot_be_read_opens_at_the_default() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        with_clean_state(async {
            write_state("this is not a key file at all\u{1}");
            let corrupt = FocusWindow::new(None);
            assert_eq!(corrupt.default_size(), DEFAULT, "a corrupt file");
            assert!(!corrupt.is_maximized());

            write_state("[Window]\nwidth=0\nheight=99999999\n");
            let absurd = FocusWindow::new(None);
            assert_eq!(absurd.default_size(), DEFAULT, "sizes no display has");

            let _ = std::fs::remove_file(path());
            let missing = FocusWindow::new(None);
            assert_eq!(missing.default_size(), DEFAULT, "no file at all");
        })
        .await;
    });
}
