use gtk::gdk;
use gtk::prelude::*;

use crate::support::{self, Fixture};

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

fn listed(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    window
        .filtered()
        .map(|view| view.subjects())
        .unwrap_or_default()
}

pub fn g_f_lists_filtered_mail_and_its_number_keys_narrow_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, client) = fixture.open().await;
        assert!(
            crate::settle_until(async || {
                pixels_of(&one(window.upcast_ref(), "focus-has-action")).is_some()
                    && inbox_day_heading(&window).is_some_and(|heading| heading.height() > 0)
            })
            .await,
            "the inbox strip and its first day heading were never drawn"
        );
        let strip = strip_type(&window);
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
        // The tabs' counts are read apart from the rows and can land a pass after them.
        let _ = crate::settle_until(async || {
            let said = view.texts();
            said.iter().any(|line| line == "4") && said.iter().any(|line| line == "2")
        })
        .await;
        assert!(
            crate::settle_until(async || {
                support::with_class(view.widget(), "focus-filtered-tab")
                    .iter()
                    .all(|tab| pixels_of(tab).is_some())
            })
            .await,
            "Filtered's tabs were never drawn"
        );
        assert_header_in_strip_type(view.widget().upcast_ref::<gtk::Widget>(), &strip);
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

pub fn selecting_in_filtered_restores_every_selected_row_as_one_undo() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        let inbox_mark_read = mark_read_in(&support::with_class(&window, "focus-bulk-bar"))
            .expect("the inbox's bar has Mark read");
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "Filtered never listed: {:?}",
            listed(&window)
        );
        let view = window.filtered().expect("the view");
        let said = async |wanted: &str| {
            let wanted = wanted.to_owned();
            crate::settle_until(async || view.texts().contains(&wanted)).await
        };

        support::press(&window, "x", gdk::ModifierType::empty());
        assert!(said("1 selected").await, "x: {:?}", view.texts());
        let mark_read = mark_read_in(&support::with_class(view.widget(), "focus-bulk-bar"))
            .expect("the Filtered bar has Mark read");
        assert!(mark_read.is_mapped(), "the Filtered bar is not on screen");
        assert_eq!(
            dress(&mark_read),
            dress(&inbox_mark_read),
            "the bar's Mark read"
        );
        support::press(&window, "J", gdk::ModifierType::SHIFT_MASK);
        assert!(said("2 selected").await, "J: {:?}", view.texts());
        assert_eq!(view.selected().len(), 2);

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !view.has_selection()).await,
            "Escape did not clear the selection"
        );
        assert!(window.filtered().is_some(), "the first Escape only clears");

        support::press(&window, "x", gdk::ModifierType::empty());
        assert!(said("1 selected").await);
        support::press(&window, "2", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !view.has_selection()).await,
            "a tab kept the selection"
        );
        assert!(
            crate::settle_until(async || listed(&window) == ["You have been selected"]).await,
            "the Spam tab never listed: {:?}",
            listed(&window)
        );
        support::press(&window, "1", gdk::ModifierType::empty());
        assert!(crate::settle_until(async || listed(&window).len() == 4).await);

        support::press(&window, "x", gdk::ModifierType::empty());
        support::press(&window, "J", gdk::ModifierType::SHIFT_MASK);
        assert!(
            said("2 selected").await,
            "J after a tab: {:?} {:?}",
            view.selected(),
            view.texts()
        );
        support::press(&window, "R", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                listed(&window) == ["You have been selected", "Build 1182 passed"]
            })
            .await,
            "R did not restore both selected rows: {:?}",
            listed(&window)
        );
        assert!(
            !view.has_selection(),
            "what was acted on is no longer selected"
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "one Ctrl+Z did not put both back: {:?}",
            listed(&window)
        );
    });
}

struct StripType {
    height: i32,
    place: (i32, gtk::pango::Weight),
    counts: (i32, gtk::pango::Weight),
    toggle: gtk::Widget,
    toggle_words: (i32, gtk::pango::Weight),
    toggle_corner: bool,
    day_heading: i32,
}

fn one(root: &gtk::Widget, class: &str) -> gtk::Widget {
    support::with_class(root, class)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("nothing wears {class}"))
}

fn strip_type(window: &postio_gtk::window::FocusWindow) -> StripType {
    let strip = one(window.upcast_ref(), "focus-header-strip");
    StripType {
        height: strip.height(),
        place: font(&one(&strip, "focus-place-name")),
        counts: font(&label_in(&one(&strip, "focus-counts"))),
        toggle: one(&strip, "focus-has-action"),
        toggle_words: font(&one(&strip, "focus-has-action-label")),
        toggle_corner: corner_drawn(&one(&strip, "focus-has-action")),
        day_heading: inbox_day_heading(window)
            .expect("the inbox's day heading")
            .height(),
    }
}

/// The list must not move between the inbox and Filtered: the tabs sit in a row of the strip's
/// height, each the toggle's size and shape.
fn assert_header_in_strip_type(view: &gtk::Widget, strip: &StripType) {
    assert_eq!(
        font(&one(view, "focus-filtered-title")),
        strip.place,
        "the title"
    );
    let sweep = one(view, "focus-filtered-sweep");
    assert_eq!(
        font(&label_in(&sweep)),
        strip.toggle_words,
        "the sweep's words"
    );
    let back = one(view, "focus-filtered-back");
    assert_eq!(font(&label_in(&back)).0, strip.counts.0, "Back's words");
    assert_eq!(
        font(&one(view, "focus-filtered-note")).0,
        strip.counts.0,
        "the note"
    );
    let tabs = support::with_class(view, "focus-filtered-tab");
    assert_eq!(tabs.len(), 7, "the tabs");
    for tab in &tabs {
        for label in support::descendants(tab)
            .iter()
            .filter(|widget| widget.is::<gtk::Label>())
        {
            assert_eq!(font(label).0, strip.counts.0, "a tab's words");
        }
    }
    let on = one(view, "focus-filtered-tab-on");
    assert_eq!(
        font(&label_in(&on)).1,
        gtk::pango::Weight::Bold,
        "the tab on screen"
    );
    let off = tabs
        .iter()
        .find(|tab| !tab.has_css_class("focus-filtered-tab-on"));
    assert_ne!(
        outline(&on),
        outline(off.expect("a tab not on screen")),
        "the tab on screen has no outline of its own"
    );
    assert_eq!(
        one(view, "focus-filtered-tab-row").height(),
        strip.height,
        "the tab row is not the strip's height"
    );
    let heading = one(view, "focus-filtered-day")
        .parent()
        .expect("the heading's row");
    assert_eq!(
        heading.height(),
        strip.day_heading,
        "Filtered's day heading is not the inbox's height"
    );
    for tab in &tabs {
        assert_eq!(
            tab.height(),
            strip.toggle.height(),
            "a tab is not the has-action toggle's height"
        );
    }
    assert!(
        strip.toggle_corner,
        "the has-action toggle's left edge is not drawn where the probe looks"
    );
    assert!(
        corner_drawn(&on),
        "the tab on screen is a pill, not the has-action toggle's chip"
    );
    for control in [&sweep, &back].into_iter().chain(&tabs) {
        assert_eq!(
            kind(control),
            kind(&strip.toggle),
            "{:?} is not the strip's kind of button",
            control.css_classes()
        );
    }
}

fn label_in(root: &gtk::Widget) -> gtk::Widget {
    support::descendants(root)
        .into_iter()
        .find(|widget| widget.is::<gtk::Label>())
        .expect("a label")
}

fn font(label: &gtk::Widget) -> (i32, gtk::pango::Weight) {
    let font = label.pango_context().font_description().expect("a font");
    (font.size(), font.weight())
}

fn inbox_day_heading(window: &postio_gtk::window::FocusWindow) -> Option<gtk::Widget> {
    support::descendants(window.upcast_ref::<gtk::Widget>())
        .into_iter()
        .find(|widget| widget.css_name() == "focusdayheading")
}

fn outline(widget: &gtk::Widget) -> String {
    let (pixels, width, _) = pixels_of(widget).expect("drawn");
    let at = usize::try_from(width / 2 * 4).expect("a column");
    format!("{:?}", &pixels[at..at + 4])
}

fn corner_drawn(widget: &gtk::Widget) -> bool {
    let (pixels, width, height) = pixels_of(widget).expect("drawn");
    let row = usize::try_from(height / 4).expect("a row");
    let stride = usize::try_from(width * 4).expect("a width");
    pixels[row * stride + 3] > 0
}

fn pixels_of(widget: &gtk::Widget) -> Option<(Vec<u8>, i32, i32)> {
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(widget.width()),
        f64::from(widget.height()),
    );
    let node = snapshot.to_node()?;
    let renderer = widget
        .native()
        .expect("a native")
        .renderer()
        .expect("a renderer");
    let texture = renderer.render_texture(&node, None);
    let (texture_width, texture_height) = (texture.width(), texture.height());
    let stride = usize::try_from(texture_width * 4).expect("a width");
    let mut pixels = vec![0u8; stride * usize::try_from(texture_height).expect("a height")];
    texture.download(&mut pixels, stride);
    Some((pixels, texture_width, texture_height))
}

fn kind(button: &gtk::Widget) -> Vec<String> {
    let mut classes: Vec<String> = button
        .css_classes()
        .iter()
        .map(|class| class.to_string())
        .filter(|class| class.starts_with("postio-button"))
        .collect();
    classes.sort();
    classes
}

fn mark_read_in(bars: &[gtk::Widget]) -> Option<gtk::Widget> {
    bars.iter().flat_map(support::descendants).find(|widget| {
        widget.is::<gtk::Button>()
            && support::descendants(widget).iter().any(|inner| {
                inner
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == "Mark read")
            })
    })
}

fn dress(button: &gtk::Widget) -> Vec<(String, Vec<String>)> {
    support::descendants(button)
        .iter()
        .map(|widget| {
            let mut classes: Vec<String> = widget
                .css_classes()
                .iter()
                .map(|class| class.to_string())
                .collect();
            classes.sort();
            (widget.type_().name().to_owned(), classes)
        })
        .collect()
}

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

        support::keys(&window, &["g", "l"]);
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
