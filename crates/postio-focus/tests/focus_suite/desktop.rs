//! What the desktop knows Focus by (T159): the name its windows give the
//! compositor, and the `postio://` links it opens.

/// GNOME matches a window to its desktop entry by the Wayland `app_id`,
/// which GDK takes from `g_get_prgname()` -- the *binary* name,
/// `postio-focus`, unless the application says otherwise. Focus's entry is
/// `dev.postio.Postio.Focus.desktop`, with `StartupWMClass` to match, so a
/// Focus that never set its name would be drawn with a generic icon under a
/// generic name. The classic app's `desktop_entry.rs` holds the same line.
pub fn focus_says_which_application_it_is() {
    let _application = postio_focus::app::application();
    let reported = gtk::glib::prgname();
    assert_eq!(
        reported.as_ref().map(|name| name.as_str()),
        Some(postio_focus::app::APP_ID),
        "Focus's windows would tell the compositor they are {reported:?}"
    );
    // The entry registers `x-scheme-handler/postio` with `%U`: a link
    // clicked elsewhere arrives as a file to open, which GApplication drops
    // unless the application says it takes them.
    assert!(
        gtk::prelude::ApplicationExt::flags(&_application)
            .contains(gtk::gio::ApplicationFlags::HANDLES_OPEN),
        "Focus would drop every postio:// link it was handed"
    );
}

/// US15 scenario 2: a `postio://message/<id>` link opens that message over
/// the list and does nothing else to it; a link naming nothing here, or
/// nothing Postio knows how to name, is refused with a sentence.
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
            .row()
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
