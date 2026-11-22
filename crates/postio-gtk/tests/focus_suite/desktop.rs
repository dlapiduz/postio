/// GNOME matches a window to its desktop entry by the Wayland `app_id`, which GDK takes from
/// `g_get_prgname()`: the binary's name unless the app says otherwise.
pub fn focus_says_which_application_it_is() {
    let _application = postio_gtk::app::application();
    let reported = gtk::glib::prgname();
    assert_eq!(
        reported.as_ref().map(|name| name.as_str()),
        Some(postio_gtk::app::APP_ID),
        "Focus's windows would tell the compositor they are {reported:?}"
    );
    // A link clicked elsewhere arrives as a file to open, which GApplication drops unless the
    // application says it takes them.
    assert!(
        gtk::prelude::ApplicationExt::flags(&_application)
            .contains(gtk::gio::ApplicationFlags::HANDLES_OPEN),
        "Postio would drop every mailto: and postio:// link it was handed"
    );
}

pub fn the_window_is_titled_postio() {
    use gtk::prelude::GtkWindowExt;
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let window = postio_gtk::window::FocusWindow::new(None);
        assert_eq!(window.title().as_deref(), Some("Postio"));
    });
}

pub fn focus_shows_the_postio_icon() {
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let (_fixture, window) = crate::support::three_in_the_inbox().await;
        let name = postio_gtk::app::ICON_NAME;
        assert_eq!(name, "dev.postio.Postio", "the package's one icon");
        assert_eq!(
            gtk::Window::default_icon_name().as_deref(),
            Some(name),
            "windows without an icon of their own would be drawn with none"
        );
        let display = gtk::prelude::WidgetExt::display(&window);
        let theme = gtk::IconTheme::for_display(&display);
        assert!(theme.has_icon(name), "the icon theme cannot find {name}");
        let paintable = theme.lookup_icon(
            name,
            &[],
            64,
            1,
            gtk::TextDirection::Ltr,
            gtk::IconLookupFlags::empty(),
        );
        assert!(
            paintable.file().is_some(),
            "the lookup found nothing to draw for {name}"
        );
    });
}

pub fn a_postio_link_opens_its_message_and_an_unknown_one_is_refused() {
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let (fixture, window) = crate::support::three_in_the_inbox().await;
        let row = window
            .pane()
            .expect("the list")
            .rows_on_screen()
            .into_iter()
            .find_map(|widget| widget.item())
            .expect("a row");
        let subject = row
            .as_conversation()
            .expect("a conversation")
            .summary
            .representative
            .subject
            .clone()
            .unwrap_or_default();

        window.open_link("postio://message/987654");
        assert!(
            crate::settle_until(async || {
                window.toast_showing().as_deref() == Some(postio_ui::links::GONE)
            })
            .await,
            "a link to nothing here was not refused: {:?}",
            window.toast_showing()
        );
        window.open_link("postio://thread/1");
        assert!(
            crate::settle_until(async || {
                window.toast_showing().as_deref() == Some(postio_ui::links::UNKNOWN)
            })
            .await,
            "a link Postio cannot read was not refused: {:?}",
            window.toast_showing()
        );
        assert!(
            window.reading().is_none_or(|reading| !reading.is_open()),
            "a refused link opens nothing"
        );

        window.open_link(&postio_ui::links::message_uri(row.id()));
        assert!(
            crate::settle_until(async || {
                window
                    .reading()
                    .is_some_and(|reading| reading.is_open() && reading.title() == subject)
            })
            .await,
            "the link did not open {subject:?}"
        );
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            crate::support::subjects(&window).len(),
            3,
            "opening a link acted on nothing"
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let stored = postio_storage::repository::MessageRepository::new(&connection)
            .get(row.id())
            .await
            .expect("a read")
            .expect("the message");
        assert!(
            !stored.flags.contains(&postio_model::Flag::Seen),
            "going to a message does not mark it read"
        );
    });
}

async fn shows_the_mailto(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || window.compose_dialog().is_some()).await,
        "a mailto: link opened no composer"
    );
    let dialog = window.compose_dialog().expect("the compose dialog");
    assert!(
        crate::settle_until(async || {
            crate::compose::field(&dialog, "To").as_deref() == Some("ada@example.com")
        })
        .await,
        "the link's recipient is not in To: {:?}",
        crate::compose::field(&dialog, "To")
    );
    assert_eq!(
        crate::compose::field(&dialog, "Subject").as_deref(),
        Some("Lunch on Friday"),
        "the link's subject"
    );
    let composer = window.composer().expect("the composer");
    let body = || composer.test_body_eval("document.body.innerText");
    assert!(
        crate::settle_until(async || body().contains("Noon at the usual place?")).await,
        "the link's body is not in the editor: {:?}",
        body()
    );
}

pub fn a_mailto_link_opens_a_composer_with_its_fields_filled() {
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let fixture = crate::support::Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        window.open_link(
            "mailto:ada@example.com?subject=Lunch%20on%20Friday&body=Noon%20at%20the%20usual%20place%3F",
        );
        shows_the_mailto(&window).await;
    });
}

pub fn a_mailto_link_that_arrives_before_the_store_waits_for_it() {
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let fixture = crate::support::Fixture::empty().await;
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        window.open_link(
            "mailto:ada@example.com?subject=Lunch%20on%20Friday&body=Noon%20at%20the%20usual%20place%3F",
        );
        crate::support::keep(postio_gtk::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        ));
        shows_the_mailto(&window).await;
    });
}

/// `run` owns the process's main loop and cannot be driven from a suite, so this holds the parts it
/// stands on: one window, and a second `start_syncing` is a no-op.
pub fn a_second_activate_has_one_window_and_starts_sync_once() {
    use adw::prelude::*;
    crate::gtk_case(async {
        if !crate::support::display() {
            return;
        }
        let application = postio_gtk::app::application();
        let _ = application.register(gtk::gio::Cancellable::NONE);
        let none = crate::support::NoAccount::new().await;
        let host = none.host_signing_in_to(postio_account::backend::MockBackend::default());
        let window = postio_gtk::window::FocusWindow::new(Some(&application));
        window.present();
        let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
        crate::settle();

        let active = application.active_window();
        assert_eq!(
            active.as_ref().map(|active| active.as_ptr()),
            Some(window.upcast_ref::<gtk::Window>().as_ptr()),
            "the application does not know Focus's window as its active one, so a \
             second launch would build another"
        );
        window.present();
        crate::settle();
        let focus_windows = || {
            let toplevels = gtk::Window::toplevels();
            (0..toplevels.n_items())
                .filter_map(|item| toplevels.item(item))
                .filter(|object| object.is::<postio_gtk::window::FocusWindow>())
                .count()
        };
        assert_eq!(focus_windows(), 1, "a second activate left two windows");

        assert!(!session.syncing(), "nothing has started sync yet");
        session.start_syncing();
        assert!(session.syncing(), "the first start brings sync up");
        session.start_syncing();
        assert!(session.syncing(), "a second start leaves it up");
        crate::support::keep(session);
    });
}
