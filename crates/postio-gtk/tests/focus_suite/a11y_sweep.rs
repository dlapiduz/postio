//! Every surface Focus draws, as a screen reader meets it: the generic sweep
//! of the classic app's `gtk_accessibility.rs`, over Focus's own surfaces.
//!
//! GTK has an audit as a test API (`gtk_test_accessible_has_property` and
//! friends), so the tree can be walked without a screen reader in the loop.
//! The sweep opens each surface by the key a person presses, proves it is
//! up by the application's own notion of open (an audit of a surface that
//! never opened walks the same tree again and passes for free), walks the
//! window for any control a screen reader cannot name, closes it, and proves
//! it closed. The harness runs under `GTK_A11Y=test`; `a11y.rs` proves the
//! backend is recording before any conclusion is drawn.

use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRelation, AccessibleRole};
use postio_gtk::window::FocusWindow;

use crate::support;

/// Roles a screen reader announces by name. A control it cannot name is a
/// control it cannot offer.
const NEEDS_A_NAME: &[AccessibleRole] = &[
    AccessibleRole::Button,
    AccessibleRole::Checkbox,
    AccessibleRole::ComboBox,
    AccessibleRole::Link,
    AccessibleRole::ListItem,
    AccessibleRole::MenuItem,
    AccessibleRole::Row,
    AccessibleRole::SearchBox,
    AccessibleRole::Switch,
    AccessibleRole::Tab,
    AccessibleRole::TextBox,
    AccessibleRole::ToggleButton,
];

/// Roles that say "I am a widget" and nothing a screen reader can use.
const SAYS_NOTHING: &[AccessibleRole] = &[AccessibleRole::Generic, AccessibleRole::Widget];

fn any_descendant_named(widget: &gtk::Widget) -> bool {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if named(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

/// Whether any label inside `widget` carries text.
fn has_text(widget: &gtk::Widget) -> bool {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        return !label.text().is_empty();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if has_text(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

/// Whether a screen reader would have something to call this widget: a set
/// `Label`, a `LabelledBy` relation, or a label of its own inside. GTK's test
/// API says whether a property was ever set and has no safe way to read its
/// value back (the classic audit did, through `unsafe`, which this crate
/// forbids), so a label set to the empty string reads as named here; the
/// rows' spoken sentences are asserted by value in `a11y`.
fn named(widget: &gtk::Widget) -> bool {
    if gtk::test_accessible_has_property(widget, AccessibleProperty::Label) {
        return true;
    }
    if gtk::test_accessible_has_relation(widget, AccessibleRelation::LabelledBy) {
        return true;
    }
    // A list item is named from its content: the row inside it is the widget
    // that carries the sentence, and is audited itself.
    if widget.accessible_role() == AccessibleRole::ListItem && any_descendant_named(widget) {
        return true;
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().is_some_and(|label| !label.is_empty())
    {
        return true;
    }
    has_text(widget)
}

/// libadwaita's toast grows a dismiss button of its own, icon-only and with
/// a tooltip, which is not a name. Postio's own button on that toast is named.
fn upstream_gap(widget: &gtk::Widget) -> bool {
    widget
        .parent()
        .is_some_and(|parent| parent.type_().name() == "AdwToastWidget")
        && widget
            .downcast_ref::<gtk::Button>()
            .and_then(|button| button.icon_name())
            .is_some_and(|icon| icon == "window-close-symbolic")
}

/// Walk the tree and collect what a screen reader could not use. `inside` is
/// whether some ancestor is already a named control: a `GtkMenuButton` wraps
/// an unnamed toggle of its own, which is an implementation detail.
fn audit(widget: &gtk::Widget, path: &str, inside: bool, problems: &mut Vec<String>) {
    let role = widget.accessible_role();
    let here = format!("{path} > {}", widget.type_().name());
    let named = named(widget);
    let classes = widget.css_classes().join(".");

    if widget.is_mapped() && !inside && !upstream_gap(widget) {
        if NEEDS_A_NAME.contains(&role) && !named {
            problems.push(format!("{here}[{classes}]: a {role:?} with no name"));
        }
        if widget.is_focusable() && SAYS_NOTHING.contains(&role) && !named {
            problems.push(format!(
                "{here}[{classes}]: focusable, and announces nothing"
            ));
        }
    }

    let inside = inside || (named && NEEDS_A_NAME.contains(&role));
    let mut child = widget.first_child();
    while let Some(current) = child {
        audit(&current, &here, inside, problems);
        child = current.next_sibling();
    }
}

/// Audit everything currently on screen, and say which surface it was.
fn expect_usable(window: &FocusWindow, surface: &str) {
    let mut problems = Vec::new();
    audit(
        window.upcast_ref::<gtk::Widget>(),
        "window",
        false,
        &mut problems,
    );
    assert!(
        problems.is_empty(),
        "{surface}: {} widget(s) a screen reader cannot use:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

/// A surface that is not on screen until something opens it.
struct Surface {
    name: &'static str,
    /// What a person presses to open it.
    open: fn(&FocusWindow),
    /// Whether it is on screen, by the application's own notion of open.
    shown: fn(&FocusWindow) -> bool,
}

fn escape(window: &FocusWindow) {
    support::press(window, "Escape", gtk::gdk::ModifierType::empty());
}

/// The digest comes first: its row is reached by a fixed walk from the top
/// of the list, which the surfaces after it would have moved the cursor from.
/// Settings comes last: keys pressed while its dialog is still closing go to
/// the dialog's search entry, so nothing that opens by a bare key follows it.
fn surfaces() -> Vec<Surface> {
    vec![
        Surface {
            name: "the digest window",
            open: |window| {
                support::keys(window, &["j"]);
                support::press(window, "Return", gtk::gdk::ModifierType::empty());
            },
            shown: |window| window.digest().is_some(),
        },
        Surface {
            name: "the open message",
            open: |window| support::keys(window, &["j", "Return"]),
            shown: |window| window.reading().is_some_and(|reading| reading.is_open()),
        },
        Surface {
            name: "the snooze picker",
            open: |window| support::keys(window, &["j", "s"]),
            shown: |window| window.open_picker().is_some_and(|picker| picker.is_shown()),
        },
        Surface {
            name: "the command bar",
            open: |window| {
                support::press(window, "k", gtk::gdk::ModifierType::CONTROL_MASK);
            },
            shown: |window| window.bar().is_some_and(|bar| bar.is_open()),
        },
        Surface {
            name: "the key map",
            open: |window| {
                support::press(window, "question", gtk::gdk::ModifierType::SHIFT_MASK);
            },
            shown: |window| window.key_map().is_some_and(|dialog| dialog.is_mapped()),
        },
        Surface {
            name: "the composer",
            open: |window| support::keys(window, &["c"]),
            shown: |window| {
                window
                    .compose_dialog()
                    .is_some_and(|dialog| dialog.is_mapped())
            },
        },
        Surface {
            name: "settings",
            open: |window| {
                support::deliver_with(window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
            },
            shown: |window| {
                window
                    .settings()
                    .is_some_and(|settings| settings.is_open() && settings.panel().is_mapped())
            },
        },
    ]
}

/// The tallest row on screen, measured at a fixed width.
fn row_height(window: &FocusWindow) -> i32 {
    window
        .pane()
        .expect("the list")
        .rows_on_screen()
        .iter()
        .map(|row| row.measure(gtk::Orientation::Vertical, 404).1)
        .max()
        .unwrap_or(0)
}

pub fn every_widget_in_focus_s_surfaces_has_a_role_and_a_name() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _delivery, _held) = crate::digest::delivered_holding().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "the inbox and its digest never reached the screen: {:?}",
            support::subjects(&window)
        );
        // The harness proves the backend is recording in `a11y.rs`; a probe
        // here keeps this audit honest on its own.
        let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        probe.update_property(&[gtk::accessible::Property::Label("probe")]);
        assert!(
            gtk::test_accessible_has_property(&probe, AccessibleProperty::Label),
            "GTK is recording no accessible properties: the suite runs under \
             GTK_A11Y=test, so something has overridden it"
        );

        // The landmark: the list is a list a screen reader can navigate, and
        // its rows are rows.
        let pane = window.pane().expect("the list");
        assert!(
            gtk::test_accessible_has_role(pane.view(), AccessibleRole::List),
            "the list is not announced as a list"
        );
        for row in pane.rows_on_screen() {
            assert!(
                gtk::test_accessible_has_role(&row, AccessibleRole::Row),
                "a row is not announced as a row"
            );
        }
        expect_usable(&window, "the inbox");

        for surface in surfaces() {
            (surface.open)(&window);
            assert!(
                crate::settle_until(async || (surface.shown)(&window)).await,
                "{}: it did not open, so the audit below would have walked \
                 the inbox again and passed for free",
                surface.name
            );
            crate::settle();
            expect_usable(&window, surface.name);
            escape(&window);
            assert!(
                crate::settle_until(async || !(surface.shown)(&window)).await,
                "{}: Escape did not close it, so every surface audited after \
                 it would be audited through this one",
                surface.name
            );
        }
    });
}

/// 96 dpi in 1024ths is GTK's own default. Anchored rather than
/// read-and-doubled: a headless compositor has no font configuration, and
/// doubling an unset DPI is not a DPI.
fn normal_dpi(settings: &gtk::Settings) -> i32 {
    match settings.gtk_xft_dpi() {
        configured if configured > 0 => configured,
        _ => 96 * 1024,
    }
}

/// At 200% text the list and an open message are still usable: everything a
/// screen reader meets is still named, and a message still opens.
pub fn at_200_percent_text_the_window_stays_usable() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let settings = gtk::Settings::default().expect("a settings object");
        let normal = normal_dpi(&settings);
        settings.set_gtk_xft_dpi(normal * 2);
        crate::settle();
        expect_usable(&window, "the inbox at 200% text");
        support::keys(&window, &["j", "Return"]);
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|reading| reading.is_open()))
                .await,
            "a message does not open at 200% text"
        );
        crate::settle();
        expect_usable(&window, "an open message at 200% text");
        settings.set_gtk_xft_dpi(normal);
        crate::settle();
    });
}

/// Rows grow with the type: if a row's height came from constants rather
/// than from the cascade, it would not move at all and the text would
/// overflow it. Held out (see `IGNORED`): Focus's rows are 40 and 72 pixels
/// whatever the text size.
pub fn at_200_percent_text_rows_grow_with_the_type() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let settings = gtk::Settings::default().expect("a settings object");
        let normal = normal_dpi(&settings);
        settings.set_gtk_xft_dpi(normal);
        crate::settle();
        let before = row_height(&window);
        settings.set_gtk_xft_dpi(normal * 2);
        crate::settle();
        let scaled = row_height(&window);
        settings.set_gtk_xft_dpi(normal);
        crate::settle();
        assert!(
            scaled as f32 > before as f32 * 1.4,
            "rows are {before}px at 100% and {scaled}px at 200%: the type is \
             not coming from the cascade"
        );
    });
}
