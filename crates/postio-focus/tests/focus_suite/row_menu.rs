//! T199: a right-click on a list row opens a menu of the row's verbs, each
//! with its key, each running its one registry command. The right-click
//! moves the cursor to the row, as a click does. Inside the selection the
//! menu acts on the selection; outside it, on that row alone -- and the
//! selection is only let go when a verb runs, so dismissing the menu loses
//! nothing. Escape closes it.

use gtk::gdk;
use gtk::prelude::*;
use postio_core::state::Selection;

use crate::support::{self, Fixture};

/// Right-click the `index`th row on screen as GTK would run it: the list's
/// secondary-button gesture, pressed at the row's middle.
pub(crate) fn right_click(window: &postio_focus::window::FocusWindow, index: usize) {
    let pane = window.pane().expect("the inbox");
    let view = pane.view().clone();
    let rows = pane.rows_on_screen();
    let row = rows.get(index).expect("a row on screen");
    let middle = row
        .compute_point(
            &view,
            &gtk::graphene::Point::new(row.width() as f32 / 2.0, row.height() as f32 / 2.0),
        )
        .expect("the row is in the list");
    let controllers = view.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|at| controllers.item(at).and_downcast::<gtk::GestureClick>())
        .find(|gesture| gesture.button() == gdk::BUTTON_SECONDARY)
        .expect("the list answers the secondary button");
    gesture.emit_by_name::<()>(
        "pressed",
        &[&1i32, &f64::from(middle.x()), &f64::from(middle.y())],
    );
    crate::settle();
}

/// The menu, up and drawn.
pub(crate) async fn menu_shown(window: &postio_focus::window::FocusWindow) -> gtk::Popover {
    assert!(
        crate::settle_until(async || window
            .row_menu()
            .is_some_and(|menu| menu.is_open() && menu.widget().width() > 0))
        .await,
        "no menu opened on the row"
    );
    window.row_menu().expect("the menu").widget().clone()
}

/// Press the menu's verb that wears `class`.
pub(crate) fn choose(menu: &gtk::Popover, class: &str) {
    support::only(menu, class)
        .downcast::<gtk::Button>()
        .expect("a button")
        .emit_clicked();
    crate::settle();
}

/// Toggle the selection of the rows on screen at `indices`, as a press in
/// each one's gutter does.
fn select_on_screen(window: &postio_focus::window::FocusWindow, indices: &[usize]) {
    for index in indices {
        let rows = window.pane().expect("the inbox").rows_on_screen();
        assert!(
            rows[*index].press_at(30.0, 20.0),
            "the gutter took the press"
        );
        crate::settle();
    }
}

fn selected(window: &postio_focus::window::FocusWindow) -> usize {
    match window.selection() {
        Selection::These(picked) => picked.len(),
        Selection::Everything { .. } => usize::MAX,
    }
}

/// The menu lists the row's verbs, in their groups, each with the key the
/// keymap gives it; and the right-click moved the cursor to the row.
pub fn a_right_click_on_a_row_offers_its_verbs_with_their_keys() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        right_click(&window, 2);
        let menu = menu_shown(&window).await;
        assert_eq!(
            window.pane().expect("the inbox").cursor().selected(),
            2,
            "the cursor went to the row that was right-clicked"
        );
        let said = support::texts(&menu);
        let keymap = window.keymap();
        for (words, command) in [
            ("Open", postio_core::CommandId::OpenMessage),
            ("Reply", postio_core::CommandId::Reply),
            ("Archive", postio_core::CommandId::Archive),
            ("Snooze\u{2026}", postio_core::CommandId::Snooze),
            ("Label\u{2026}", postio_core::CommandId::AddLabel),
            ("Move\u{2026}", postio_core::CommandId::Move),
            ("Delete", postio_core::CommandId::Delete),
        ] {
            let at = said
                .iter()
                .position(|text| text == words)
                .unwrap_or_else(|| panic!("no {words:?} in the menu: {said:?}"));
            let key = keymap.binding(command).expect("a bound verb");
            assert_eq!(
                said.get(at + 1).map(String::as_str),
                Some(key),
                "{words:?} does not show its key {key:?}: {said:?}"
            );
        }
        assert!(
            said.iter()
                .any(|text| text == "Mark read" || text == "Mark unread"),
            "no read verb: {said:?}"
        );
        let rules = support::with_class(&menu, "focus-row-menu-rule");
        assert_eq!(rules.len(), 4, "a rule between each of the five groups");
        for rule in &rules {
            assert!(
                rule.is_mapped() && rule.height() >= 1,
                "a group's rule is not drawn: {}x{}, mapped {}, visible {}",
                rule.width(),
                rule.height(),
                rule.is_mapped(),
                rule.is_visible()
            );
        }
        assert!(
            support::keycaps_are_taught(&menu),
            "the menu's keys are not its items' shortcuts"
        );
    });
}

/// A verb runs its command on the right-clicked row: Archive takes that
/// row, and only that row, out of the inbox.
pub fn a_menu_verb_runs_its_command_on_the_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let before = support::subjects(&window);
        right_click(&window, 1);
        let menu = menu_shown(&window).await;
        choose(&menu, "focus-row-menu-archive");
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == before.len() - 1)
                .await,
            "Archive from the menu did not take a row out"
        );
        let after = support::subjects(&window);
        assert!(
            !after.contains(&before[1]) && after.contains(&before[0]),
            "the wrong row went: {before:?} became {after:?}"
        );
        assert!(!menu.is_visible(), "the menu stayed up after its verb ran");
    });
}

/// Outside the selection, the menu is for the row alone: dismissing it
/// keeps the selection, and a verb lets the selection go and acts on the
/// row. Inside the selection, it acts on the selection, and says so.
pub fn a_right_click_outside_the_selection_is_for_that_row_inside_it_for_the_selection() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let before = support::subjects(&window);
        // Select the first two rows, by their gutters.
        select_on_screen(&window, &[0, 1]);
        assert_eq!(selected(&window), 2);

        // Outside: row 3. Escape closes the menu and the selection stays.
        right_click(&window, 3);
        let menu = menu_shown(&window).await;
        assert!(
            !support::texts(&menu).iter().any(|t| t.contains("selected")),
            "the menu for an unselected row names the selection"
        );
        assert!(support::deliver(&window, "Escape"), "Escape was not taken");
        assert!(!menu.is_visible(), "Escape did not close the menu");
        assert_eq!(
            selected(&window),
            2,
            "closing the menu dropped the selection"
        );

        // Inside: row 1. The menu names the selection and offers no reply.
        right_click(&window, 1);
        let menu = menu_shown(&window).await;
        let said = support::texts(&menu);
        assert!(
            said.iter().any(|t| t == "2 selected"),
            "the menu does not say it acts on the selection: {said:?}"
        );
        assert!(
            !said.iter().any(|t| t == "Reply"),
            "Reply on a selection: {said:?}"
        );
        choose(&menu, "focus-row-menu-archive");
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == before.len() - 2)
                .await,
            "Archive on the selection did not take both rows"
        );
        let after = support::subjects(&window);
        assert!(after.contains(&before[3]), "{after:?}");

        // Outside again, with a fresh selection: the verb is for the row.
        select_on_screen(&window, &[0]);
        assert_eq!(selected(&window), 1);
        let kept = support::subjects(&window)[0].clone();
        let target = support::subjects(&window)[2].clone();
        right_click(&window, 2);
        let menu = menu_shown(&window).await;
        choose(&menu, "focus-row-menu-archive");
        assert!(
            crate::settle_until(async || !support::subjects(&window).contains(&target)).await,
            "the right-clicked row was not archived"
        );
        assert!(
            support::subjects(&window).contains(&kept),
            "the selection was archived instead of the right-clicked row"
        );
        assert_eq!(selected(&window), 0, "the selection was not let go");
    });
}
