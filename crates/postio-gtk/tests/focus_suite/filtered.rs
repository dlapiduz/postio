//! The Filtered view (US9, T124; screen 21): `g f` lists what filing
//! archived, newest first, each with its reason; `1`-`7` narrow it to a
//! reason; `R` restores the focused row, and `Ctrl+Z` takes that back.

use gtk::gdk;

use crate::support::{self, Fixture};

/// Four filtered messages and one conversation in the inbox.
async fn fixture() -> Fixture {
    let fixture = Fixture::empty().await;
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Atlas budget",
            "Numbers.",
            5,
        )
        .await;
    for (from, subject, reason, source, minutes) in [
        (
            ("Forge", "noreply@forge.test"),
            "Review requested",
            "notification",
            Some("Forge"),
            10,
        ),
        (
            ("Outdoor Supply", "deals@outdoor.test"),
            "30% off tents",
            "promotion",
            None,
            20,
        ),
        (
            ("Lucky Rewards", "win@lucky.test"),
            "You have been selected",
            "spam",
            None,
            30,
        ),
        // Forty days ago: Filtered never deletes (scenario 7).
        (
            ("Forge", "noreply@forge.test"),
            "Build 1182 passed",
            "notification",
            Some("Forge"),
            40 * 24 * 60,
        ),
    ] {
        fixture
            .filtered(from, subject, reason, source, minutes)
            .await;
    }
    fixture
}

/// The Filtered view's row subjects, top to bottom.
fn listed(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    window
        .filtered()
        .map(|view| view.subjects())
        .unwrap_or_default()
}

/// US9 scenarios 5 and 7: every filtered message, newest first, the old
/// one included; the Notifications tab's number lists only notifications,
/// one request to the store.
pub fn g_f_lists_filtered_mail_and_its_number_keys_narrow_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, client) = fixture.open().await;
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || {
                listed(&window)
                    == [
                        "Review requested",
                        "30% off tents",
                        "You have been selected",
                        "Build 1182 passed",
                    ]
            })
            .await,
            "g f did not list the filtered mail, newest first: {:?}",
            listed(&window)
        );
        let view = window.filtered().expect("the view");
        let said = view.texts();
        for wanted in [
            "Filtered",
            "All",
            "4",
            "Notifications",
            "2",
            "notification \u{b7} Forge",
        ] {
            assert!(
                said.iter().any(|line| line == wanted),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            support::keycaps_are_taught(view.widget()),
            "the Filtered view's keycaps do not teach their controls (T142)"
        );

        let before = client.counts().snapshot();
        support::press(&window, "4", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                listed(&window) == ["Review requested", "Build 1182 passed"]
            })
            .await,
            "the Notifications tab did not narrow the list: {:?}",
            listed(&window)
        );
        let after = client.counts().snapshot();
        let asked: Vec<(&str, u64)> = after
            .iter()
            .map(|(family, count)| (*family, count - before.get(family).copied().unwrap_or(0)))
            .filter(|(_, count)| *count > 0)
            .collect();
        assert_eq!(asked, [("Filtered", 1)], "a tab is one page read");

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.filtered().is_none()).await,
            "Escape went back to the inbox"
        );

        // Screen 21's footer: j walks the rows, g i goes to the inbox.
        support::keys(&window, &["g", "f"]);
        let view = window.filtered().expect("Filtered again");
        assert!(crate::settle_until(async || listed(&window).len() == 4).await);
        let first = view.focused();
        support::press(&window, "j", gdk::ModifierType::empty());
        assert!(
            view.focused().is_some() && view.focused() != first,
            "j did not move to the next row"
        );
        let cursor_before = window.cursor_row().map(|row| row.id());
        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || window.filtered().is_none()).await,
            "g i did not go to the inbox"
        );
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            cursor_before,
            "j in Filtered did not move the inbox's cursor"
        );
    });
}

/// US9 scenario 4, in the view: `R` restores the focused row -- it leaves
/// Filtered for the inbox -- and one `Ctrl+Z` takes it back.
pub fn r_restores_the_focused_row_and_ctrl_z_takes_it_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "Filtered never listed: {:?}",
            listed(&window)
        );
        let view = window.filtered().expect("the view");
        assert!(
            view.texts()
                .iter()
                .any(|line| line == "Restore, never filter this sender"),
            "the focused row offers its restore: {:?}",
            view.texts()
        );
        support::press(&window, "R", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                listed(&window)
                    == [
                        "30% off tents",
                        "You have been selected",
                        "Build 1182 passed",
                    ]
            })
            .await,
            "R did not take the row out of Filtered: {:?}",
            listed(&window)
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "Ctrl+Z did not put it back: {:?}",
            listed(&window)
        );
    });
}

/// T126: the header strip says how many were filtered since local
/// midnight, with `g f`, and pressing it opens Filtered; the folders
/// popover lists Filtered with the same count.
pub fn the_strip_counts_what_was_filtered_today() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        // Filed a minute and two ago, by the real clock: today. And one
        // forty days ago, which is not.
        let ago = |minutes: i64| (support::now() - chrono::Utc::now()).num_minutes() + minutes;
        fixture
            .filtered(
                ("Forge", "noreply@forge.test"),
                "Review requested",
                "notification",
                Some("Forge"),
                ago(1),
            )
            .await;
        fixture
            .filtered(
                ("Outdoor Supply", "deals@outdoor.test"),
                "30% off tents",
                "promotion",
                None,
                ago(2),
            )
            .await;
        fixture
            .filtered(
                ("Forge", "noreply@forge.test"),
                "Old build",
                "notification",
                Some("Forge"),
                40 * 24 * 60,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let chrome = window.chrome().expect("the strip");
        assert!(
            crate::settle_until(async || {
                chrome.filtered_today_said().as_deref() == Some("2 filtered today")
            })
            .await,
            "the strip does not count today's: {:?}",
            chrome.filtered_today_said()
        );
        let said = support::texts(chrome.strip());
        assert!(
            said.iter().any(|text| text == "g f"),
            "with its key: {said:?}"
        );

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Filtered".to_owned())).await,
            "the popover lists no Filtered: {:?}",
            places.names()
        );
        places.set_filter("filt");
        support::click_row_saying(&window, &window, "Filtered");
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "the popover's Filtered row did not open Filtered"
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        support::click(
            &window,
            support::with_class(chrome.strip(), "focus-filtered-today-button")
                .iter()
                .find(|widget| {
                    gtk::prelude::WidgetExt::is_mapped(*widget)
                        && gtk::prelude::ObjectExt::is::<gtk::Button>(*widget)
                })
                .expect("the strip's filtered-today button"),
            1,
        );
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "pressing the strip's count did not open Filtered"
        );
    });
}

/// FR-153 (T129): Focus notifies about new mail in its inbox as the
/// classic app would, and never about mail it filtered or held -- even
/// when an arrival names them.
pub fn focus_never_notifies_for_mail_it_filtered_or_held() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (kept, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let (held, _) = fixture
            .file(
                ("Ledger", "news@ledger.test"),
                "The weekly numbers",
                "Rates.",
                6,
            )
            .await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::DigestRepository::new(&connection)
                .hold(held, "Newsletters", support::now())
                .await
                .expect("held");
        }
        let filtered = fixture
            .filtered(
                ("Outdoor Supply", "deals@outdoor.test"),
                "30% off tents",
                "promotion",
                None,
                7,
            )
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let told: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::default();
        window.set_notification_sink({
            let told = std::rc::Rc::clone(&told);
            move |notification| told.borrow_mut().push(notification.body.clone())
        });
        // Looking at Filtered, not the inbox: an arrival there is news.
        support::keys(&window, &["g", "f"]);

        assert!(sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![held, filtered],
        }));
        crate::settle_for(std::time::Duration::from_millis(500)).await;
        assert!(
            told.borrow().is_empty(),
            "notified about {:?}",
            told.borrow()
        );

        assert!(sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![kept],
        }));
        assert!(
            crate::settle_until(async || !told.borrow().is_empty()).await,
            "no notification for mail that stayed in the inbox"
        );
        assert!(
            told.borrow()[0].contains("Atlas budget"),
            "it names what stayed: {:?}",
            told.borrow()
        );
    });
}

/// FR-118 (T128's surface): `F` says first how much of the inbox the
/// filtering rules would file away, and moves it only when the person
/// says so; one `Ctrl+Z` takes the sweep back. The main menu offers the
/// same.
pub fn f_says_what_a_sweep_would_move_then_moves_it_as_one_undo() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        fixture
            .file(
                (" ", "notifications@forge.example"),
                "Build passed",
                "Green.",
                10,
            )
            .await;
        fixture
            .file(
                (" ", "alerts@builds.example"),
                "Deploy finished",
                "Done.",
                15,
            )
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "F", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || window.sweep_confirmation().is_some()).await,
            "F asked nothing"
        );
        let dialog = window.sweep_confirmation().expect("the confirmation");
        let said = format!(
            "{} {}",
            gtk::prelude::ObjectExt::property::<String>(&dialog, "heading"),
            gtk::prelude::ObjectExt::property::<String>(&dialog, "body")
        );
        assert!(said.contains('2'), "it says how many would move: {said}");
        crate::settle_for(std::time::Duration::from_millis(200)).await;
        assert_eq!(support::subjects(&window).len(), 3, "nothing moved yet");

        // What a click on the button does: the response, and the dialog
        // closes.
        let sweep = support::button_labelled(&dialog, &postio_ui::filtered::sweep_action(2));
        support::click(&window, &sweep, 1);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Atlas budget"]).await,
            "the sweep did not move the two: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || window.sweep_confirmation().is_none()).await,
            "the confirmation stayed up"
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "one Ctrl+Z did not take the sweep back: {:?}",
            support::subjects(&window)
        );
    });
}
