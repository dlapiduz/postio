//! Focus as a screen reader meets it (FR-096, T142): a row announces its
//! sender, subject, first line, unread state and marker, and a keycap is
//! announced once, as its control's shortcut, never read as text.
//!
//! GTK records accessible properties only when an accessibility backend is
//! running; the harness runs the suite under `GTK_A11Y=test`, and
//! [`require_an_accessibility_backend`] proves it before anything here
//! draws a conclusion (the classic app's `gtk_accessibility.rs` tells why).
//! The Orca pass is by hand.

use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRole};

use crate::support;

fn require_an_accessibility_backend() {
    let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    probe.update_property(&[gtk::accessible::Property::Label("probe")]);
    assert!(
        gtk::test_accessible_has_property(&probe, AccessibleProperty::Label),
        "GTK is recording no accessible properties: the suite runs under \
         GTK_A11Y=test, so something has overridden it"
    );
}

/// Every widget under `root` that is a control carrying a keycap.
fn controls_with_caps(root: &gtk::Widget) -> Vec<(gtk::Widget, gtk::Widget)> {
    let mut found = Vec::new();
    for cap in support::with_class(root, "postio-keyhint") {
        if !cap.is_visible() {
            continue;
        }
        let mut up = cap.parent();
        while let Some(widget) = up {
            if widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>() {
                found.push((widget, cap.clone()));
                break;
            }
            up = widget.parent();
        }
    }
    found
}

pub fn a_row_announces_its_marker_and_every_keycap_is_its_control_s_shortcut() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        require_an_accessibility_backend();
        let fixture = support::Fixture::empty().await;
        let (budget, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "The numbers are attached.",
                10,
            )
            .await;
        fixture
            .ask(budget, "Can you approve these by Friday?")
            .await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the list");
        let row = || pane.rows_on_screen().into_iter().next().expect("a row");
        assert!(
            crate::settle_until(async || !row().drawn().actions.is_empty()).await,
            "the marked row never drew its marker"
        );

        // The row: a Row, named, and its name says everything a sighted
        // person reads on it, the marker included.
        let row = row();
        assert!(gtk::test_accessible_has_role(&row, AccessibleRole::Row));
        assert!(gtk::test_accessible_has_property(
            &row,
            AccessibleProperty::Label
        ));
        let spoken = row.spoken();
        for part in [
            "Ada Moreno",
            "Atlas budget",
            "The numbers are attached.",
            "unread",
            "Question",
            "Can you approve these by Friday?",
        ] {
            assert!(
                spoken.contains(part),
                "the row does not say {part:?}: {spoken}"
            );
        }
        // Its drawn Reply button's key is the row's shortcut, not text.
        assert!(
            gtk::test_accessible_has_property(&row, AccessibleProperty::KeyShortcuts),
            "the row's action keys are not its shortcuts"
        );

        // Every keycap on a control is presentation, and its control
        // carries the key as its shortcut: the chrome and the bulk bar.
        support::keys(&window, &["j", "x"]);
        crate::settle();
        let found = controls_with_caps(window.upcast_ref());
        assert!(
            found.len() >= 5,
            "the top bar, the strip and the bulk bar carry caps: {}",
            found.len()
        );
        for (control, cap) in found {
            assert!(
                gtk::test_accessible_has_role(&cap, AccessibleRole::Presentation),
                "a cap is read as text"
            );
            assert!(
                gtk::test_accessible_has_property(&control, AccessibleProperty::KeyShortcuts),
                "a control with a cap ({:?}) does not announce its key as its shortcut",
                support::texts(&control)
            );
        }
    });
}
