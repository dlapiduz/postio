//! The PLATE three-pane layout: sidebar, message list, reader.
//!
//! # Why `GtkPaned` and not `AdwNavigationSplitView`
//!
//! The split views are the usual GNOME answer, and they are the wrong one
//! here for two reasons. They have no user-draggable divider — the sidebar is
//! a fraction of the window, not something you set — and collapsing one is
//! animated. The motion budget in CLAUDE.md is explicit that **pane switches
//! use no transition**, and a pane that slides is a pane you are waiting for.
//!
//! `GtkPaned` gives a real handle, a `position` worth saving, and a layout
//! that changes between one frame and the next. The adaptive behaviour the
//! split views would have given us is a handful of `AdwBreakpoint`s setting
//! [`Shell::set_mode`], which is a smaller thing than it sounds: every mode is
//! the same three widgets with different ones shown.
//!
//! # The modes
//!
//! docs/PRODUCT.md §9, at the widths canvas 1b's proportions actually need:
//!
//! | Mode | Window | Shows |
//! |---|---|---|
//! | [`Mode::ThreePane`] | ≥ 1040px | sidebar, list, reader |
//! | [`Mode::TwoPane`] | ≥ 720px | list, reader |
//! | [`Mode::MessageFocused`] | < 720px | list *or* reader |
//!
//! The sidebar is still reachable in the narrower modes — the header's toggle
//! shows it — so "collapsed" means "not by default", never "not available".

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

/// The sidebar's width, from canvas 1b.
pub const SIDEBAR_WIDTH: i32 = 212;

/// The message list's width, from canvas 1b.
pub const LIST_WIDTH: i32 = 404;

/// Below this the sidebar is not shown by default.
pub const TWO_PANE_WIDTH: i32 = 1040;

/// Below this only one of the list and the reader is shown.
pub const MESSAGE_FOCUSED_WIDTH: i32 = 720;

/// How many panes are on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Sidebar, list and reader — what the canvas draws.
    #[default]
    ThreePane,
    /// List and reader; the sidebar is a toggle away.
    TwoPane,
    /// One pane, chosen by [`Shell::focused_pane`].
    MessageFocused,
}

/// Which pane has the screen in [`Mode::MessageFocused`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    /// The message list.
    #[default]
    List,
    /// The reading pane.
    Reader,
}

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::Shell)]
    pub struct Shell {
        /// sidebar | (list | reader)
        pub outer: gtk::Paned,
        /// list | reader
        pub inner: gtk::Paned,
        pub sidebar: gtk::Box,
        pub list: gtk::Box,
        pub reader: gtk::Box,
        pub mode: Cell<Mode>,
        pub focused: Cell<Pane>,
        /// Whether the sidebar is showing.
        ///
        /// A property rather than a plain field because two things move it —
        /// the header's toggle and the breakpoints — and each has to see the
        /// other's change. `notify` is how the button learns that widening
        /// the window brought the sidebar back.
        #[property(get, set = Self::set_sidebar_visible, name = "sidebar-visible")]
        pub sidebar_visible: Cell<bool>,
    }

    impl Default for Shell {
        fn default() -> Self {
            let pane = |class: &str| {
                let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
                pane.add_css_class(class);
                pane
            };
            let reader = pane("postio-reader");
            // The reader is the inner `Paned`'s *end* child, which `build`
            // already tells to grow (`set_resize_end_child(true)`) -- but
            // that only grows this box's own allocation. Nothing told the
            // box itself to fill that allocation rather than sit at its
            // natural width, so maximizing the window left it cropped
            // (#428). `sidebar` and `list` sit against the fixed *start*
            // side and must not do this.
            reader.set_hexpand(true);
            Shell {
                outer: gtk::Paned::new(gtk::Orientation::Horizontal),
                inner: gtk::Paned::new(gtk::Orientation::Horizontal),
                sidebar: pane("postio-sidebar"),
                list: pane("postio-list"),
                reader,
                mode: Cell::new(Mode::default()),
                sidebar_visible: Cell::new(true),
                focused: Cell::new(Pane::default()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Shell {
        const NAME: &'static str = "PostioShell";
        type Type = super::Shell;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Shell {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Shell {}
    impl BinImpl for Shell {}

    impl Shell {
        fn set_sidebar_visible(&self, visible: bool) {
            self.sidebar_visible.set(visible);
            self.obj().apply();
        }
    }
}

glib::wrapper! {
    /// The three panes and the rules about when each one is on screen.
    pub struct Shell(ObjectSubclass<imp::Shell>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Shell {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Shell {
    /// A shell at the canvas' proportions, in [`Mode::ThreePane`].
    pub fn new() -> Self {
        Self::default()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-shell");

        // The sidebar and the list keep their width when the window is
        // resized; the reader takes the slack. A mail client that steals the
        // list's width every time you widen the window is unusable.
        imp.outer.set_start_child(Some(&imp.sidebar));
        imp.outer.set_end_child(Some(&imp.inner));
        imp.outer.set_resize_start_child(false);
        imp.outer.set_shrink_start_child(false);
        imp.outer.set_resize_end_child(true);
        imp.outer.set_position(SIDEBAR_WIDTH);

        imp.inner.set_start_child(Some(&imp.list));
        imp.inner.set_end_child(Some(&imp.reader));
        imp.inner.set_resize_start_child(false);
        imp.inner.set_shrink_start_child(false);
        imp.inner.set_resize_end_child(true);
        imp.inner.set_position(LIST_WIDTH);

        imp.sidebar.set_size_request(160, -1);
        imp.list.set_size_request(280, -1);
        imp.reader.set_size_request(320, -1);

        // The paned handles are focusable so the panes can be resized from
        // the keyboard. GTK gives them no role of their own, so a name is
        // the only thing standing between a screen reader user and a tab
        // stop that announces nothing at all.
        imp.outer
            .update_property(&[gtk::accessible::Property::Label("Sidebar width")]);
        imp.inner
            .update_property(&[gtk::accessible::Property::Label("Message list width")]);

        // The panes are landmarks: a screen reader user navigates by them, and
        // so does the keyboard focus order the rest of the epic hangs off.
        imp.sidebar
            .set_accessible_role(gtk::AccessibleRole::Navigation);
        imp.list.set_accessible_role(gtk::AccessibleRole::List);
        imp.reader.set_accessible_role(gtk::AccessibleRole::Article);

        self.set_child(Some(&imp.outer));
        self.apply();
    }

    /// The container the folder list fills.
    pub fn sidebar(&self) -> gtk::Box {
        self.imp().sidebar.clone()
    }

    /// The container the message list fills.
    pub fn list(&self) -> gtk::Box {
        self.imp().list.clone()
    }

    /// The container the reader fills.
    pub fn reader(&self) -> gtk::Box {
        self.imp().reader.clone()
    }

    /// How many panes are currently on screen.
    pub fn mode(&self) -> Mode {
        self.imp().mode.get()
    }

    /// Switch modes.
    ///
    /// This takes effect immediately — before the call returns, and without a
    /// frame in between. It is the one thing the motion budget says about pane
    /// switches, so it is worth being able to point at.
    pub fn set_mode(&self, mode: Mode) {
        if self.imp().mode.replace(mode) == mode {
            return;
        }
        // Entering a narrower mode drops the sidebar; leaving it brings the
        // sidebar back, because the width that hid it is the width that is now
        // gone. A toggle afterwards still wins — the property is the last word.
        self.set_sidebar_visible(mode == Mode::ThreePane);
        self.apply();
    }

    /// Which pane has the screen in [`Mode::MessageFocused`].
    pub fn focused_pane(&self) -> Pane {
        self.imp().focused.get()
    }

    /// Choose the pane that has the screen when there is only room for one.
    ///
    /// Harmless in the wider modes: it is recorded, and takes effect if the
    /// window is ever narrowed.
    pub fn set_focused_pane(&self, pane: Pane) {
        if self.imp().focused.replace(pane) != pane {
            self.apply();
        }
    }

    /// The two divider positions, sidebar first.
    pub fn divider_positions(&self) -> (i32, i32) {
        let imp = self.imp();
        (imp.outer.position(), imp.inner.position())
    }

    /// Put the dividers back where they were left.
    pub fn set_divider_positions(&self, sidebar: i32, list: i32) {
        let imp = self.imp();
        imp.outer.set_position(sidebar);
        imp.inner.set_position(list);
    }

    /// Add the breakpoints that drive [`Mode`] to `window`.
    ///
    /// libadwaita applies the last matching breakpoint and unapplies the one
    /// before it, so each handler sets the whole mode rather than nudging it.
    pub fn install_breakpoints(&self, window: &impl IsA<adw::ApplicationWindow>) {
        let window = window.as_ref();
        for (max_width, mode) in [
            (TWO_PANE_WIDTH, Mode::TwoPane),
            (MESSAGE_FOCUSED_WIDTH, Mode::MessageFocused),
        ] {
            let condition = adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                (max_width - 1) as f64,
                adw::LengthUnit::Px,
            );
            let breakpoint = adw::Breakpoint::new(condition);
            breakpoint.connect_apply(glib::clone!(
                #[weak(rename_to = shell)]
                self,
                move |_| shell.set_mode(mode)
            ));
            breakpoint.connect_unapply(glib::clone!(
                #[weak(rename_to = shell)]
                self,
                move |_| shell.set_mode(Mode::ThreePane)
            ));
            window.add_breakpoint(breakpoint);
        }
    }

    /// Put the three panes' visibility in step with the mode.
    ///
    /// Everything is a `set_visible`: no reparenting, no revealer, no
    /// animation. Switching modes costs a relayout and nothing else.
    fn apply(&self) {
        let imp = self.imp();
        let mode = imp.mode.get();
        let one_pane = mode == Mode::MessageFocused;

        imp.sidebar.set_visible(imp.sidebar_visible.get());
        imp.list
            .set_visible(!one_pane || imp.focused.get() == Pane::List);
        imp.reader
            .set_visible(!one_pane || imp.focused.get() == Pane::Reader);
    }
}
