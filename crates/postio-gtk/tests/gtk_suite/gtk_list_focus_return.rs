//! #437: focus returning to the list from whatever comes after it in the
//! window's tab order has to land on the row the keyboard is on — the
//! **cursor** — not on wherever GTK's own default keynav traversal decides
//! to put it.
//!
//! No custom Tab/Shift-Tab handling exists anywhere in this crate, which is
//! itself the reason this is worth its own test: the bug is not "what does
//! Postio do with Shift-Tab", it is "what does `GtkListView` do when GTK's
//! *built-in* focus-chain traversal re-enters it". That path is only ever
//! exercised for real by `GtkWidget::child_focus` — driven, in the running
//! app, by the Tab/Shift-Tab keybindings the toplevel already has — so this
//! drives that entry point directly rather than a stand-in.
//!
//! Isolated to a bare `MessageListView` in a plain `gtk::Window`, on the same
//! grounds `gtk_list_select_message.rs` and `gtk_selection.rs` already use:
//! the full `Window` and a real `WebKitWebView` reader would make the "what
//! comes after the list" widget slow and unreliable to drive, and the
//! property under test lives entirely inside `MessageListView`. Skips
//! without a display. Nothing here touches the network.

use crate::pump;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::list_view::MessageListView;
use postio_gtk::row::MessageRowView;
use postio_gtk::{fonts, style};
use postio_model::ids::MessageId;

const ROWS: u32 = 6;

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
}

fn row(position: u32) -> Row {
    Row {
        id: MessageId::new(position as i64 + 1),
        thread: None,
        from: Some(postio_model::address::EmailAddress::new(
            Some("Ada Lovelace"),
            "ada@example.com",
        )),
        subject: Some(format!("Note {position}")),
        preview: Some("…".into()),
        received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 0, 0).unwrap(),
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
        participants: Vec::new(),
    }
}

/// Which message id's row currently has real GTK keyboard focus, if any.
///
/// `MessageRowView` is deliberately *not* the focusable widget — the factory
/// that builds it calls `set_focusable(false)` and leaves the `GtkListItem`
/// wrapper around it as "the accessible row" instead (`list_view.rs`'s own
/// comment: "a focusable widget that also calls itself presentational is a
/// contradiction GTK does not survive"). So the widget GTK actually focuses
/// is each row's *parent*, and the only way to ask "which row" is to compare
/// that parent against every row's parent in turn — the same traversal
/// `MessageListView::each_row` already does, reused here for its wrapper
/// rather than its content.
///
/// Deliberately not `has_focus()`: `gtk_focus_visible.rs` documents why —
/// GTK gates `has-focus` on the toplevel being *active*, which a headless
/// window never is. `GtkWindowExt::focus` is the property that tracks the
/// focus widget regardless, and what `window.rs`'s own `focused_type` reads
/// for exactly this reason.
fn cursor_message_of_focused_row(
    pane: &MessageListView,
    window: &gtk::Window,
) -> Option<MessageId> {
    let focused = gtk::prelude::GtkWindowExt::focus(window)?;
    let found: std::cell::RefCell<Option<MessageId>> = std::cell::RefCell::new(None);
    pane.each_row(|view: &MessageRowView| {
        if view.parent().as_ref() == Some(&focused) {
            *found.borrow_mut() = view.row().map(|r| r.id);
        }
    });
    found.into_inner()
}

/// A list with six rows delivered, a button on either side of it in the tab
/// order, and the cursor already on the middle row — not the first, not the
/// last, so a fix that merely swapped which end of the list wins would still
/// fail this.
///
/// Returns `None` if there is no display, in which case the caller should
/// skip: matches every other file's `if adw::init()...` guard, just hoisted
/// out since two functions here need it.
fn a_list_with_neighbours_and_a_mid_list_cursor() -> Option<(
    gtk::Window,
    MessageListView,
    gtk::Button,
    gtk::Button,
    MessageId,
)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = MessageListView::new();
    // Standing in for whatever sits on either side of the list in the
    // window's tab order — the sidebar before it, the reading pane after.
    // Plain buttons are enough: what matters is that focus can leave the
    // list and come back from either direction.
    let before = gtk::Button::with_label("before");
    let after = gtk::Button::with_label("after");

    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    layout.append(&before);
    layout.append(&pane);
    layout.append(&after);

    let window = gtk::Window::new();
    window.set_default_size(600, 600);
    window.set_child(Some(&layout));
    window.present();
    pump();

    pane.model().set_source(Rc::new(Pages));
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();

    let middle = MessageId::new(4);
    pane.select_message(middle);
    pump();
    assert_eq!(
        pane.cursor_id(),
        Some(middle),
        "the cursor should be on the middle row before anything about focus \
         is asked of it"
    );

    // Establishing that the list can genuinely take real widget focus on
    // that row is the precondition every case below depends on -- and it is
    // already proven correct, deliberately not what either case re-tests:
    // `grab_focus()` calling `self.view.grab_focus()` has never been the bug,
    // only the separate path GTK's own keynav takes to reach this widget.
    pane.grab_focus();
    pump();
    assert_eq!(
        cursor_message_of_focused_row(&pane, &window),
        Some(middle),
        "grabbing focus on the list should land real widget focus on the \
         cursor row, which is the precondition every case below depends on"
    );

    Some((window, pane, before, after, middle))
}

pub fn shift_tab_from_after_the_list_returns_to_the_cursor_row() {
    let Some((window, pane, _before, after, middle)) =
        a_list_with_neighbours_and_a_mid_list_cursor()
    else {
        return;
    };

    // ── focus moves on, the way reading the message would ─────────────────
    after.grab_focus();
    pump();
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&window).as_ref(),
        Some(after.upcast_ref::<gtk::Widget>()),
        "the button after the list should now have it"
    );

    // ── Shift-Tab: the actual mechanism, not a stand-in for it ─────────────
    //
    // A real Shift-Tab press is the toplevel's own Tab/ISO_Left_Tab
    // keybinding, which calls exactly this on the widget that has focus.
    // Nothing in this crate intercepts it, so driving `child_focus` directly
    // exercises the identical path a keystroke would.
    let moved = window.child_focus(gtk::DirectionType::TabBackward);
    pump();
    assert!(
        moved,
        "there should be somewhere for focus to go backward to"
    );

    assert_eq!(
        cursor_message_of_focused_row(&pane, &window),
        Some(middle),
        "Shift-Tab back into the list should return focus to the cursor row \
         -- the message actually being read -- not wherever GTK's default \
         focus-chain traversal happens to land"
    );
}

/// The reported bug is Shift-Tab specifically, but the same GTK mechanism
/// handles plain Tab entering from the *other* side — from the sidebar, in
/// the real app. A fix narrow enough to special-case "backward" would leave
/// this direction defaulting to row 0, which the acceptance criteria rules
/// out just as explicitly as the last row.
pub fn tab_from_before_the_list_also_returns_to_the_cursor_row() {
    let Some((window, pane, before, _after, middle)) =
        a_list_with_neighbours_and_a_mid_list_cursor()
    else {
        return;
    };

    before.grab_focus();
    pump();
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&window).as_ref(),
        Some(before.upcast_ref::<gtk::Widget>()),
        "the button before the list should now have it"
    );

    let moved = window.child_focus(gtk::DirectionType::TabForward);
    pump();
    assert!(
        moved,
        "there should be somewhere for focus to go forward to"
    );

    assert_eq!(
        cursor_message_of_focused_row(&pane, &window),
        Some(middle),
        "Tab forward into the list should land on the cursor row too, not \
         on row 0 the way an ordinary container's default first-child \
         behaviour would"
    );
}
