//! Multi-select on a real display: the cursor and the selection as two
//! separate facts, the gestures that move each, and the bar that appears when
//! there is something to act on.
//!
//! One test function, for the reason `gtk_style.rs` gives — a window costs
//! seconds to realise and every assertion here wants the same one. Skips
//! without a display. Nothing here touches the network.
//!
//! Pixels are deliberately *not* asserted: a headless compositor hands back an
//! unpainted window, and "selected draws differently" is `gtk_row.rs`'s job
//! anyway. What is checked here is the state the drawing reads.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::list_view::MessageListView;
use postio_gtk::row::MessageRowView;
use postio_gtk::{fonts, style};
use postio_model::ids::MessageId;

/// Enough rows to select a range in, few enough to fit one screenful.
const ROWS: u32 = 6;

/// A source that answers immediately, so the list has rows to select.
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
    }
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

/// The row widget currently showing `position`, if it is realised.
fn realised_row(pane: &MessageListView, position: u32) -> Option<MessageRowView> {
    let found = RefCell::new(None);
    pane.each_row(|row| {
        if row.index() == position {
            *found.borrow_mut() = Some(row.clone());
        }
    });
    found.into_inner()
}

fn id(position: u32) -> MessageId {
    MessageId::new(position as i64 + 1)
}

#[test]
fn the_cursor_and_the_selection_are_two_different_things() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = MessageListView::new();
    pane.model().set_source(Rc::new(Pages));
    pane.model().deliver(0, (0..ROWS).map(row).collect());

    let window = gtk::Window::new();
    window.set_default_size(404, 600);
    window.set_child(Some(&pane));
    window.present();
    pump();

    // ── nothing is selected until somebody says so ───────────────────────
    assert!(
        pane.selection().is_empty(),
        "a list nobody has acted on has nothing to act on"
    );

    // ── the cursor moves without selecting ───────────────────────────────
    // `j` is "look at the next one", not "and act on it too". A list that
    // selected as the cursor passed would make every bulk action a question
    // of where the keyboard happened to stop.
    pane.first_row();
    pane.next_row();
    pump();
    assert_eq!(pane.cursor().selected(), 1, "the cursor moved");
    assert!(
        pane.selection().is_empty(),
        "moving the cursor selected something"
    );

    // ── `x` selects without moving the cursor ────────────────────────────
    pane.toggle_cursor_row();
    pump();
    assert!(pane.selection().contains(id(1)));
    assert_eq!(pane.cursor().selected(), 1, "toggling moved the keyboard");
    if let Some(view) = realised_row(&pane, 1) {
        assert!(view.is_selected(), "the row shows it is in the selection");
        assert!(view.is_cursor(), "and that the keyboard is on it");
    }
    if let Some(view) = realised_row(&pane, 0) {
        assert!(!view.is_selected());
        assert!(!view.is_cursor());
    }

    // ── `Shift+J` takes the cursor and the selection together ────────────
    pane.extend_down();
    pump();
    assert_eq!(pane.cursor().selected(), 2, "extending moves the keyboard");
    assert!(pane.selection().contains(id(1)));
    assert!(pane.selection().contains(id(2)));

    // ── `x` again takes a row back out ───────────────────────────────────
    pane.toggle_cursor_row();
    pump();
    assert!(!pane.selection().contains(id(2)), "toggled back out");
    assert!(pane.selection().contains(id(1)), "and left the rest alone");

    // ── `Ctrl+A` is a predicate, not six ids ─────────────────────────────
    pane.select_all();
    pump();
    let all = pane.selection().selection();
    assert!(all.is_everything());
    assert_eq!(all.ids(), None, "select-all named no rows");
    assert!(
        pane.selection().contains(MessageId::new(99_999)),
        "including rows this list has never loaded"
    );

    // ── `Esc` gives it back ──────────────────────────────────────────────
    pane.clear_selection();
    pump();
    assert!(pane.selection().is_empty());

    // ── the bulk bar runs the registry's commands ────────────────────────
    // Not its own: a button that archived directly would be a second
    // implementation of a verb `a` already means.
    let ran: Rc<RefCell<Vec<postio_core::Command>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = ran.clone();
    pane.connect_command(move |command| seen.borrow_mut().push(command));

    pane.toggle_cursor_row();
    pump();
    let bar = bulk_buttons(&pane);
    assert_eq!(bar.len(), 3, "three verbs, as the canvas has room for");
    for button in &bar {
        button.emit_clicked();
    }
    pump();
    // The registry's own invocations, aimed at the selection — which is what
    // makes a bulk archive one undo entry rather than twelve.
    let ids: Vec<CommandId> = ran.borrow().iter().map(postio_core::Command::id).collect();
    assert_eq!(
        ids,
        vec![CommandId::Archive, CommandId::Delete, CommandId::Move]
    );
    assert!(
        ran.borrow().iter().all(|command| matches!(
            command,
            postio_core::Command::Archive {
                target: postio_core::MessageTarget::Selection
            } | postio_core::Command::Delete {
                target: postio_core::MessageTarget::Selection
            } | postio_core::Command::Move {
                target: postio_core::MessageTarget::Selection,
                ..
            }
        )),
        "the bar acts on the selection, not on a row it picked itself"
    );

    // ── and the count says what is about to be hit ───────────────────────
    let counts: Vec<String> = labels(&pane)
        .into_iter()
        .filter(|text| text.contains("selected"))
        .collect();
    assert_eq!(counts, vec!["1 selected".to_string()]);

    pane.clear_selection();
    pump();
    assert!(
        labels(&pane).iter().all(|text| !text.contains("selected")),
        "the bar goes away with the selection that put it there"
    );

    // ── the rows are really there ────────────────────────────────────────
    // Everything below reads a realised row, and a list that realised none
    // would make those assertions vacuous rather than false.
    let realised = std::cell::Cell::new(0);
    pane.each_row(|_| realised.set(realised.get() + 1));
    assert!(realised.get() > 0, "no rows were realised");
    let view = realised_row(&pane, 0).expect("a realised first row");

    // ── the mouse reaches the same verbs the keyboard does ───────────────
    assert!(
        !view.offers_actions(),
        "a row nobody is pointing at offers nothing"
    );
    assert!(
        view.action_at(300.0, 20.0).is_none(),
        "and answers no hit test either"
    );

    view.set_hovered(true);
    pump();
    assert!(view.offers_actions());

    // Every action is somewhere on the row, in order, and none of them
    // overlaps another: three glyphs answering the same point would make
    // which one you got a matter of luck.
    let width = view.width().max(404);
    let mut seen: Vec<postio_gtk::row::RowAction> = Vec::new();
    for x in 0..width {
        if let Some(action) = view.action_at(x as f64, 22.0)
            && seen.last() != Some(&action)
        {
            seen.push(action);
        }
    }
    assert_eq!(
        seen,
        postio_gtk::row::RowAction::ALL.to_vec(),
        "the three actions, left to right, each in one place"
    );

    // `[ui].show_hover_actions = false` takes the offer off the row. The
    // verbs stay reachable by key and through the context menu; what goes is
    // the row's own invitation.
    view.set_show_actions(false);
    pump();
    assert!(!view.offers_actions());
    assert!(view.action_at(300.0, 22.0).is_none());

    window.close();
}

/// Every button in the pane's header, in order.
fn bulk_buttons(pane: &MessageListView) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    walk(pane.clone().upcast(), &mut |widget| {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && widget.is_visible()
        {
            found.push(button);
        }
    });
    found
}

/// The text of every visible label in the pane.
fn labels(pane: &MessageListView) -> Vec<String> {
    let mut found = Vec::new();
    walk(pane.clone().upcast(), &mut |widget| {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>()
            && widget.is_visible()
        {
            found.push(label.text().to_string());
        }
    });
    found
}

fn walk(widget: gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(&widget);
    let mut child = widget.first_child();
    while let Some(current) = child {
        walk(current.clone(), visit);
        child = current.next_sibling();
    }
}
