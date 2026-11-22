use adw::prelude::*;
use postio_core::CommandId;

use crate::support::{self, Fixture};

fn detach_key(window: &postio_gtk::window::FocusWindow) {
    support::press(
        window,
        "o",
        gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
    );
}

pub fn the_detach_command_moves_the_open_composer_to_a_window_and_back() {
    crate::gtk_case(async {
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
        assert_eq!(
            window.observe().keyboard.region,
            postio_ui::observe::Region::Composer,
            "a detached composition left the keyboard on the list"
        );
        composer.dispatch(CommandId::DetachComposer);
        assert!(
            crate::settle_until(async || !composer.is_detached()).await,
            "the command did not bring it home"
        );
    });
}

pub fn a_detached_composer_window_holds_the_keyboard() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        let composer = window.composer().expect("the composer is mounted");
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "`c` did not open the composer"
        );
        composer.dispatch(CommandId::DetachComposer);
        assert!(
            crate::settle_until(async || composer.is_detached()).await,
            "Detach left the composer in its dialog"
        );
        let host = composer.detached_window().expect("the detached window");
        assert!(
            crate::settle_until(async || {
                gtk::prelude::GtkWindowExt::focus(&host)
                    .is_some_and(|focus| focus.is_ancestor(&composer))
            })
            .await,
            "the detached window's focus is {:?}, not inside the composition",
            gtk::prelude::GtkWindowExt::focus(&host)
        );
        let seen = window.observe();
        assert!(
            seen.keyboard.widget.contains("PostioComposer"),
            "the observation says the keyboard is on {:?}",
            seen.keyboard.widget
        );
        assert_eq!(
            seen.app.get("focus.composer.holds_keyboard"),
            Some(&serde_json::json!(true)),
            "the observation does not report the detached window holding the keyboard"
        );
    });
}

pub fn the_window_warms_its_editing_surface_without_being_asked_to_compose() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "Numbers.", 5).await;
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

pub fn a_warm_ask_made_before_the_composer_is_mounted_still_warms_it() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "Numbers.", 5).await;
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        window.warm_composer();
        let session =
            postio_gtk::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
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

pub fn a_key_typed_in_a_detached_composition_lands_in_it() {
    use postio_widgets::storyboard::deliver;

    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        let composer = window.composer().expect("the composer is mounted");
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "`c` did not open the composer"
        );
        detach_key(&window);
        assert!(
            crate::settle_until(async || composer.detached_window().is_some()).await,
            "the composer never left its dialog"
        );
        let host: gtk::Window = composer.detached_window().expect("its own window").upcast();
        assert!(
            crate::settle_until(
                async || composer.focused_field() == Some(postio_widgets::composer::Field::To)
            )
            .await,
            "detaching did not leave the keyboard in To"
        );

        let seen = window.observe();
        assert_eq!(seen.keyboard.field.as_deref(), Some("to"));
        assert!(
            seen.keyboard.typing,
            "the keyboard is in a text field of the detached window and the \
             observation says it is not typing"
        );

        assert!(
            matches!(
                deliver::deliver(&host, gtk::gdk::Key::j, gtk::gdk::ModifierType::empty()),
                deliver::Delivery::Dropped
            ),
            "a letter typed in the detached composer was taken as a command"
        );
        assert_eq!(
            deliver::type_text(&host, "ada@example.com", None),
            deliver::TypeOutcome::Typed
        );
        assert!(
            composer
                .draft()
                .to
                .iter()
                .any(|address| address.to_string().contains("ada@example.com")),
            "what was typed in the detached window did not reach the composition: {:?}",
            composer.draft().to
        );
    });
}
