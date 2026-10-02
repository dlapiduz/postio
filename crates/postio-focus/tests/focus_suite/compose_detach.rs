//! The composer in the running Focus: a person can detach it to a window of
//! its own and bring it home with the keys, and its editing surface is
//! started before anybody asks to write.
//!
//! The widget is proved by `postio-widgets`' own tests. That is not the same
//! claim as this one, and the difference is `postio-bl2`: a capability built,
//! tested, documented and wired to nothing, with every test green. So these
//! start where Focus starts -- a store, a window, `startup::adopt` -- and
//! assert from the far end: the keys a person presses, a second window with
//! the composition in it, and a web process nobody asked for yet.

use adw::prelude::*;
use postio_core::CommandId;

use crate::support::{self, Fixture};

fn detach_key(window: &postio_focus::window::FocusWindow) {
    support::press(
        window,
        "o",
        gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
    );
}

pub fn the_detach_key_reaches_the_composer_in_a_wired_focus() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let composer = window.composer().expect("the composer is mounted");
        assert!(!composer.is_open(), "nothing is being composed yet");

        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "`c` did not open the composer, so this case says nothing about \
             detaching"
        );
        assert!(!composer.is_detached(), "in the dialog is the default");

        detach_key(&window);
        assert!(
            crate::settle_until(async || composer.detached_window().is_some()).await,
            "the key resolved to nothing a person could see: every layer under \
             this one passes, and that is the shape of bug postio-bl2 is about"
        );
        let host = composer.detached_window().expect("the detached window");
        assert!(composer.is_detached());
        assert!(host.is_visible(), "a window nobody can see is not a window");
        assert!(!host.is_modal(), "the main window must stay usable");
        assert!(
            window
                .compose_dialog()
                .is_none_or(|dialog| !dialog.is_mapped()),
            "the composition is in two places: the dialog is still over the list"
        );

        // The key in the detached window puts it back, through the same
        // registry: one command, two containers.
        composer.handle_key(
            gtk::gdk::Key::from_name("o").expect("a key"),
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(
            crate::settle_until(async || !composer.is_detached()).await,
            "the key in the detached window did not bring the composer home"
        );
        assert!(composer.is_open(), "still one composition, still open");
        assert!(
            crate::settle_until(async || window
                .compose_dialog()
                .is_some_and(|dialog| dialog.is_mapped()))
            .await,
            "the composition did not come home to its dialog"
        );
    });
}

/// The header's Detach button and the command both move the composition: to
/// a window of its own, and home again.
pub fn the_detach_command_moves_the_open_composer_to_a_window_and_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let composer = window.composer().expect("the composer is mounted");
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "`c` did not open the composer"
        );
        let detach = support::only(&window, "focus-compose-detach");
        detach
            .downcast_ref::<gtk::Button>()
            .expect("Detach is a button")
            .emit_clicked();
        assert!(
            crate::settle_until(async || composer.is_detached()).await,
            "pressing Detach ({:?}) left the composer in its dialog",
            CommandId::DetachComposer
        );
        composer.dispatch(CommandId::DetachComposer);
        assert!(
            crate::settle_until(async || !composer.is_detached()).await,
            "the command did not bring it home"
        );
    });
}

/// #1216: the editing surface's web process is started before anybody
/// composes -- `app::run` asks for it on an idle turn after the first frame
/// -- and warming it opens nothing. It does not matter whether the ask
/// comes before the composer is mounted (the window remembers it) or after.
pub fn the_window_warms_its_editing_surface_without_being_asked_to_compose() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        let composer = window.composer().expect("the composer is mounted");
        assert!(
            !composer.is_warm(),
            "the composer was warm before anything asked, so this case could \
             not fail"
        );

        window.warm_composer();
        assert!(
            crate::settle_until(async || composer.is_warm()).await,
            "nothing warmed the editing surface: `Composer::warm` exists and its \
             own test passes, which is the shape of bug #327 -- check what is \
             supposed to *call* it"
        );
        assert!(
            !composer.is_open(),
            "warming the editing surface opened the composer"
        );
        assert!(
            window
                .compose_dialog()
                .is_none_or(|dialog| !dialog.is_mapped()),
            "warming the editing surface put the dialog over the list"
        );
    });
}

/// The ask made before the composer is mounted -- the first frame can come
/// before the inbox does -- still warms it once it is.
pub fn a_warm_ask_made_before_the_composer_is_mounted_still_warms_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        window.warm_composer();
        let session = postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        );
        support::keep(session);
        assert!(
            crate::settle_until(async || window
                .composer()
                .is_some_and(|composer| composer.is_warm()))
            .await,
            "a warm ask that came before the composer was mounted was forgotten"
        );
    });
}
