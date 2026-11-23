//! Every field `FocusWindow::observe` reports must come from what is on screen: a field read from
//! what a layer was told cannot fail when the wiring breaks.

use gtk::prelude::*;
use postio_ui::keymap::Chord;
use postio_ui::observe::{Overlay, Region, Tone, View};
use postio_widgets::storyboard::deliver;

use crate::support;

#[track_caller]
fn press(window: &postio_gtk::window::FocusWindow, chord: &str) {
    let chord: Chord = chord.parse().expect("a chord");
    let delivery = deliver::press(window.upcast_ref(), &chord).expect("a deliverable chord");
    assert!(
        matches!(delivery, deliver::Delivery::Delivered { .. }),
        "`{chord:?}` was dropped on the way to the window"
    );
    crate::settle();
}

pub fn the_window_says_where_the_keyboard_cursor_and_notices_are() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;

        let seen = window.observe();
        assert_eq!(seen.keyboard.region, Region::List, "{seen:#?}");
        assert!(seen.keyboard.reachable, "{seen:#?}");

        let before = window.observe().cursor.index;
        press(&window, "j");
        let seen = window.observe();
        assert_eq!(seen.keyboard.region, Region::List, "{seen:#?}");
        assert!(seen.keyboard.reachable, "{seen:#?}");
        assert!(!seen.keyboard.typing);
        assert_eq!(seen.view, View::List);
        assert_eq!(seen.overlay.kind, Overlay::None);
        assert_eq!(seen.cursor.index, Some(before.map_or(0, |at| at + 1)));
        assert!(seen.cursor.id.is_some() && seen.cursor.subject.is_some());
        assert_eq!(seen.rows.count, Some(3));
        assert_eq!(seen.back_depth, None, "Focus's Back is a cascade");
        let cursor = seen.cursor.clone();
        assert!(seen.app.contains_key("focus.bulk"), "{:?}", seen.app.keys());
        assert_eq!(seen.selection.count, 0);

        press(&window, "/");
        let seen = window.observe();
        assert_eq!(seen.overlay.kind, Overlay::Finder, "{seen:#?}");
        assert_eq!(seen.keyboard.region, Region::Search);
        assert!(seen.keyboard.typing, "the bar's field takes text");
        assert_eq!(seen.app["focus.bar.typed"], "", "{:?}", seen.app);
        assert_eq!(seen.app["focus.bar.messages"], 0, "{:?}", seen.app);

        press(&window, "Escape");
        let seen = window.observe();
        assert_eq!(seen.keyboard.region, Region::List, "{seen:#?}");
        assert_eq!(seen.overlay.kind, Overlay::None);
        assert_eq!(seen.cursor, cursor);

        press(&window, "x");
        let seen = window.observe();
        assert_eq!(seen.selection.count, 1, "{seen:#?}");
        let bulk = seen.app.get("focus.bulk").expect("the bulk bar's state");
        assert_eq!(bulk["shown"], true, "{bulk}");
        assert!(bulk["summary"].is_string(), "{bulk}");
        press(&window, "x");
        let seen = window.observe();
        assert_eq!(seen.selection.count, 0);
        assert_eq!(seen.app["focus.bulk"]["shown"], false);

        press(&window, "Return");
        let seen = window.observe();
        assert_eq!(seen.view, View::Reader, "{seen:#?}");
        assert_eq!(seen.keyboard.region, Region::Reader);
        assert_eq!(seen.reading.id, cursor.id, "the dialog names its message");
        assert!(seen.reading.scroll.is_some(), "{seen:#?}");

        // The keyboard starts on the message, which a key can reach once the dialog has drawn its
        // first frame.
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.reader().view().is_mapped()).await,
            "the dialog never drew"
        );
        press(&window, "Escape");
        assert!(
            crate::settle_until(async || window.observe().keyboard.region == Region::List).await,
            "the dialog never closed"
        );
        let seen = window.observe();
        assert_eq!(seen.view, View::List, "{seen:#?}");
        assert_eq!(seen.keyboard.region, Region::List);
        assert_eq!(seen.cursor, cursor);
        assert_eq!(seen.reading.id, None);

        press(&window, "e");
        assert!(
            crate::settle_until(async || window.observe().notice.text.is_some()).await,
            "no notice after archive"
        );
        let seen = window.observe();
        assert!(seen.notice.undo, "{seen:#?}");
        assert_eq!(seen.notice.tone, Some(Tone::Info));

        press(&window, "r");
        assert!(
            crate::settle_until(async || window.observe().composer.open).await,
            "no composer after reply"
        );
        let seen = window.observe();
        assert_eq!(seen.view, View::Composer, "{seen:#?}");
        assert_eq!(seen.keyboard.region, Region::Composer);
        assert_eq!(seen.keyboard.field.as_deref(), Some("body"), "{seen:#?}");
    });
}
