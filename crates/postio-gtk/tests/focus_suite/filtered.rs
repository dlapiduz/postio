use gtk::gdk;
use gtk::prelude::*;

use crate::support::{self, Fixture};

async fn fixture() -> Fixture {
    let fixture = Fixture::empty().await;
    fixture.file_from_ada("Atlas budget", "Numbers.", 5).await;
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
