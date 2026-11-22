//! The keyboard has one home, the list: focus is never on the top bar's Compose or place button,
//! where Space or Return would press a control.

use gtk::prelude::*;

use crate::support;

fn in_the_list(window: &postio_gtk::window::FocusWindow) -> bool {
    let view = window.pane().expect("the inbox").view().clone();
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focus| focus == view || focus.is_ancestor(&view))
}

fn said(window: &postio_gtk::window::FocusWindow) -> String {
    format!("the keyboard is on {}", support::focus_path(window))
}

pub fn the_window_opens_with_the_keyboard_in_the_list() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        crate::settle();
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn escape_from_the_bar_returns_the_keyboard_to_the_list() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        assert!(bar.is_open(), "the bar is up");
        assert!(!in_the_list(&window), "the keyboard is in the bar's entry");
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!bar.is_open(), "Escape closed the bar");
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn going_to_a_place_leaves_the_keyboard_in_the_list() {
    crate::gtk_case(async {
        let fixture = support::Fixture::empty().await;
        fixture.file_from_ada("Inbox mail", "Hello.", 5).await;
        let travel = fixture.folder("Travel").await;
        fixture.file_in(travel, "Train to the coast", 60).await;
        fixture.file_in(travel, "Hotel booking", 20).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
        assert!(places.is_open(), "the popover is up");
        assert!(
            crate::settle_until(async || places.names().contains(&"Travel".to_owned())).await,
            "the popover never listed Travel: {:?}",
            places.names()
        );
        places.set_filter("trav");
        places.activate();
        assert!(
            crate::settle_until(async || !places.is_open()).await,
            "choosing a place closed the popover"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window)
                == ["Hotel booking", "Train to the coast"]
                && in_the_list(&window))
            .await,
            "{:?}: {}",
            support::subjects(&window),
            said(&window)
        );

        window.act(postio_core::CommandId::GoToInbox);
        assert!(
            crate::settle_until(async || window.place_name() == "Inbox"
                && support::subjects(&window) == ["Inbox mail"]
                && in_the_list(&window))
            .await,
            "{}: {}",
            window.place_name(),
            said(&window)
        );
    });
}

fn on_the_empty_page(window: &postio_gtk::window::FocusWindow) -> bool {
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focus| focus.is_mapped() && focus.has_css_class("focus-empty"))
}

fn on_a_control(window: &postio_gtk::window::FocusWindow) -> bool {
    gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
        focus.is::<gtk::Button>()
            || focus.ancestor(gtk::Button::static_type()).is_some()
            || focus.is::<gtk::CheckButton>()
            || focus.is::<gtk::Switch>()
    })
}

pub fn going_to_an_empty_place_rests_the_keyboard_on_its_page() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        assert!(
            crate::settle_until(async || in_the_list(&window)).await,
            "{}",
            said(&window)
        );
        support::keys(&window, &["g", "b"]);
        assert!(
            crate::settle_until(
                async || window.place_name() == "Snoozed" && on_the_empty_page(&window)
            )
            .await,
            "{}: {}",
            window.place_name(),
            said(&window)
        );
        // The page came in on a stack switch, which hands the keyboard on as it goes; let every
        // pending turn run.
        crate::settle();
        assert!(
            on_the_empty_page(&window) && !on_a_control(&window),
            "{}",
            said(&window)
        );
        let footer_shown = || {
            support::with_class(&window, "focus-scope-footer")
                .first()
                .is_some_and(|footer| footer.is_mapped())
        };
        assert!(
            !footer_shown(),
            "the footer still names a row that is not here"
        );

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || window.place_name() == "Inbox"
                && support::subjects(&window).len() == 3
                && in_the_list(&window)
                && footer_shown())
            .await,
            "{}: {}",
            window.place_name(),
            said(&window)
        );
    });
}

pub fn an_empty_inbox_at_launch_rests_the_keyboard_on_its_page() {
    crate::gtk_case(async {
        let fixture = support::Fixture::empty().await;
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || on_the_empty_page(&window)).await,
            "{}",
            said(&window)
        );
        crate::settle();
        assert!(
            on_the_empty_page(&window) && !on_a_control(&window),
            "{}",
            said(&window)
        );
    });
}

pub fn the_keyboard_follows_the_cursor_row() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j"]);
        crate::settle();
        let pane = window.pane().expect("the inbox");
        assert_eq!(pane.cursor().selected(), 1);
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn a_list_that_opens_takes_the_keyboard_from_a_row_to_the_first() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        let pane = window.pane().expect("the inbox");
        support::keys(&window, &["j", "j"]);
        pane.focus_cursor();
        crate::settle();
        assert_eq!(pane.cursor().selected(), 2);

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "g i left the cursor on {}",
            pane.cursor().selected()
        );
        // The list view rebuilds its rows on the next layout and the row holding the keyboard goes
        // with the old ones, so for a turn or more
        // nothing has focus; wait for `scroll_to(FOCUS)`.
        let on_the_first_row = || {
            let first = pane.rows_on_screen().into_iter().next()?;
            let item = first.ancestor(gtk::glib::Type::from_name("GtkListItemWidget")?)?;
            let focus = gtk::prelude::GtkWindowExt::focus(&window)?;
            Some(focus == item || focus.is_ancestor(&item))
        };
        assert!(
            crate::settle_until(async || on_the_first_row() == Some(true)).await,
            "the keyboard stayed on a row the cursor left: {}",
            support::focus_path(&window)
        );
    });
}
