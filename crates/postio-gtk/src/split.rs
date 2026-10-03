//! The list and the reading pane, side by side (T232; screens.md, "Reading
//! beside the list").
//!
//! A layout manager of its own, so the two widths come from the allocation
//! itself -- the pane `postio_ui::focus_dialog::pane_width` of the split's
//! width, the list the rest -- and never from a size request: a request the
//! window would refuse to shrink below, so a window could never be narrowed
//! past the point where the pane should give way to the dialog. The pane
//! asks only for what its content cannot do without.
//!
//! The first child is the list, the second the pane. A hidden pane gives
//! the list the whole width.

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_ui::focus_dialog;

/// Told the split's width at every allocation.
type WidthHandler = Box<dyn Fn(i32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SplitLayout {
        pub on_width: RefCell<Option<WidthHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplitLayout {
        const NAME: &'static str = "PostioFocusSplitLayout";
        type Type = super::SplitLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for SplitLayout {}

    impl LayoutManagerImpl for SplitLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let (list, pane) = children(widget);
            let measure = |child: Option<&gtk::Widget>| match child {
                Some(child) if child.is_visible() => {
                    let (min, nat, _, _) = child.measure(orientation, for_size);
                    (min, nat)
                }
                _ => (0, 0),
            };
            let (list_min, list_nat) = measure(list.as_ref());
            let (pane_min, pane_nat) = measure(pane.as_ref());
            match orientation {
                gtk::Orientation::Horizontal => (list_min + pane_min, list_nat + pane_nat, -1, -1),
                _ => (list_min.max(pane_min), list_nat.max(pane_nat), -1, -1),
            }
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            if let Some(on_width) = self.on_width.borrow().as_ref() {
                on_width(width);
            }
            let (list, pane) = children(widget);
            let pane = pane.filter(gtk::Widget::is_visible);
            let pane_width = pane.as_ref().map_or(0, |pane| {
                let (min, _, _, _) = pane.measure(gtk::Orientation::Horizontal, -1);
                let list_min = list
                    .as_ref()
                    .map_or(0, |list| list.measure(gtk::Orientation::Horizontal, -1).0);
                split(width, min, list_min)
            });
            if let Some(list) = list.filter(gtk::Widget::is_visible) {
                list.size_allocate(&gtk::Allocation::new(0, 0, width - pane_width, height), -1);
            }
            if let Some(pane) = pane {
                pane.size_allocate(
                    &gtk::Allocation::new(width - pane_width, 0, pane_width, height),
                    -1,
                );
            }
        }
    }
}

glib::wrapper! {
    /// The split's layout: the list, and the reading pane at its right.
    pub struct SplitLayout(ObjectSubclass<imp::SplitLayout>)
        @extends gtk::LayoutManager;
}

impl SplitLayout {
    /// A layout telling `on_width` the split's width at every allocation.
    pub fn new(on_width: impl Fn(i32) + 'static) -> Self {
        let layout: Self = glib::Object::new();
        layout.imp().on_width.replace(Some(Box::new(on_width)));
        layout
    }
}

/// The split's two children: the list, then the pane.
fn children(widget: &gtk::Widget) -> (Option<gtk::Widget>, Option<gtk::Widget>) {
    let list = widget.first_child();
    let pane = list.as_ref().and_then(gtk::Widget::next_sibling);
    (list, pane)
}

/// The pane's width in a split `width` wide: the geometry's, kept at least
/// `pane_min` (what its content cannot do without) and, beyond that, short
/// of the list's own `list_min`. Between the moment a window narrows past
/// the pane's room and the moment the pane gives way, the geometry has no
/// width to give, and the pane keeps its floor.
fn split(width: i32, pane_min: i32, list_min: i32) -> i32 {
    focus_dialog::pane_width(width)
        .unwrap_or(focus_dialog::PANE_MIN)
        .min(width - list_min)
        .max(pane_min)
        .clamp(0, width.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pane_takes_the_geometrys_width_and_the_list_the_rest() {
        assert_eq!(split(1280, 400, 300), 820);
        assert_eq!(split(1024, 400, 300), 620);
    }

    #[test]
    fn a_pane_never_gets_less_than_its_content_needs() {
        assert_eq!(split(1024, 700, 300), 700);
        // Past the room for one, until it gives way, it keeps its floor.
        assert_eq!(split(960, 400, 300), focus_dialog::PANE_MIN);
    }

    #[test]
    fn a_split_narrower_than_the_pane_gives_it_what_there_is() {
        assert_eq!(split(500, 600, 0), 500);
    }
}
