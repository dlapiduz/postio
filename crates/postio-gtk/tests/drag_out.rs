//! The panes a drag crosses scroll under it. The drag's provider moved to
//! `postio-widgets` (T245), and its tests with it (`widgets_suite`'s
//! `drag_out`); what stays is the classic sidebar's autoscroll.
//!
//! One test function, deliberately: GTK initialises once per process and only
//! from one thread.

use gtk::prelude::*;

#[test]
fn the_panes_a_drag_crosses_can_scroll_under_it() {
    if gtk::init().is_err() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    scroll_check();
}

/// A folder below the fold has to be reachable without putting the drag down.
///
/// What this proves is that the controller is *attached* — the failure it
/// exists to catch is nobody calling `autoscroll::attach`, which leaves a
/// perfectly correct ramp wired to nothing. How far each tick moves is
/// `autoscroll`'s own unit tests; emitting a real drag motion needs a real
/// drag, which no headless harness can start.
fn scroll_check() {
    fn scrolls_under_a_drag(widget: &gtk::Widget) -> bool {
        let mut queue = vec![widget.clone()];
        while let Some(current) = queue.pop() {
            if current.downcast_ref::<gtk::ScrolledWindow>().is_some() {
                let controllers = current.observe_controllers();
                for index in 0..controllers.n_items() {
                    if controllers
                        .item(index)
                        .and_downcast::<gtk::DropControllerMotion>()
                        .is_some()
                    {
                        return true;
                    }
                }
            }
            let mut child = current.first_child();
            while let Some(node) = child {
                queue.push(node.clone());
                child = node.next_sibling();
            }
        }
        false
    }

    let sidebar = postio_gtk::sidebar::Sidebar::default();
    assert!(
        scrolls_under_a_drag(sidebar.upcast_ref()),
        "a folder below the fold cannot be dropped on: the sidebar does not scroll under a drag"
    );

    let list = postio_gtk::list_view::MessageListView::default();
    assert!(
        scrolls_under_a_drag(list.upcast_ref()),
        "the message list does not scroll under a drag"
    );
}
