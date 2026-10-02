//! A window wired by Focus's real startup still frees when it is destroyed
//! (#1072).
//!
//! The cycle is the one #794 catalogued: a handler stored on a child widget
//! holding a strong reference back to the window that owns it. A bare
//! `FocusWindow` cannot show it; the handlers `startup::adopt` registers, and
//! the dialogs a person opens over the window, can. Nothing else notices:
//! Focus opens one window for the life of the process, so the leak costs
//! what a suite pays that builds and destroys a window per case.

use adw::prelude::*;

use crate::support::{self, Fixture};

pub fn a_window_startup_wired_and_used_still_frees_when_destroyed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;

        let (weak_window, weak_list) = {
            let (window, _client) = fixture.open().await;
            // Open what a person opens over the window: the message dialog
            // registers handlers on the window that owns it.
            support::keys(&window, &["j", "Return"]);
            assert!(
                crate::settle_until(async || window
                    .reading()
                    .is_some_and(|reading| reading.is_open()))
                .await,
                "the message never opened"
            );
            support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
            crate::settle();
            let weak_list = window.pane().expect("the list").view().downgrade();
            let weak_window = window.downgrade();
            window.destroy();
            (weak_window, weak_list)
        };
        crate::settle_for(std::time::Duration::from_millis(300)).await;

        assert!(
            weak_window.upgrade().is_none(),
            "a window that ran through `startup::adopt` outlived its own \
             destruction: something registered on a child widget holds a \
             strong reference back to it (the cycle #794 catalogued)"
        );
        assert!(
            weak_list.upgrade().is_none(),
            "the list outlived the window that built it"
        );
    });
}

/// Destroying is required, and that part is GTK's, not ours: a window joins
/// the toplevel list when it is constructed and leaves on destroy, so
/// dropping the handle is never enough on its own. If a dropped window is
/// released by itself one day, the teardown the suite does can be dropped.
pub fn dropping_a_window_without_destroying_it_is_not_enough() {
    if !support::display() {
        return;
    }
    let window = {
        let window = postio_focus::window::FocusWindow::new(None);
        window.downgrade()
    };
    crate::settle();
    assert!(
        window.upgrade().is_some(),
        "GTK's toplevel behaviour changed: a dropped window is released \
         without being destroyed"
    );
    if let Some(window) = window.upgrade() {
        window.destroy();
    }
    crate::settle();
}

/// The composer Focus mounts, with its web process, frees with the window.
/// Held out (see `IGNORED`): a mounted composer outlives its window.
pub fn a_destroyed_window_releases_its_composer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let weak_composer = {
            let (window, _client) = fixture.open().await;
            let composer = window.composer().expect("the composer is mounted");
            support::keys(&window, &["c"]);
            assert!(
                crate::settle_until(async || composer.is_open()).await,
                "the composer never opened"
            );
            let weak = composer.upcast_ref::<gtk::Widget>().downgrade();
            window.destroy();
            weak
        };
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        assert!(
            weak_composer.upgrade().is_none(),
            "the composer (and its web process) outlived the window that built it"
        );
    });
}
