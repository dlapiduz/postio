//! The list pane's three named states: inbox zero, offline, sync failure.
//!
//! Canvas 3d: each one "names the local store and gives a key, not a
//! shrug." [`derive`] decides which state applies, and is a pure function
//! tested with no display, the same split [`crate::cheatsheet`] uses.
//! [`ListStateView`] is the widget around it.
//!
//! # Where it lives, and how much of it it covers
//!
//! It is an overlay over [`crate::list_view::MessageListView`], and hides
//! itself the moment [`derive`] returns `None` — there are rows to show and
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
//! [`ConnectionState::Failing`] carries no reason of its own — deliberately,
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

use postio_core::ConnectionState;

use crate::sidebar::{SyncStatus, age};

/// What the list pane shows in place of rows.
///
/// `None` from [`derive`] means there are rows to show and the widget should
/// stay out of the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing left to triage in the mailbox in view.
    InboxZero {
        /// When the last sync completed; `None` before the first one.
        last_sync: Option<Instant>,
        /// Messages still in the local store, elsewhere, and searchable.
        stored: u64,
    },
    /// No connection right now; local mail is still fully usable.
    Offline {
        /// Local writes waiting to reach the server.
        queued: u64,
    },
    /// The sync engine cannot reach the server.
    Failing {
        /// The actual error, phrased for the user. Never a shrug.
        reason: String,
    },
    /// A search matched nothing.
    ///
    /// Separate from [`InboxZero`](State::InboxZero) because the mailbox is
    /// not empty — the query is. Telling someone who searched for an invoice
    /// that they have nothing left to triage is a different statement, and a
    /// false one.
    NoMatches {
        /// What was searched for, shown back so what to widen is visible.
        query: String,
    },
}

/// How much of the pane a [`State`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Opaque, filling the pane. Correct only when there is nothing under it
    /// — an empty mailbox, whatever the reason it is empty.
    Full,
    /// A strip over the top of the rows, which stay visible and scrollable
    /// underneath.
    Banner,
}

impl State {
    /// Whether this state may share the pane with rows, or has to fill it.
    ///
    /// `item_count` is the same count [`derive`] was given: `InboxZero` never
    /// needs it, since being empty is what put it here in the first place,
    /// but `Offline` and `Failing` can arrive with a full mailbox loaded
    /// underneath, and that is exactly the mail the banner treatment exists
    /// to keep visible.
    pub fn placement(&self, item_count: u64) -> Placement {
        match self {
            State::InboxZero { .. } | State::NoMatches { .. } => Placement::Full,
            State::Offline { .. } | State::Failing { .. } => {
                if item_count == 0 {
                    Placement::Full
                } else {
                    Placement::Banner
                }
            }
        }
    }
}

/// Which state the list pane shows, from what it knows right now.
///
/// `item_count` is what the loaded page of the windowed model reports for
/// the mailbox in view; `stored` and `queued` come from the local store and
/// do not depend on the server being reachable at all — that is the whole
/// point of "everything already synced still opens."
///
/// `searching` is the query the list is showing results for, or `None` when
/// it is showing a mailbox. It is passed in rather than inferred from the
/// query box, because a box with text in it is not the same thing as a list
/// showing that text's results — the box stays up after `Esc` puts the
/// folder back.
///
/// [`ConnectionState::Connecting`] folds into [`State::Offline`]: from the
/// user's chair both mean "not connected right now, local mail still
/// works," and a fourth named state for a transition that resolves itself
/// would be a state nobody could tell apart from the one before it.
pub fn derive(
    status: &SyncStatus,
    item_count: u64,
    stored: u64,
    queued: u64,
    searching: Option<&str>,
) -> Option<State> {
    // A search answers for itself, ahead of the connection. The index is
    // local and it answered completely, so "Offline — reading local mail"
    // over an empty result set would be true and useless: the local mail is
    // exactly what was just searched. A search that *did* match still gets
    // the connection's banner over its rows, because that is a fact about
    // the rows rather than about the query.
    if let Some(query) = searching.filter(|_| item_count == 0) {
        return Some(State::NoMatches {
            query: query.to_string(),
        });
    }
    match status.state {
        ConnectionState::Failing => Some(State::Failing {
            reason: status
                .detail
                .clone()
                .unwrap_or_else(|| "the server did not say why".to_string()),
        }),
        ConnectionState::Offline | ConnectionState::Connecting => Some(State::Offline { queued }),
        ConnectionState::Online if item_count == 0 => Some(State::InboxZero {
            last_sync: status.last_sync,
            stored,
        }),
        ConnectionState::Online => None,
    }
}

/// One key hint: what it does, and the key that does it.
///
/// Every key named here is already a live [`postio_core::CommandId`] with
/// its own binding and palette entry — this widget only points at it, the
/// same way the focused row's key hints do, rather than growing a fourth
/// clickable-button idiom the app does not otherwise have.
type Hint = (&'static str, &'static str);

struct Content {
    icon: &'static str,
    icon_class: &'static str,
    title: &'static str,
    detail: String,
    hints: Vec<Hint>,
}

fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn describe(state: &State, now: Instant) -> Content {
    match state {
        State::InboxZero { last_sync, stored } => {
            let synced = match last_sync {
                Some(at) => format!(
                    "Last synced {} ago.",
                    age(now.saturating_duration_since(*at))
                ),
                None => "Never synced yet.".to_string(),
            };
            Content {
                icon: "emblem-ok-symbolic",
                icon_class: "inbox-zero",
                title: "Inbox is empty",
                detail: format!(
                    "Nothing left to triage. {} still in the local store and searchable. {synced}",
                    plural(*stored, "message")
                ),
                hints: vec![("Search all mail", "/"), ("Compose", "c")],
            }
        }
        State::Offline { queued } => Content {
            icon: "network-offline-symbolic",
            icon_class: "offline",
            title: "Offline — reading local mail",
            detail: if *queued == 0 {
                "Everything already synced still opens.".to_string()
            } else {
                format!(
                    "Everything already synced still opens. {} waiting to send when the link is back.",
                    plural(*queued, "change")
                )
            },
            hints: vec![("Retry now", "R")],
        },
        State::Failing { reason } => Content {
            icon: "dialog-error-symbolic",
            icon_class: "failing",
            title: "Sync failed",
            detail: format!("{reason} Local mail is untouched."),
            hints: vec![("Retry now", "R")],
        },
        // The query is echoed back rather than described, because what to
        // change is the thing the user cannot see from here: the box holds
        // chips, and the operators they stand for are what actually ran.
        //
        // Quoted, and that is not decoration. Unquoted it renders as
        // "Nothing in the local store matches from:ada invoice." -- prose
        // and query in one face with nothing between them, which wraps
        // mid-query and reads as a sentence. Quotes rather than a mono span,
        // because a query is user-typed and a Pango markup span would mean
        // escaping it; a label that renders `&` wrong is a worse bug than a
        // face that is not quite the token.
        State::NoMatches { query } => Content {
            icon: "system-search-symbolic",
            icon_class: "no-matches",
            title: "No matches",
            detail: format!("Nothing in the local store matches \u{201c}{query}\u{201d}."),
            hints: vec![("Back to the folder", "Esc")],
        },
    }
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct ListStateView {
        pub icon: gtk::Image,
        pub title: gtk::Label,
        pub detail: gtk::Label,
        pub hints: gtk::Box,
        pub inputs: RefCell<(SyncStatus, u64, u64, u64, Option<String>)>,
        pub tick: RefCell<Option<glib::SourceId>>,
    }

    impl Default for ListStateView {
        fn default() -> Self {
            Self {
                icon: gtk::Image::new(),
                title: gtk::Label::new(None),
                detail: gtk::Label::new(None),
                hints: gtk::Box::new(gtk::Orientation::Horizontal, 16),
                inputs: RefCell::new((SyncStatus::default(), 0, 0, 0, None)),
                tick: RefCell::new(None),
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

    /// The state currently on screen, if any.
    pub fn state(&self) -> Option<State> {
        let (status, item_count, stored, queued, searching) = self.imp().inputs.borrow().clone();
        derive(&status, item_count, stored, queued, searching.as_deref())
    }

    fn render(&self) {
        let imp = self.imp();
        let now = Instant::now();
        let (status, item_count, stored, queued, searching) = imp.inputs.borrow().clone();
        let state = derive(&status, item_count, stored, queued, searching.as_deref());

        self.set_visible(state.is_some());
        if let Some(state) = &state {
            let content = describe(state, now);

            imp.icon.set_icon_name(Some(content.icon));
            for class in ["inbox-zero", "offline", "failing", "no-matches"] {
                imp.icon.remove_css_class(class);
            }
            imp.icon.add_css_class(content.icon_class);

            imp.title.set_text(content.title);
            imp.detail.set_text(&content.detail);

            let spoken = content
                .hints
                .iter()
                .map(|(label, key)| format!("{label}, press {key}"))
                .collect::<Vec<_>>()
                .join(". ");
            self.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}. {}. {spoken}",
                content.title, content.detail
            ))]);

            while let Some(child) = imp.hints.first_child() {
                imp.hints.remove(&child);
            }
            for hint in &content.hints {
                imp.hints.append(&hint_widget(hint));
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

fn hint_widget((label, key): &Hint) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("postio-liststate-hint");
    row.set_accessible_role(gtk::AccessibleRole::Presentation);

    let text = gtk::Label::new(Some(label));
    text.add_css_class("postio-liststate-hint-label");
    text.set_accessible_role(gtk::AccessibleRole::Presentation);

    let key = gtk::Label::new(Some(key));
    key.add_css_class("postio-keyhint");
    key.set_accessible_role(gtk::AccessibleRole::Presentation);

    row.append(&text);
    row.append(&key);
    row
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn status(state: ConnectionState) -> SyncStatus {
        SyncStatus {
            state,
            ..SyncStatus::default()
        }
    }

    #[test]
    fn an_empty_online_mailbox_is_inbox_zero() {
        let derived = derive(&status(ConnectionState::Online), 0, 4291, 0, None);
        assert_eq!(
            derived,
            Some(State::InboxZero {
                last_sync: None,
                stored: 4291,
            })
        );
    }

    #[test]
    fn a_search_that_matched_nothing_does_not_claim_the_inbox_is_clear() {
        // The same inputs that make an empty mailbox `InboxZero`. What
        // changes the answer is that the emptiness belongs to the query.
        let derived = derive(
            &status(ConnectionState::Online),
            0,
            4291,
            0,
            Some("from:ada invoice"),
        );
        assert_eq!(
            derived,
            Some(State::NoMatches {
                query: "from:ada invoice".to_string(),
            })
        );
        // Nothing underneath it to keep visible.
        assert_eq!(derived.unwrap().placement(0), Placement::Full);
    }

    #[test]
    fn a_search_answers_for_itself_whatever_the_connection_is_doing() {
        // The index is local and it answered completely, so a connection
        // state over an empty result set would be true and useless -- the
        // local mail is exactly what was just searched.
        for state in [
            ConnectionState::Offline,
            ConnectionState::Connecting,
            ConnectionState::Failing,
        ] {
            assert!(
                matches!(
                    derive(&status(state), 0, 4291, 2, Some("invoice")),
                    Some(State::NoMatches { .. })
                ),
                "{state:?} spoke over the search"
            );
        }
    }

    #[test]
    fn a_search_that_found_something_still_hears_about_the_connection() {
        // The banner is a fact about the rows, not about the query, so
        // finding hits does not silence it.
        assert_eq!(
            derive(
                &status(ConnectionState::Online),
                14,
                4291,
                0,
                Some("invoice")
            ),
            None,
            "a search with hits invented a state of its own"
        );
        let derived = derive(
            &status(ConnectionState::Offline),
            14,
            4291,
            2,
            Some("invoice"),
        );
        assert_eq!(derived, Some(State::Offline { queued: 2 }));
        assert_eq!(
            derived.unwrap().placement(14),
            Placement::Banner,
            "the hits were hidden behind the connection"
        );
    }

    #[test]
    fn the_no_matches_plate_says_the_query_and_the_way_out() {
        let content = describe(
            &State::NoMatches {
                query: "from:ada invoice".to_string(),
            },
            Instant::now(),
        );
        assert!(
            content.detail.contains("from:ada invoice"),
            "the plate does not say what was searched for: {}",
            content.detail
        );
        // Never a dead end: every named state names a key.
        assert_eq!(content.hints, vec![("Back to the folder", "Esc")]);
    }

    #[test]
    fn a_populated_online_mailbox_has_no_named_state() {
        assert_eq!(
            derive(&status(ConnectionState::Online), 12, 4291, 0, None),
            None
        );
    }

    #[test]
    fn offline_is_the_state_regardless_of_how_many_rows_are_loaded() {
        // "Everything already synced still opens" is true whether the
        // mailbox in view is empty or not; the point is the connection, not
        // the count. Whether that turns into a full plate or a banner is
        // `State::placement`'s decision, not `derive`'s — see the
        // `placement` tests below, which is where `postio-ma4` actually
        // lived: this state was always right, only how much of the pane it
        // took was wrong.
        assert_eq!(
            derive(&status(ConnectionState::Offline), 12, 0, 2, None),
            Some(State::Offline { queued: 2 })
        );
    }

    #[test]
    fn only_an_empty_mailbox_gets_the_full_opaque_plate() {
        let offline = State::Offline { queued: 2 };
        let failing = State::Failing {
            reason: "IMAP rejected the credentials.".to_string(),
        };

        // `postio-ma4`: offline or failing with rows already loaded must not
        // hide mail that is synced and readable — canvas 3d's "nothing is a
        // dead end" and CLAUDE.md's "everything already synced still opens"
        // are both broken by an opaque plate over rows that are right there.
        assert_eq!(offline.placement(12), Placement::Banner);
        assert_eq!(failing.placement(12), Placement::Banner);

        // Nothing underneath to hide: the full plate is the right answer,
        // not a banner floating over an empty pane.
        assert_eq!(offline.placement(0), Placement::Full);
        assert_eq!(failing.placement(0), Placement::Full);
    }

    #[test]
    fn inbox_zero_is_always_the_full_plate() {
        // True by construction -- `derive` only ever produces `InboxZero`
        // when `item_count` is already 0 -- but the state's own rule should
        // not silently depend on that invariant holding elsewhere.
        let empty = State::InboxZero {
            last_sync: None,
            stored: 0,
        };
        assert_eq!(empty.placement(0), Placement::Full);
    }

    #[test]
    fn connecting_reads_the_same_as_offline() {
        assert_eq!(
            derive(&status(ConnectionState::Connecting), 0, 0, 0, None),
            Some(State::Offline { queued: 0 })
        );
    }

    #[test]
    fn a_failing_connection_never_shrugs() {
        let with_reason = SyncStatus {
            state: ConnectionState::Failing,
            detail: Some("AUTHENTICATIONFAILED".to_string()),
            ..SyncStatus::default()
        };
        assert_eq!(
            derive(&with_reason, 0, 0, 0, None),
            Some(State::Failing {
                reason: "AUTHENTICATIONFAILED".to_string(),
            })
        );

        let without_reason = status(ConnectionState::Failing);
        let State::Failing { reason } = derive(&without_reason, 0, 0, 0, None).unwrap() else {
            panic!("failing status did not produce a failing state");
        };
        assert!(!reason.is_empty(), "a failing state never shows nothing");
    }

    #[test]
    fn every_state_offers_a_working_key() {
        let now = Instant::now();
        for state in [
            State::InboxZero {
                last_sync: Some(now - Duration::from_secs(12)),
                stored: 4291,
            },
            State::Offline { queued: 2 },
            State::Failing {
                reason: "IMAP rejected the credentials.".to_string(),
            },
            State::NoMatches {
                query: "from:ada invoice".to_string(),
            },
        ] {
            let content = describe(&state, now);
            assert!(!content.hints.is_empty(), "{} offers no key", content.title);
            for (label, key) in &content.hints {
                assert!(!label.is_empty());
                assert!(!key.is_empty());
            }
        }
    }

    #[test]
    fn no_state_ever_shrugs() {
        let now = Instant::now();
        for state in [
            State::InboxZero {
                last_sync: None,
                stored: 0,
            },
            State::Offline { queued: 0 },
            State::Failing {
                reason: "IMAP rejected the credentials.".to_string(),
            },
            State::NoMatches {
                query: "from:ada invoice".to_string(),
            },
        ] {
            let content = describe(&state, now);
            assert_ne!(content.detail.to_lowercase(), "something went wrong");
            assert!(!content.detail.is_empty());
            assert!(content.detail.len() > 10, "too terse to name anything");
        }
    }

    #[test]
    fn inbox_zero_names_when_it_last_synced() {
        let now = Instant::now();
        let never = describe(
            &State::InboxZero {
                last_sync: None,
                stored: 4291,
            },
            now,
        );
        assert!(never.detail.contains("Never synced"));

        let recently = describe(
            &State::InboxZero {
                last_sync: Some(now - Duration::from_secs(12)),
                stored: 4291,
            },
            now,
        );
        assert!(recently.detail.contains("Last synced"));
        assert!(recently.detail.contains("12s"));
    }

    #[test]
    fn offline_names_the_local_store_and_what_is_queued() {
        let now = Instant::now();
        let nothing_queued = describe(&State::Offline { queued: 0 }, now);
        assert!(nothing_queued.detail.contains("still opens"));

        let queued = describe(&State::Offline { queued: 3 }, now);
        assert!(queued.detail.contains("3 changes"));
    }
}
