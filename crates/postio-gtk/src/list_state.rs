//! The list pane's three named states: inbox zero, offline, sync failure.
//!
//! Canvas 3d: each one "names the local store and gives a key, not a
//! shrug." [`derive`] decides which state applies, and is a pure function
//! tested with no display, the same split [`crate::cheatsheet`] uses.
//! [`ListStateView`] is the widget around it.
//!
//! # Where it lives
//!
//! It is an overlay over [`crate::list_view::MessageListView`], and hides
//! itself the moment [`derive`] returns `None` — there are rows to show. So
//! the list pane always has its header and its rows underneath, and this
//! widget is an opaque plate that covers them when there is something to say
//! instead.
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
}

/// Which state the list pane shows, from what it knows right now.
///
/// `item_count` is what the loaded page of the windowed model reports for
/// the mailbox in view; `stored` and `queued` come from the local store and
/// do not depend on the server being reachable at all — that is the whole
/// point of "everything already synced still opens."
///
/// [`ConnectionState::Connecting`] folds into [`State::Offline`]: from the
/// user's chair both mean "not connected right now, local mail still
/// works," and a fourth named state for a transition that resolves itself
/// would be a state nobody could tell apart from the one before it.
pub fn derive(status: &SyncStatus, item_count: u64, stored: u64, queued: u64) -> Option<State> {
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
        pub inputs: RefCell<(SyncStatus, u64, u64, u64)>,
        pub tick: RefCell<Option<glib::SourceId>>,
    }

    impl Default for ListStateView {
        fn default() -> Self {
            Self {
                icon: gtk::Image::new(),
                title: gtk::Label::new(None),
                detail: gtk::Label::new(None),
                hints: gtk::Box::new(gtk::Orientation::Horizontal, 16),
                inputs: RefCell::new((SyncStatus::default(), 0, 0, 0)),
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
        // Fill, not centre: this widget sits *over* the message list now
        // that `crate::list_view` fills the pane, so it has to be an opaque
        // plate covering the rows rather than a caption printed across
        // them. The column inside it is what centres.
        self.set_valign(gtk::Align::Fill);
        self.set_vexpand(true);

        imp.icon.add_css_class("postio-liststate-icon");
        imp.icon.set_pixel_size(30);

        imp.title.add_css_class("postio-liststate-title");
        imp.title.set_justify(gtk::Justification::Center);
        imp.title.set_wrap(true);

        imp.detail.add_css_class("postio-liststate-detail");
        imp.detail.set_justify(gtk::Justification::Center);
        imp.detail.set_wrap(true);
        imp.detail.set_max_width_chars(36);

        imp.hints.set_halign(gtk::Align::Center);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        column.set_halign(gtk::Align::Center);
        column.set_valign(gtk::Align::Center);
        column.set_vexpand(true);
        column.set_margin_start(32);
        column.set_margin_end(32);
        column.append(&imp.icon);
        column.append(&imp.title);
        column.append(&imp.detail);
        column.append(&imp.hints);
        self.set_child(Some(&column));

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
    /// Call it whenever any of those change. The widget hides itself the
    /// instant there are rows to show.
    pub fn set_status(&self, status: SyncStatus, item_count: u64, stored: u64, queued: u64) {
        let inputs = (status, item_count, stored, queued);
        // Cheap to call and cheap to call often: the row count moves with
        // every page the message list takes delivery of, and re-rendering
        // an unchanged state would also re-arm the age timer each time.
        if *self.imp().inputs.borrow() == inputs {
            return;
        }
        *self.imp().inputs.borrow_mut() = inputs;
        self.render();
    }

    /// The state currently on screen, if any.
    pub fn state(&self) -> Option<State> {
        let (status, item_count, stored, queued) = self.imp().inputs.borrow().clone();
        derive(&status, item_count, stored, queued)
    }

    fn render(&self) {
        let imp = self.imp();
        let now = Instant::now();
        let (status, item_count, stored, queued) = imp.inputs.borrow().clone();
        let state = derive(&status, item_count, stored, queued);

        self.set_visible(state.is_some());
        if let Some(state) = &state {
            let content = describe(state, now);

            imp.icon.set_icon_name(Some(content.icon));
            for class in ["inbox-zero", "offline", "failing"] {
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
        let derived = derive(&status(ConnectionState::Online), 0, 4291, 0);
        assert_eq!(
            derived,
            Some(State::InboxZero {
                last_sync: None,
                stored: 4291,
            })
        );
    }

    #[test]
    fn a_populated_online_mailbox_has_no_named_state() {
        assert_eq!(derive(&status(ConnectionState::Online), 12, 4291, 0), None);
    }

    #[test]
    fn offline_wins_even_with_rows_still_loaded() {
        // "Everything already synced still opens" is true whether the
        // mailbox in view is empty or not; the point is the connection, not
        // the count.
        assert_eq!(
            derive(&status(ConnectionState::Offline), 12, 0, 2),
            Some(State::Offline { queued: 2 })
        );
    }

    #[test]
    fn connecting_reads_the_same_as_offline() {
        assert_eq!(
            derive(&status(ConnectionState::Connecting), 0, 0, 0),
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
            derive(&with_reason, 0, 0, 0),
            Some(State::Failing {
                reason: "AUTHENTICATIONFAILED".to_string(),
            })
        );

        let without_reason = status(ConnectionState::Failing);
        let State::Failing { reason } = derive(&without_reason, 0, 0, 0).unwrap() else {
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
