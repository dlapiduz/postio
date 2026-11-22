//! A handler stored on a child widget holding a strong reference back to its window is a cycle
//! (#794) that a bare `FocusWindow` cannot show, but the handlers `startup::adopt` registers and
//! the dialogs a person opens can (#1072).

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

/// Destroying is GTK's requirement: a window joins the toplevel list when constructed and leaves on
/// destroy, so dropping the handle is never enough.
pub fn dropping_a_window_without_destroying_it_is_not_enough() {
    if !support::display() {
        return;
    }
    let window = {
        let window = postio_gtk::window::FocusWindow::new(None);
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
