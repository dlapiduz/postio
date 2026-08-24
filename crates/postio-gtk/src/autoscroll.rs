//! Scrolling a pane while a drag hovers near its edge.
//!
//! A drag is a held pointer, so the usual ways of scrolling are unavailable
//! mid-gesture: the wheel is awkward with a button down, and the scrollbar is
//! somewhere else. Without this, a folder below the fold cannot be dropped on
//! at all — the user has to abandon the drag, scroll, and start again.
//!
//! # The ramp is the whole design
//!
//! A fixed step makes the edge feel like a switch: nothing, then a lurch past
//! the target. The speed instead ramps with how far into the edge zone the
//! pointer has gone, so resting just inside the zone creeps and pressing to
//! the very edge moves quickly. That is what makes it possible to stop on the
//! folder you wanted.

/// How deep the edge zone reaches, in pixels.
///
/// Two rows' worth at the airy density. Wide enough to enter without aiming,
/// narrow enough that the middle of a short sidebar is still still.
pub const MARGIN: f64 = 48.0;

/// The most one tick may move, in pixels. See [`TICK`].
pub const MAX_STEP: f64 = 24.0;

/// How often a scroll tick fires while the pointer is in the zone.
///
/// About one frame at 60Hz, so [`MAX_STEP`] is roughly 1440 px/s at the very
/// edge — brisk without overshooting a folder list.
pub const TICK: std::time::Duration = std::time::Duration::from_millis(16);

/// How far to scroll this tick, with the pointer at `y` in a pane `height`
/// tall.
///
/// Negative scrolls up, positive down, zero holds still. `margin` is how deep
/// the edge zone reaches and `max_step` the most one tick may move.
pub fn step(y: f64, height: f64, margin: f64, max_step: f64) -> f64 {
    // A pane shorter than two zones would have every point in both, and the
    // one nearest the middle would win by accident. Splitting it means the
    // zones meet rather than overlap, and the middle stays still.
    let margin = margin.min(height / 2.0).max(0.0);
    if margin <= 0.0 || !height.is_finite() {
        return 0.0;
    }

    if y < margin {
        // Depth is 0 at the inner edge of the zone and 1 at the pane's edge,
        // and stays 1 outside it: a pointer dragged past the top of the pane
        // is asking for the fastest scroll, not for none.
        -max_step * depth((margin - y) / margin)
    } else if y > height - margin {
        max_step * depth((y - (height - margin)) / margin)
    } else {
        0.0
    }
}

/// How hard the scroll pulls, from a position through the zone.
fn depth(fraction: f64) -> f64 {
    fraction.clamp(0.0, 1.0)
}

/// Scroll `scroller` while a drag hovers near its top or bottom edge.
///
/// Uses `GtkDropControllerMotion`, which reports a drag passing over a widget
/// whether or not that widget is a drop target — the message list is not one,
/// and still has to scroll while a drag crosses it.
///
/// The tick stops when the pointer leaves the zone, when the drag leaves the
/// widget, and when the widget goes away, so nothing is left running after a
/// drag ends.
pub fn attach(scroller: &gtk::ScrolledWindow) {
    use gtk::glib;
    use gtk::prelude::*;

    let motion = gtk::DropControllerMotion::new();
    // The handle for the tick in flight, so entering the zone twice does not
    // start two of them and scroll at double speed.
    let ticking: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>> = Default::default();
    // Where the pointer is *now*. Shared with the running tick rather than
    // captured by it: the whole point of the ramp is that moving deeper into
    // the zone scrolls faster, and a tick holding the position the drag
    // entered at would keep whatever speed it started with for as long as the
    // drag lasted.
    let pointer = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));

    let stop = {
        let ticking = ticking.clone();
        move || {
            if let Some(source) = ticking.borrow_mut().take() {
                source.remove();
            }
        }
    };

    motion.connect_motion(glib::clone!(
        #[weak]
        scroller,
        #[strong]
        ticking,
        #[strong]
        stop,
        #[strong]
        pointer,
        move |_, _, y| {
            pointer.set(y);
            let height = f64::from(scroller.height());
            if step(y, height, MARGIN, MAX_STEP) == 0.0 {
                stop();
                return;
            }
            if ticking.borrow().is_some() {
                // Already running; it re-reads the pointer each tick.
                return;
            }
            let source = glib::timeout_add_local(
                TICK,
                glib::clone!(
                    #[weak]
                    scroller,
                    #[strong]
                    ticking,
                    #[strong]
                    pointer,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        let height = f64::from(scroller.height());
                        let delta = step(pointer.get(), height, MARGIN, MAX_STEP);
                        if delta == 0.0 {
                            ticking.borrow_mut().take();
                            return glib::ControlFlow::Break;
                        }
                        let adjustment = scroller.vadjustment();
                        let upper = adjustment.upper() - adjustment.page_size();
                        adjustment.set_value(
                            (adjustment.value() + delta)
                                .clamp(adjustment.lower(), upper.max(adjustment.lower())),
                        );
                        glib::ControlFlow::Continue
                    }
                ),
            );
            ticking.replace(Some(source));
        }
    ));
    motion.connect_leave(glib::clone!(
        #[strong]
        stop,
        move |_| stop()
    ));
    scroller.add_controller(motion);
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEIGHT: f64 = 400.0;

    fn at(y: f64) -> f64 {
        step(y, HEIGHT, MARGIN, MAX_STEP)
    }

    #[test]
    fn the_middle_of_a_pane_does_not_scroll() {
        assert_eq!(at(HEIGHT / 2.0), 0.0);
        assert_eq!(at(MARGIN + 1.0), 0.0);
        assert_eq!(at(HEIGHT - MARGIN - 1.0), 0.0);
    }

    #[test]
    fn the_top_edge_scrolls_up_and_the_bottom_down() {
        assert!(at(0.0) < 0.0, "the very top must scroll up");
        assert!(at(HEIGHT) > 0.0, "the very bottom must scroll down");
    }

    #[test]
    fn the_speed_ramps_with_how_far_into_the_edge_the_pointer_is() {
        // The property that makes it possible to stop on the folder you meant:
        // a fixed step would feel like a switch, and overshoot.
        let just_inside = at(MARGIN - 1.0).abs();
        let halfway = at(MARGIN / 2.0).abs();
        let at_the_edge = at(0.0).abs();
        assert!(
            just_inside < halfway && halfway < at_the_edge,
            "{just_inside} {halfway} {at_the_edge}"
        );
    }

    #[test]
    fn a_tick_is_bounded() {
        // Dragged well past the edge of the window, which is where a pointer
        // ends up when someone is reaching for a folder near the bottom.
        assert_eq!(at(-500.0), -MAX_STEP);
        assert_eq!(at(HEIGHT + 500.0), MAX_STEP);
    }

    #[test]
    fn a_short_pane_still_has_a_still_middle() {
        // A sidebar shorter than two margins: without splitting the zones,
        // every point would be in both and the pane would scroll wherever the
        // pointer rested.
        let height = MARGIN;
        assert_eq!(step(height / 2.0, height, MARGIN, MAX_STEP), 0.0);
        assert!(step(0.0, height, MARGIN, MAX_STEP) < 0.0);
        assert!(step(height, height, MARGIN, MAX_STEP) > 0.0);
    }

    #[test]
    fn a_pane_with_no_height_does_not_scroll() {
        assert_eq!(step(0.0, 0.0, MARGIN, MAX_STEP), 0.0);
    }
}
