//! The list pane's three named states: inbox zero, offline, sync failure.
//!
//! Canvas 3d: each one "names the local store and gives a key, not a
//! shrug." [`derive()`] decides which state applies, and is a pure function
//! tested with no display, the same split [`crate::cheatsheet`] uses.
//! [`ListStateView`] is the widget around it.
//! The machine itself now lives in [`postio_ui::list_state`].
//!
//! # Where it lives, and how much of it it covers
//!
//! It is an overlay over [`crate::list_view::MessageListView`], and hides
//! itself the moment [`derive()`] returns `None` — there are rows to show and
//! nothing needs saying about them.
//!
//! The rest of the time, [`State::placement`] decides how much of the pane it
//! takes. [`State::InboxZero`] is, by definition, an empty mailbox — there is
//! nothing underneath to protect, so it is the opaque plate this widget
//! started as. [`State::Offline`] and [`State::Failing`] are not: the whole
//! point of "everything already synced still opens" is a promise about rows
//! that are, in fact, still there. Covering them to say so would keep the
//! promise in words and break it on screen — `postio-ma4` was exactly that
//! bug, caught only once mailboxes actually had rows in them. So with any
//! rows loaded, both become a [`Placement::Banner`] instead: a strip over the
//! top of the list, rows still visible and scrollable underneath. Only an
//! empty mailbox — offline or failing with nothing loaded at all — still
//! takes the [`Placement::Full`] plate, because there is, once again, nothing
//! under it to hide.
//!
//! # What is not wired yet
//!
//! Same shape as [`crate::sidebar`]'s own gap: [`ListStateView::set_status`]
//! is the whole input surface, and nothing calls it with live data yet.
//! [`postio_core::ConnectionState::Failing`] carries a typed category, not prose —
//! see its doc comment — so the reason has to arrive through
//! [`SyncStatus::detail`], the same field the sidebar's status line already
//! reads. The store and queue counts are plain `u64`s a caller supplies,
//! because the repository accessors this bead would need
//! (`postio-storage`'s operation queue has no cheap count yet) do not exist
//! on this side of the crate boundary.

use std::time::Instant;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use postio_core::Keymap;

use crate::sidebar::SyncStatus;

/// The list pane's state machine, derived rather than stored.
// Moved to `postio-ui` so the macOS list pane derives the same six states,
// with the same precedence between them, rather than deciding either for
// itself. The names are re-exported so nothing in this crate had to change,
// and so every comment that names `derive` still reads.
pub use postio_ui::list_state::{
    Content, OPENING_THRESHOLD, Offer, Placement, State, Waiting, derive, derive_aggregate,
    derive_opening, describe, describe_wait, is_current, resolve,
};

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct ListStateView {
        pub icon: gtk::Image,
        pub title: gtk::Label,
        pub detail: gtk::Label,
        pub hints: gtk::Box,
        /// The keymap the hints are read from; the registry's own until
        /// the window hands over the one in force.
        pub keymap: RefCell<Keymap>,
        pub inputs: RefCell<(SyncStatus, u64, u64, u64, Option<String>)>,
        /// The folder in view, by the name the sidebar shows, when it is
        /// not the inbox -- what an empty plate is titled with (#1535). Its
        /// own cell for the reason `accounts` has one: it arrives from the
        /// sidebar's pick, not from the status feed.
        pub place: RefCell<Option<String>>,
        /// The accounts an aggregate view is drawing, when it is one.
        ///
        /// `None` is an ordinary single-account view, which is what every
        /// scope but the unified list is. Its own cell rather than a sixth
        /// slot in `inputs` for the reason `set_searching` has its own: it
        /// arrives from the sidebar's account list on a completely different
        /// occasion from the sync feed's status.
        pub accounts: RefCell<Option<Vec<(String, SyncStatus)>>>,
        /// What the store is still doing, and since when (#1114).
        ///
        /// `None` once there is a store — and on every window nothing has
        /// told otherwise, which is what keeps this out of the way of every
        /// pane built for a test of one widget.
        pub opening: RefCell<Option<(Waiting, Instant)>>,
        pub tick: RefCell<Option<glib::SourceId>>,
        /// The one-shot that brings the opening plate up at the threshold.
        ///
        /// Its own timer rather than a second job for `tick`: that one is
        /// re-armed by every render from the sync status's own cadence, and
        /// this one has to fire exactly once, a fixed interval after the
        /// wait began.
        pub opening_tick: RefCell<Option<glib::SourceId>>,
    }

    impl Default for ListStateView {
        fn default() -> Self {
            Self {
                icon: gtk::Image::new(),
                title: gtk::Label::new(None),
                detail: gtk::Label::new(None),
                hints: gtk::Box::new(gtk::Orientation::Horizontal, 16),
                keymap: RefCell::new(Keymap::defaults().clone()),
                inputs: RefCell::new((SyncStatus::default(), 0, 0, 0, None)),
                place: RefCell::new(None),
                accounts: RefCell::new(None),
                opening: RefCell::new(None),
                tick: RefCell::new(None),
                opening_tick: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ListStateView {
        const NAME: &'static str = "PostioListStateView";
        type Type = super::ListStateView;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for ListStateView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(tick) = self.tick.borrow_mut().take() {
                tick.remove();
            }
            if let Some(tick) = self.opening_tick.borrow_mut().take() {
                tick.remove();
            }
        }
    }

    impl WidgetImpl for ListStateView {}
    impl BinImpl for ListStateView {}
}

glib::wrapper! {
    /// The list pane's placeholder for its three named states (canvas 3d).
    pub struct ListStateView(ObjectSubclass<imp::ListStateView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ListStateView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ListStateView {
    /// A view with nothing to show yet — offline, never synced, the same
    /// honest default [`crate::sidebar::Sidebar`] renders before it is fed.
    pub fn new() -> Self {
        Self::default()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-liststate");
        self.set_halign(gtk::Align::Fill);

        imp.icon.add_css_class("postio-liststate-icon");
        imp.title.add_css_class("postio-liststate-title");
        imp.title.set_wrap(true);
        imp.detail.add_css_class("postio-liststate-detail");
        imp.detail.set_wrap(true);

        // A live region: the sync engine can flip this from "empty" to
        // "failing" with nobody having touched anything, and that has to be
        // announced without stealing focus. The children are decorative —
        // this widget's own label carries the one sentence a screen reader
        // should say.
        self.set_accessible_role(gtk::AccessibleRole::Status);
        imp.icon
            .set_accessible_role(gtk::AccessibleRole::Presentation);
        imp.title
            .set_accessible_role(gtk::AccessibleRole::Presentation);
        imp.detail
            .set_accessible_role(gtk::AccessibleRole::Presentation);

        self.render();
    }

    /// Name the keys the keymap in force binds, redrawing the state on
    /// screen if there is one.
    pub fn set_keymap(&self, keymap: &Keymap) {
        *self.imp().keymap.borrow_mut() = keymap.clone();
        self.render();
    }

    /// What the list pane currently knows: the connection, how many rows are
    /// loaded for the mailbox in view, how many messages the local store
    /// still holds, and how many local writes have not reached the server.
    ///
    /// Call it whenever any of those change. The widget hides itself once
    /// there is nothing left to say — see [`State::placement`] for when
    /// having rows to show stops meaning that.
    pub fn set_status(&self, status: SyncStatus, item_count: u64, stored: u64, queued: u64) {
        let searching = self.imp().inputs.borrow().4.clone();
        let inputs = (status, item_count, stored, queued, searching);
        // Cheap to call and cheap to call often: the row count moves with
        // every page the message list takes delivery of, and re-rendering
        // an unchanged state would also re-arm the age timer each time.
        if *self.imp().inputs.borrow() == inputs {
            return;
        }
        *self.imp().inputs.borrow_mut() = inputs;
        self.render();
    }

    /// Say that the list is an aggregate over `accounts`, or a single
    /// account's view again.
    ///
    /// `None` restores the single-account states. `Some` switches the pane to
    /// ADR 0005 Q10's rule — see [`derive_aggregate`] — and the list must
    /// hold only the accounts it is actually drawing, in the sidebar's order.
    pub fn set_accounts(&self, accounts: Option<Vec<(String, SyncStatus)>>) {
        if *self.imp().accounts.borrow() == accounts {
            return;
        }
        *self.imp().accounts.borrow_mut() = accounts;
        self.render();
    }

    /// Say that the list is showing results for `query`, or a mailbox again.
    ///
    /// Its own setter rather than a fifth argument to
    /// [`set_status`](Self::set_status): the status arrives from the sync
    /// feed and the query from the search, they change on completely
    /// different occasions, and a combined call would make each of them
    /// carry a value it has no business knowing.
    pub fn set_searching(&self, query: Option<String>) {
        if self.imp().inputs.borrow().4 == query {
            return;
        }
        self.imp().inputs.borrow_mut().4 = query;
        self.render();
    }

    /// Which folder the empty plate is about: `None` for the inbox, the
    /// sidebar's name for any other folder. Call it when the folder in view
    /// changes; the plate re-titles itself.
    pub fn set_place(&self, mailbox: Option<String>) {
        if *self.imp().place.borrow() == mailbox {
            return;
        }
        *self.imp().place.borrow_mut() = mailbox;
        self.render();
    }

    /// Say that there is no store behind this window yet, and what it is
    /// waiting on — or that there is one now (#1114).
    ///
    /// Its own setter for [`set_searching`](Self::set_searching)'s reason,
    /// and a stronger version of it: this does not arrive from the sync feed
    /// at all, because there is no sync feed until the thing it is waiting
    /// for has finished.
    ///
    /// Nothing appears when this is set. The plate comes up one
    /// [`OPENING_THRESHOLD`] later, on the timer armed here, and only if the
    /// wait is still going — which is what makes an ordinary start draw
    /// nothing that is then removed.
    pub fn set_opening(&self, waiting: Option<Waiting>) {
        let imp = self.imp();
        if let Some(tick) = imp.opening_tick.borrow_mut().take() {
            tick.remove();
        }
        let previous = imp.opening.borrow().map(|(waiting, _)| waiting);
        if previous == waiting {
            // The same wait, still going: re-arming would push the plate
            // back by a threshold every time a caller repeated itself, which
            // is how a plate that should appear never does.
            if waiting.is_some() {
                self.arm_opening_tick();
            }
            return;
        }
        // A *different* wait restarts the clock, and deliberately: reaching
        // the migrations means the store opened, so the reader has not been
        // looking at an unexplained window for a second yet. What it must
        // not do is leave the old sentence up while the new wait runs.
        *imp.opening.borrow_mut() = waiting.map(|waiting| (waiting, Instant::now()));
        if waiting.is_some() {
            self.arm_opening_tick();
        }
        self.render();
    }

    fn arm_opening_tick(&self) {
        let source = glib::timeout_add_local_once(
            OPENING_THRESHOLD,
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move || {
                    view.imp().opening_tick.borrow_mut().take();
                    view.render();
                }
            ),
        );
        *self.imp().opening_tick.borrow_mut() = Some(source);
    }

    /// What this pane is waiting for, if it is waiting for anything.
    ///
    /// Answers even below the threshold, when nothing is drawn: the wait is a
    /// fact about the window, and [`state`](Self::state) is only what the
    /// pane is currently *saying* about it. The keyboard's refusal reads this
    /// one, so that a key pressed at 200 ms and a plate shown at 1 s give the
    /// same sentence.
    pub fn waiting(&self) -> Option<Waiting> {
        self.imp().opening.borrow().map(|(waiting, _)| waiting)
    }

    /// Pretend the current wait started `by` earlier.
    ///
    /// The test seam for [`OPENING_THRESHOLD`], and the reason there is one:
    /// a case that waits out a real second either costs a second or races a
    /// loaded runner, and the alternative — making the threshold a tunable —
    /// would put a number that is a product decision behind an environment
    /// variable. Nothing in the application calls this.
    pub fn wind_back(&self, by: std::time::Duration) {
        {
            let imp = self.imp();
            let mut opening = imp.opening.borrow_mut();
            let Some((_, since)) = opening.as_mut() else {
                return;
            };
            *since = since.checked_sub(by).unwrap_or(*since);
        }
        self.render();
    }

    /// The state currently on screen, if any.
    pub fn state(&self) -> Option<State> {
        self.derived()
    }

    /// The one place the choice between the single-account states and the
    /// aggregate ones is made.
    ///
    /// Shared by [`state`](Self::state) and [`render`](Self::render) because
    /// they answered separately once and disagreed: `render` learned about
    /// aggregate views and `state` did not, so the pane drew the right thing
    /// and every reader of the accessor -- the tests, and the screen reader
    /// label that follows them -- was told the old answer. A widget whose
    /// picture and whose description of itself come from different code is a
    /// widget that can be wrong in exactly the way nothing catches.
    fn derived(&self) -> Option<State> {
        let imp = self.imp();
        // Before everything, and answering `None` below the threshold rather
        // than falling through: with no store there is no connection worth
        // describing, no mailbox to be empty and no query to have matched
        // nothing. A window that said "Offline — reading local mail" here
        // would be describing mail it has not opened.
        if let Some((waiting, since)) = *imp.opening.borrow() {
            return derive_opening(waiting, since.elapsed());
        }
        let (status, item_count, stored, queued, searching) = imp.inputs.borrow().clone();
        let aggregate = imp.accounts.borrow().clone();
        let place = imp.place.borrow().clone();
        match &aggregate {
            Some(accounts) => derive_aggregate(
                accounts,
                item_count,
                stored,
                searching.as_deref(),
                place.as_deref(),
            ),
            None => derive(
                &status,
                item_count,
                stored,
                queued,
                searching.as_deref(),
                place.as_deref(),
            ),
        }
    }

    fn render(&self) {
        let imp = self.imp();
        let now = Instant::now();
        let (status, item_count) = {
            let inputs = imp.inputs.borrow();
            (inputs.0.clone(), inputs.1)
        };
        let state = self.derived();

        self.set_visible(state.is_some());
        if let Some(state) = &state {
            let content = describe(state, now);

            imp.icon.set_icon_name(Some(content.icon));
            for class in ["inbox-zero", "offline", "failing", "no-matches", "opening"] {
                imp.icon.remove_css_class(class);
            }
            imp.icon.add_css_class(content.icon_class);

            imp.title.set_text(&content.title);
            imp.detail.set_text(&content.detail);

            let hints = resolve(&content.hints, &imp.keymap.borrow());
            let spoken = hints
                .iter()
                .map(|hint| format!("{}, press {}", hint.label, hint.key))
                .collect::<Vec<_>>()
                .join(". ");
            self.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}. {}. {spoken}",
                content.title, content.detail
            ))]);

            while let Some(child) = imp.hints.first_child() {
                imp.hints.remove(&child);
            }
            for hint in &hints {
                imp.hints.append(&crate::widgets::keyhint::chip(
                    hint,
                    "postio-liststate-hint",
                ));
            }

            let placement = state.placement(item_count);
            if placement == Placement::Banner {
                self.add_css_class("postio-liststate-banner");
            } else {
                self.remove_css_class("postio-liststate-banner");
            }
            self.set_valign(match placement {
                Placement::Full => gtk::Align::Fill,
                Placement::Banner => gtk::Align::Start,
            });
            self.set_vexpand(placement == Placement::Full);

            // The three decorative widgets move between the two layouts
            // rather than existing twice — `unparent` first since a widget
            // already inside last render's container cannot simply be
            // `append`ed into a new one.
            imp.icon.unparent();
            imp.title.unparent();
            imp.detail.unparent();
            imp.hints.unparent();
            let container = match placement {
                Placement::Full => full_container(&imp.icon, &imp.title, &imp.detail, &imp.hints),
                Placement::Banner => {
                    banner_container(&imp.icon, &imp.title, &imp.detail, &imp.hints)
                }
            };
            self.set_child(Some(&container));
        }

        // Re-arm at the granularity the inbox-zero sentence is actually
        // showing, so an age in days does not wake the process up every
        // second — the same reasoning as `Sidebar::render_status`.
        if let Some(tick) = imp.tick.borrow_mut().take() {
            tick.remove();
        }
        if let Some(interval) = status.refresh_interval(now) {
            let source = glib::timeout_add_local(
                interval,
                glib::clone!(
                    #[weak(rename_to = view)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        view.render();
                        glib::ControlFlow::Break
                    }
                ),
            );
            *imp.tick.borrow_mut() = Some(source);
        }
    }
}

/// The opaque plate: a centred column, filling the pane. What this widget
/// always looked like, before there was a rows-still-loaded case to protect.
fn full_container(
    icon: &gtk::Image,
    title: &gtk::Label,
    detail: &gtk::Label,
    hints: &gtk::Box,
) -> gtk::Box {
    icon.set_pixel_size(30);
    title.set_justify(gtk::Justification::Center);
    detail.set_justify(gtk::Justification::Center);
    detail.set_max_width_chars(36);
    hints.set_halign(gtk::Align::Center);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Center);
    column.set_vexpand(true);
    column.set_margin_start(32);
    column.set_margin_end(32);
    column.append(icon);
    column.append(title);
    column.append(detail);
    column.append(hints);
    column
}

/// The banner: a strip along the top edge, rows still visible and scrollable
/// underneath it.
fn banner_container(
    icon: &gtk::Image,
    title: &gtk::Label,
    detail: &gtk::Label,
    hints: &gtk::Box,
) -> gtk::Box {
    icon.set_pixel_size(20);
    title.set_justify(gtk::Justification::Left);
    detail.set_justify(gtk::Justification::Left);
    detail.set_max_width_chars(-1);
    hints.set_halign(gtk::Align::End);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(title);
    text.append(detail);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("postio-liststate-banner-row");
    row.set_valign(gtk::Align::Center);
    row.set_margin_start(16);
    row.set_margin_end(16);
    row.set_margin_top(10);
    row.set_margin_bottom(10);
    row.append(icon);
    row.append(&text);
    row.append(hints);
    row
}
