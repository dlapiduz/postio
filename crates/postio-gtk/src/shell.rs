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

/// Who has the reading pane.
///
/// Three surfaces live in the pane [`Shell::reader`] hands out — the reader,
/// the search preview and the composer — and #502 is what happens when each
/// toggles its own visibility from its own snapshot: a message drawn twice
/// (reader above, a preview that never left below), and a cleared preview
/// hanging under an inbox message after search was dismissed. The pane is a
/// mode surface, so it gets what every mode here gets: one owner, and a
/// current occupant computed from what is active rather than replayed from
/// what somebody remembered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderOccupant {
    /// Nothing to show; the pane is its empty self.
    #[default]
    Empty,
    /// The hardened reading view, on an open message.
    Reader,
    /// The search preview — canvas 2b's right-hand pane.
    SearchPreview,
    /// The composer, which takes the pane over (docs/PRODUCT.md).
    Composer,
}

/// Whether the sidebar is drawn, given what the user asked for and what the
/// window can afford.
///
/// The two are different facts and only one of them is the user's (ADR 0024).
/// `wanted` is a standing preference that survives a resize and a restart;
/// the mode is a property of the window this instant. Deriving visibility
/// from both, every time, is what stops a narrow window's answer being
/// mistaken for the user's — which is what happened when one boolean held
/// both and `save_state` persisted whichever it was holding (#825).
///
/// Pure, so the whole rule is testable without a window.
pub fn sidebar_shown(wanted: bool, mode: Mode) -> bool {
    wanted && mode == Mode::ThreePane
}

/// The standing preference after the user toggles the sidebar to `on`.
///
/// A toggle in `ThreePane` is a preference: the user is looking at a window
/// wide enough to hold the sidebar and has said whether they want it.
///
/// A toggle in a narrower mode is an *override* — `shell.rs`'s own promise
/// that "the sidebar is still reachable in the narrower modes" — and must not
/// be recorded, or reaching for it once on a small window would be read as
/// wanting it for ever afterwards on a large one.
///
/// Pure, for the same reason as [`sidebar_shown`].
pub fn sidebar_wanted_after_toggle(on: bool, mode: Mode, wanted: bool) -> bool {
    if mode == Mode::ThreePane { on } else { wanted }
}

/// The occupant the active surfaces call for, by rank.
///
/// The composer outranks everything: a half-written draft on a hidden widget
/// is the worst state the pane has. Search outranks plain reading while it is
/// up, because the pane is the search's answer column. Pure, so the whole
/// priority is testable without a window.
pub fn fallback(composing: bool, searching: bool, reading: bool) -> ReaderOccupant {
    if composing {
        ReaderOccupant::Composer
    } else if searching {
        ReaderOccupant::SearchPreview
    } else if reading {
        ReaderOccupant::Reader
    } else {
        ReaderOccupant::Empty
    }
}

mod imp {
    use std::cell::Cell;
    use std::cell::RefCell;

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
        /// Whether the sidebar is drawn right now — derived, never stored as
        /// an answer to "does the user want it". See [`sidebar_wanted`].
        #[property(get, set = Self::set_sidebar_visible, name = "sidebar-visible")]
        pub sidebar_visible: Cell<bool>,
        /// Whether the user wants the sidebar, independent of whether this
        /// window is currently wide enough to hold it (ADR 0024).
        ///
        /// Written only by a toggle at full width, and by `restore` putting
        /// back what was saved. A breakpoint never touches it, which is the
        /// whole point: the mode is a fact about the window, not about the
        /// person using it.
        pub sidebar_wanted: Cell<bool>,
        /// The reading pane's occupants, registered as each is built. Weak,
        /// because the composer can leave for a detached window and the
        /// arbiter must not keep a widget alive that its owner let go.
        pub occupants: RefCell<Vec<(ReaderOccupant, glib::WeakRef<gtk::Widget>)>>,
        /// Who has the reading pane right now.
        pub occupant: Cell<ReaderOccupant>,
        /// The activity flags `fallback` is computed from.
        pub composing: Cell<bool>,
        pub searching: Cell<bool>,
        pub reading: Cell<bool>,
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
                sidebar_wanted: Cell::new(true),
                focused: Cell::new(Pane::default()),
                occupants: RefCell::new(Vec::new()),
                occupant: Cell::new(ReaderOccupant::default()),
                composing: Cell::new(false),
                searching: Cell::new(false),
                reading: Cell::new(false),
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
            // The property is the effective state; the preference behind it
            // moves only when the window is wide enough for the answer to
            // mean anything (ADR 0024).
            self.sidebar_wanted.set(super::sidebar_wanted_after_toggle(
                visible,
                self.mode.get(),
                self.sidebar_wanted.get(),
            ));
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

    // -- One owner for the reading pane (#502) ------------------------------

    /// Registers `widget` as the reading pane's `occupant` surface.
    ///
    /// Called once by each surface as it mounts itself into [`reader`]. The
    /// arbiter hides it now and shows it only while it is the current
    /// occupant; the surface must never toggle its own pane visibility
    /// again — three surfaces doing exactly that, each from its own
    /// snapshot, is the bug this exists to end.
    ///
    /// Registering a second, still-parented widget for a kind that already
    /// has one is always a bug, never a legitimate re-registration — each of
    /// the three real callers (`Reader`, `SearchPreview`, `Composer`) mounts
    /// exactly once per window's life, detaching and reattaching the same
    /// widget rather than building a second one. #831 is what the silent
    /// version of this looks like: a second `search::View::attach` on one
    /// shell left the first preview parented and visible forever, since
    /// nothing removed it and the tracking that drives visibility had
    /// already moved on to the second. Panicking here turns that into a
    /// crash at the mistake, not a screenshot two owners drew together.
    ///
    /// [`reader`]: Self::reader
    pub fn register_reader_occupant(&self, occupant: ReaderOccupant, widget: &gtk::Widget) {
        let mut occupants = self.imp().occupants.borrow_mut();
        if let Some((_, previous)) = occupants.iter().find(|(kind, _)| *kind == occupant)
            && let Some(previous) = previous.upgrade()
            && previous.parent().as_ref() == Some(&self.imp().reader.clone().upcast())
        {
            panic!(
                "a second {occupant:?} was registered for the reading pane while \
                 the first is still attached — one owner per occupant kind, see \
                 register_reader_occupant's doc comment (#831)"
            );
        }
        let weak = glib::WeakRef::new();
        weak.set(Some(widget));
        occupants.retain(|(existing, _)| *existing != occupant);
        occupants.push((occupant, weak));
        drop(occupants);
        widget.set_visible(self.imp().occupant.get() == occupant);
    }

    /// Who has the reading pane right now.
    pub fn reader_occupant(&self) -> ReaderOccupant {
        self.imp().occupant.get()
    }

    /// The pane is (or is no longer) open on a message.
    ///
    /// A *flag*, not a claim: this may be re-stated by anything syncing the
    /// window's state — the composer closing re-syncs it, for one — and a
    /// re-statement must not steal the pane from the search preview. Only
    /// [`claim_reading`] shows the reader; what this does is keep the flag
    /// current, and settle the pane when the message goes away.
    ///
    /// While a higher-ranked surface holds the pane the flag still updates —
    /// what shows when that surface leaves is computed from the flags at
    /// that moment, never replayed from a snapshot.
    ///
    /// [`claim_reading`]: Self::claim_reading
    pub fn set_reading(&self, reading: bool) {
        self.imp().reading.set(reading);
        if self.imp().composing.get() {
            return;
        }
        if !reading && self.imp().occupant.get() == ReaderOccupant::Reader {
            self.settle_reader_pane();
        }
    }

    /// A message was just opened: the reader takes the pane.
    ///
    /// The deliberate gesture, distinct from the flag: `Enter` on a search
    /// result opens it in the real reader over the preview, and that is this
    /// call. The composer still outranks it — a half-written draft is not
    /// lost to a cursor movement.
    pub fn claim_reading(&self) {
        self.imp().reading.set(true);
        if !self.imp().composing.get() {
            self.show_occupant(ReaderOccupant::Reader);
        }
    }

    /// Search took (or left) the pane's column.
    pub fn set_searching(&self, searching: bool) {
        self.imp().searching.set(searching);
        if self.imp().composing.get() {
            return;
        }
        if searching {
            self.show_occupant(ReaderOccupant::SearchPreview);
        } else if self.imp().occupant.get() == ReaderOccupant::SearchPreview {
            self.settle_reader_pane();
        }
    }

    /// The focus moved through the search results.
    ///
    /// Browsing means previewing: if `Enter` had put the real reader up, the
    /// next arrow puts the preview back. A no-op outside search or while the
    /// composer holds the pane.
    pub fn preview_focused(&self) {
        if self.imp().searching.get() && !self.imp().composing.get() {
            self.show_occupant(ReaderOccupant::SearchPreview);
        }
    }

    /// The composer took or released the pane. It outranks everything: see
    /// [`fallback`].
    pub fn set_composing(&self, composing: bool) {
        self.imp().composing.set(composing);
        if composing {
            self.show_occupant(ReaderOccupant::Composer);
        } else {
            self.settle_reader_pane();
        }
    }

    /// Shows what the flags call for — after the current occupant left.
    fn settle_reader_pane(&self) {
        let imp = self.imp();
        self.show_occupant(fallback(
            imp.composing.get(),
            imp.searching.get(),
            imp.reading.get(),
        ));
    }

    /// Makes `occupant` the one visible surface in the pane.
    ///
    /// Only widgets currently *in* the pane are touched: the composer can be
    /// away in a detached window, and hiding it there — because it is not
    /// the pane's occupant — would blank the window it took the draft to.
    fn show_occupant(&self, occupant: ReaderOccupant) {
        self.imp().occupant.set(occupant);
        let pane: gtk::Widget = self.imp().reader.clone().upcast();
        for (registered, widget) in self.imp().occupants.borrow().iter() {
            if let Some(widget) = widget.upgrade()
                && widget.parent().as_ref() == Some(&pane)
            {
                widget.set_visible(*registered == occupant);
            }
        }
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
        // Entering a narrower mode drops the sidebar; leaving it brings back
        // whatever the user actually asked for, which may be nothing. Derived
        // rather than assigned: writing the mode's answer into the preference
        // is what made a narrow window's constraint outlive it, all the way
        // into `save_state` and the next launch (ADR 0024, #825).
        //
        // Set on the imp directly, not through `set_sidebar_visible`: that one
        // is the *user's* entry point and records a preference, which is
        // exactly what a breakpoint must not do.
        let imp = self.imp();
        imp.sidebar_visible
            .set(sidebar_shown(imp.sidebar_wanted.get(), mode));
        self.apply();
    }

    /// Whether the user wants the sidebar, whatever this window can currently
    /// afford — the value worth saving across a restart (ADR 0024).
    ///
    /// [`sidebar_visible`](Self::sidebar_visible) is the effective state and
    /// is the wrong thing to persist: on a narrow window it is the
    /// breakpoint's answer, not the user's.
    pub fn sidebar_wanted(&self) -> bool {
        self.imp().sidebar_wanted.get()
    }

    /// Put back a saved preference, without treating it as a toggle.
    ///
    /// Used by `Window::restore` before the breakpoints have had a chance to
    /// fire. The effective state is derived from it and the current mode, so
    /// restoring "wanted" on a window that opens narrow shows no sidebar and
    /// still remembers the answer.
    pub fn set_sidebar_wanted(&self, wanted: bool) {
        let imp = self.imp();
        imp.sidebar_wanted.set(wanted);
        imp.sidebar_visible
            .set(sidebar_shown(wanted, imp.mode.get()));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole priority, in one place: the composer outranks search,
    /// search outranks reading, and nothing active is an empty pane. #502's
    /// two screenshots are both rows of this table — the states existed
    /// simultaneously because three owners each answered a different row.
    // -- intent and constraint are different facts (ADR 0024, #825) --------

    #[test]
    fn a_narrow_window_hides_the_sidebar_without_forgetting_it() {
        // The bug this is about: one boolean held "what the viewport affords"
        // and "what the user asked for", and `save_state` persisted whichever
        // it was holding. Narrow the window, quit, reopen wide -- no sidebar,
        // no explanation, because the breakpoint's answer had been recorded as
        // the preference.
        assert!(sidebar_shown(true, Mode::ThreePane));
        assert!(!sidebar_shown(true, Mode::TwoPane));
        assert!(!sidebar_shown(true, Mode::MessageFocused));

        // And the preference itself is untouched by any of that: the mode
        // never writes it.
        for mode in [Mode::ThreePane, Mode::TwoPane, Mode::MessageFocused] {
            assert!(
                sidebar_wanted_after_toggle(true, mode, true),
                "{mode:?} rewrote a preference it does not own"
            );
        }
    }

    #[test]
    fn a_user_who_does_not_want_the_sidebar_does_not_get_it_back_on_widening() {
        // The other direction, and the reason this is a derivation rather
        // than "put it back when the window grows": widening restores what the
        // user asked for, which may be nothing.
        assert!(!sidebar_shown(false, Mode::ThreePane));
    }

    #[test]
    fn reaching_for_the_sidebar_on_a_narrow_window_is_not_a_preference() {
        // `shell.rs` promises the sidebar stays reachable in the narrower
        // modes. Recording that reach would turn one look at the folder list
        // on a small window into a standing answer on every large one.
        assert!(!sidebar_wanted_after_toggle(true, Mode::TwoPane, false));
        assert!(!sidebar_wanted_after_toggle(
            true,
            Mode::MessageFocused,
            false
        ));

        // A toggle at full width is exactly what a preference is.
        assert!(sidebar_wanted_after_toggle(true, Mode::ThreePane, false));
        assert!(!sidebar_wanted_after_toggle(false, Mode::ThreePane, true));
    }

    #[test]
    fn the_fallback_ranks_composer_over_search_over_reading() {
        use ReaderOccupant::*;
        assert_eq!(fallback(true, true, true), Composer);
        assert_eq!(fallback(true, false, false), Composer);
        assert_eq!(fallback(false, true, true), SearchPreview);
        assert_eq!(fallback(false, true, false), SearchPreview);
        assert_eq!(fallback(false, false, true), Reader);
        assert_eq!(fallback(false, false, false), Empty);
    }
}
