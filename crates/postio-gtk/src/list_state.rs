//! The list pane's three named states: inbox zero, offline, sync failure.
//!
//! Canvas 3d: each one "names the local store and gives a key, not a
//! shrug." [`derive()`] decides which state applies, and is a pure function
//! tested with no display, the same split [`crate::cheatsheet`] uses.
//! [`ListStateView`] is the widget around it.
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
//! [`ConnectionState::Failing`] carries a typed category, not prose —
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
/// `None` from [`derive()`] means there are rows to show and the widget should
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
        /// Accounts that could not be reached, and so could not be fully
        /// searched. Empty is the ordinary single-account answer.
        ///
        /// ADR 0005 Q10 calls an empty result set the single most important
        /// instance of the omission rule: someone searches for an invoice,
        /// finds nothing, and concludes it does not exist. "Nothing matched"
        /// is a claim about the whole corpus, and a corpus short an account
        /// cannot support it.
        incomplete: Vec<String>,
    },
    /// An aggregate view is showing what it has, and it is not everything.
    ///
    /// ADR 0005 Q10: *a view that cannot include an account says so, names
    /// the account, and stays usable.* Rows from the accounts that did answer
    /// are real mail and stay readable underneath — this is a
    /// [`Placement::Banner`] whenever there is anything to put it over.
    ///
    /// **The rows of a named account are not missing, they are unrefreshed.**
    /// Postio is local-first, so an offline account's synced mail is still in
    /// this list; what cannot be vouched for is that it is current. The
    /// wording says exactly that, because "showing 1 of 2 accounts" would be
    /// its own lie whenever the absent account has mail on disk — which is
    /// almost always.
    Partial {
        /// The accounts that did not answer, in the order the sidebar draws
        /// them — which is the order their hues are keyed to.
        accounts: Vec<String>,
    },
    /// The window is up and the store is not open yet (#1114).
    ///
    /// Postio presents its window before it has opened anything, so this is
    /// the only state in the family that is about the *application* rather
    /// than about mail. It outranks every other: there is no connection
    /// worth describing, no mailbox to be empty, and no query to have
    /// matched nothing, because there is nothing behind the window yet.
    ///
    /// It is also the only one with a threshold — see [`derive_opening`].
    /// An ordinary start never shows it.
    Opening {
        /// What is being waited on, because the four are different waits and
        /// two of them can legitimately take tens of seconds.
        waiting: Waiting,
    },
}

/// What a start that has not finished is actually waiting on.
///
/// Four waits, named separately because "Updating your mailbox's storage" is
/// a different promise from "Opening your mailbox" — and because the two that
/// can legitimately take tens of seconds are the two a person most needs told
/// about. Measured on the live install: a schema migration held a launch for
/// 12.6 s with nothing on screen, and a keyring prompt held another for 28 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// The store key is being read out of the OS keyring.
    ///
    /// A D-Bus round trip against a service that may be showing a passphrase
    /// prompt of its own, which is why this can be the longest of the four
    /// and the one least under Postio's control.
    Keyring,
    /// The encrypted database is being opened.
    Store,
    /// Schema migrations are being applied.
    Migrating,
    /// The local search index is being built or rebuilt.
    Indexing,
}

impl Waiting {
    /// Every wait, in the order a start meets them.
    pub const ALL: [Waiting; 4] = [
        Waiting::Keyring,
        Waiting::Store,
        Waiting::Migrating,
        Waiting::Indexing,
    ];
}

/// How long a start may take before it is worth saying anything.
///
/// Twice `docs/PRODUCT.md` §18's 500 ms budget: by here the start has already
/// failed its own budget, so there is no risk of speaking over an ordinary
/// one. And far enough past the measured ~150 ms store phase that it cannot
/// fire on a healthy launch at all — which matters more than the exact
/// number, because a plate that appears and is gone inside 100 ms is the
/// flicker §18 forbids rather than the reassurance it was meant to be.
pub const OPENING_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(1);

/// What the list pane shows while the store is still opening, if anything.
///
/// `None` below the threshold, and that is the whole of #1114's "normal case
/// — nothing that will disappear": no spinner, no skeleton rows, no
/// "Loading…", no progress of any kind. The ordinary start draws its first
/// frame and then fills it, with nothing in between to be removed.
pub fn derive_opening(waiting: Waiting, waited: std::time::Duration) -> Option<State> {
    (waited >= OPENING_THRESHOLD).then_some(State::Opening { waiting })
}

/// The heading and the line under it for one wait.
///
/// Split out of [`describe`] so the copy can be asserted on without a
/// display, the same reason [`derive`] is a pure function.
pub fn describe_wait(waiting: Waiting) -> (&'static str, &'static str) {
    match waiting {
        // Named as the keyring rather than as Postio, because what to *do*
        // about it is somewhere else entirely: an unlock prompt that is
        // behind another window, or a keyring that is not running.
        Waiting::Keyring => (
            "Opening your mailbox",
            "Waiting for the keyring to unlock the local store.",
        ),
        Waiting::Store => ("Opening your mailbox", "Reading the local store from disk."),
        // A different promise, and deliberately so: this one changes the
        // store rather than reading it, it is once per upgrade, and it is
        // the wait that has actually taken tens of seconds on a real
        // mailbox.
        Waiting::Migrating => (
            "Updating your mailbox\u{2019}s storage",
            "This happens once after an update, and the mail is not touched.",
        ),
        Waiting::Indexing => (
            "Rebuilding the search index",
            "Your mail is all here; searching it will be ready in a moment.",
        ),
    }
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
    /// `item_count` is the same count [`derive()`] was given: `InboxZero` never
    /// needs it, since being empty is what put it here in the first place,
    /// but `Offline` and `Failing` can arrive with a full mailbox loaded
    /// underneath, and that is exactly the mail the banner treatment exists
    /// to keep visible.
    pub fn placement(&self, item_count: u64) -> Placement {
        match self {
            // Nothing has been read, so there is nothing underneath to
            // protect: the plate is the pane.
            State::Opening { .. } => Placement::Full,
            State::InboxZero { .. } | State::NoMatches { .. } => Placement::Full,
            State::Offline { .. } | State::Failing { .. } | State::Partial { .. } => {
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
            // One account, and it is the one that just answered: there is no
            // other account whose absence could have hidden a match.
            incomplete: Vec::new(),
        });
    }
    match status.state {
        ConnectionState::Failing { .. } => Some(State::Failing {
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

/// Whether an account's contribution to an aggregate can be vouched for.
///
/// Public because it is also what a whole-view selection is scoped by: the
/// accounts the banner does *not* name are exactly the accounts `Ctrl+A` here
/// is about, and two spellings of "reachable" would let the banner and the
/// selection disagree about which account is which (#811).
///
/// [`ConnectionState::Connecting`] is deliberately *not* a reason to name an
/// account. The single-account states fold it into
/// [`State::Offline`] because from the user's chair both mean
/// "local mail still works", and that is right for a whole-pane statement
/// about the one account they are looking at. A banner is a different act: it
/// names an account, and one that appears for the two seconds an account
/// takes to connect — on every launch, for every account — is how people
/// learn to stop reading banners.
pub fn is_current(status: &SyncStatus) -> bool {
    match status.state {
        ConnectionState::Online | ConnectionState::Connecting => true,
        ConnectionState::Offline | ConnectionState::Failing { .. } => false,
    }
}

/// Which state an *aggregate* view shows — the unified list, across accounts.
///
/// ADR 0005 Q10's rule: **a view that cannot include an account says so,
/// names the account, and stays usable.** [`fn@derive`] answers for one account
/// and cannot express this; the difference is not the number of statuses but
/// that a whole-pane "Offline" would be a claim about every account when only
/// one of them is away.
///
/// `accounts` carries one entry per **enabled** account, in the sidebar's own
/// order. That an account disabled by the user simply is not in the list is
/// the whole of Q10's disabled-account rule: it drops out silently and
/// correctly, because the user asked for that, and there is nothing to
/// disclose about a view that is showing what it was told to show.
///
/// The order of the checks is the argument:
///
/// 1. **A search that matched nothing** answers first, and carries the
///    accounts it could not reach — the instance Q10 calls the most
///    important, because "nothing matched" reads as proof.
/// 2. **An account that did not answer** outranks everything else, including
///    inbox zero: "nothing left to triage" is a claim about every account.
/// 3. Otherwise the aggregate behaves as one healthy view does.
pub fn derive_aggregate(
    accounts: &[(String, SyncStatus)],
    item_count: u64,
    stored: u64,
    searching: Option<&str>,
) -> Option<State> {
    let absent: Vec<String> = accounts
        .iter()
        .filter(|(_, status)| !is_current(status))
        .map(|(name, _)| name.clone())
        .collect();

    if let Some(query) = searching.filter(|_| item_count == 0) {
        return Some(State::NoMatches {
            query: query.to_string(),
            incomplete: absent,
        });
    }
    if !absent.is_empty() {
        return Some(State::Partial { accounts: absent });
    }

    // Every account answered, so the aggregate can speak with one voice --
    // and the only thing left worth saying is that there is nothing in it.
    // The oldest last sync across the accounts, because the freshest would
    // overstate how current the view is.
    if item_count == 0 && !accounts.is_empty() {
        return Some(State::InboxZero {
            last_sync: accounts
                .iter()
                .map(|(_, status)| status.last_sync)
                .min()
                .flatten(),
            stored,
        });
    }
    None
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
        State::NoMatches { query, incomplete } => Content {
            icon: "system-search-symbolic",
            icon_class: "no-matches",
            title: "No matches",
            // The caveat goes *after* the query, not instead of it: what was
            // searched for is still the thing to change. But "nothing
            // matches" reads as proof the mail does not exist, so an
            // unreachable account has to be named here or the sentence is a
            // lie by omission (ADR 0005 Q10).
            detail: match incomplete.as_slice() {
                [] => format!("Nothing in the local store matches \u{201c}{query}\u{201d}."),
                absent => format!(
                    "Nothing in the local store matches \u{201c}{query}\u{201d}. {} \
                     not reachable, so {} mail was searched only as far as it \
                     had already synced.",
                    naming(absent),
                    if absent.len() == 1 { "its" } else { "their" },
                ),
            },
            hints: vec![("Back to the folder", "Esc")],
        },
        // The one plate in the family that offers no verb, and that is
        // correct rather than an omission: the work is in flight, so `R`
        // would either do nothing or restart a read that is already running.
        State::Opening { waiting } => {
            let (title, detail) = describe_wait(*waiting);
            Content {
                icon: "content-loading-symbolic",
                icon_class: "opening",
                title,
                detail: detail.to_string(),
                hints: Vec::new(),
            }
        }
        State::Partial { accounts } => Content {
            icon: "network-offline-symbolic",
            icon_class: "offline",
            // Named in the title, because the account is the fact. A title
            // that said "Some accounts are offline" would make the reader
            // open something else to find out which.
            title: "Showing local mail",
            detail: format!(
                "{} not reachable, so {} mail is what was already synced. \
                 Everything here still opens.",
                naming(accounts),
                if accounts.len() == 1 { "its" } else { "their" },
            ),
            hints: vec![("Retry now", "R")],
        },
    }
}

/// "Personal is", "Personal and Work are", "Personal, Work and Archive are".
///
/// Every name, never "and 2 others": naming one of three absent accounts is
/// its own omission, and the list is bounded by how many accounts a person
/// configures.
fn naming(accounts: &[String]) -> String {
    let verb = if accounts.len() == 1 { "is" } else { "are" };
    // The joining itself is `postio_ui::format::names`, shared with the
    // selection summary: the banner and the summary name the same absent
    // accounts and must spell the list the same way (#811).
    format!("{} {verb}", postio_ui::format::names(accounts))
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
                inputs: RefCell::new((SyncStatus::default(), 0, 0, 0, None)),
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
        match &aggregate {
            Some(accounts) => derive_aggregate(accounts, item_count, stored, searching.as_deref()),
            None => derive(&status, item_count, stored, queued, searching.as_deref()),
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
    fn an_ordinary_start_says_nothing_at_all() {
        // #1114's first acceptance line, and the whole reason the threshold
        // exists: the measured store phase is tens of milliseconds, and
        // anything drawn and removed inside that is flicker. `PRODUCT.md`
        // §18 allows a transition of ≤100ms *or none*, and this path adds
        // none.
        for waited in [Duration::ZERO, Duration::from_millis(999)] {
            assert_eq!(
                derive_opening(Waiting::Store, waited),
                None,
                "a plate at {waited:?} would be on screen for less time than \
                 it takes to read, and gone before anybody could"
            );
        }
    }

    #[test]
    fn a_start_that_has_already_failed_its_budget_says_what_it_is_waiting_on() {
        // Twice `PRODUCT.md` §18's 500ms budget, and far enough past the
        // measured ~150ms that it can never fire on an ordinary start.
        let plate = derive_opening(Waiting::Migrating, OPENING_THRESHOLD)
            .expect("past the threshold, the wait is worth naming");
        assert_eq!(
            plate,
            State::Opening {
                waiting: Waiting::Migrating
            }
        );
        assert_eq!(
            plate.placement(0),
            Placement::Full,
            "there are no rows behind this one — there is no store to have \
             read any — so there is nothing for a banner to protect"
        );
    }

    #[test]
    fn each_wait_is_named_as_itself() {
        // #1114: "one line of specific copy naming what is being waited on
        // — the keyring read and the database open are different waits and
        // the line should say which." So it is the *line* that has to be
        // unique. The heading is deliberately shared by the two ordinary
        // waits, because both are the same promise to the reader: your
        // mailbox is opening.
        let details: Vec<&str> = Waiting::ALL.iter().map(|w| describe_wait(*w).1).collect();
        let mut unique = details.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            details.len(),
            "two waits say the same thing, so the line names a wait it is \
             not: {details:?}"
        );

        assert_eq!(describe_wait(Waiting::Keyring).0, "Opening your mailbox");
        assert_eq!(describe_wait(Waiting::Store).0, "Opening your mailbox");
        // And the two that change the store rather than reading it promise
        // something else, because they are something else: they are once per
        // upgrade and they are the waits that have actually taken tens of
        // seconds on a real mailbox.
        for different in [Waiting::Migrating, Waiting::Indexing] {
            assert_ne!(
                describe_wait(different).0,
                "Opening your mailbox",
                "{different:?} is not the same promise as opening a mailbox"
            );
        }
    }

    #[test]
    fn the_opening_plate_offers_no_verb() {
        // The one plate in the family with no key hint and no retry, and
        // that is correct rather than an omission: the work is in flight, so
        // `R` would either do nothing or restart a read that is already
        // running.
        let content = describe(
            &State::Opening {
                waiting: Waiting::Keyring,
            },
            Instant::now(),
        );
        assert!(
            content.hints.is_empty(),
            "offering a verb here promises something to press, and there is \
             nothing: {:?}",
            content.hints
        );
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
                incomplete: Vec::new(),
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
            ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
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
                incomplete: Vec::new(),
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
            state: ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
            detail: Some("AUTHENTICATIONFAILED".to_string()),
            ..SyncStatus::default()
        };
        assert_eq!(
            derive(&with_reason, 0, 0, 0, None),
            Some(State::Failing {
                reason: "AUTHENTICATIONFAILED".to_string(),
            })
        );

        let without_reason = status(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        });
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
                incomplete: Vec::new(),
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
                incomplete: Vec::new(),
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

#[cfg(test)]
mod aggregate_tests {
    use super::*;

    fn status(state: ConnectionState) -> SyncStatus {
        SyncStatus {
            state,
            ..SyncStatus::default()
        }
    }

    fn named(entries: &[(&str, ConnectionState)]) -> Vec<(String, SyncStatus)> {
        entries
            .iter()
            .map(|(name, state)| ((*name).to_owned(), status(*state)))
            .collect()
    }

    #[test]
    fn every_account_online_says_nothing_at_all() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            None,
            "a complete view has nothing to disclose, and a banner that is \
             always up is a banner nobody reads"
        );
    }

    #[test]
    fn an_unreachable_account_is_named_over_the_rows_it_could_not_refresh() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 40, 4291, None);
        assert_eq!(
            derived,
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            }),
            "ADR 0005 Q10: the view names the account it cannot vouch for"
        );
        assert_eq!(
            derived.unwrap().placement(40),
            Placement::Banner,
            "the rows are real mail and stay readable -- covering them to say \
             the view is incomplete keeps the promise in words and breaks it \
             on screen"
        );
    }

    #[test]
    fn a_failing_account_counts_as_one_it_cannot_vouch_for_too() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            (
                "Personal",
                ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
            ),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            })
        );
    }

    #[test]
    fn connecting_is_not_worth_naming_because_it_resolves_itself() {
        // The single-account states fold `Connecting` into `Offline`, because
        // from the user's chair both mean "local mail still works". A banner
        // is different: it names an account, and one that appears for the two
        // seconds an account takes to connect -- on every launch, for every
        // account -- teaches people to stop reading banners.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Connecting),
        ]);
        assert_eq!(derive_aggregate(&accounts, 40, 4291, None), None);
    }

    #[test]
    fn every_unreachable_account_is_named_in_the_order_given() {
        let accounts = named(&[
            ("Work", ConnectionState::Offline),
            ("Personal", ConnectionState::Online),
            ("Archive", ConnectionState::Offline),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            Some(State::Partial {
                accounts: vec!["Work".to_owned(), "Archive".to_owned()],
            }),
            "naming one of two absent accounts is its own lie by omission, \
             and the order is the sidebar's so the colours line up"
        );
    }

    #[test]
    fn a_search_that_found_nothing_while_an_account_is_away_says_so() {
        // ADR 0005 Q10 calls this the single most important instance of the
        // rule: someone searches for an invoice, finds nothing, and concludes
        // it does not exist. `NoMatches` on its own is a claim about the
        // whole corpus, and here the corpus is short an account.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 0, 4291, Some("invoice"));
        assert_eq!(
            derived,
            Some(State::NoMatches {
                query: "invoice".to_owned(),
                incomplete: vec!["Personal".to_owned()],
            }),
            "an empty result set has to carry the accounts it could not \
             search fully, or it reads as proof the mail is not there"
        );
    }

    #[test]
    fn a_search_that_found_nothing_with_everything_online_is_a_plain_no_match() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 0, 4291, Some("invoice")),
            Some(State::NoMatches {
                query: "invoice".to_owned(),
                incomplete: Vec::new(),
            }),
            "nothing to disclose, so the answer is the same one a single \
             account gives"
        );
    }

    #[test]
    fn an_empty_aggregate_with_everything_online_is_still_inbox_zero() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 0, 4291, None),
            Some(State::InboxZero {
                last_sync: None,
                stored: 4291,
            })
        );
    }

    #[test]
    fn an_unreachable_account_outranks_inbox_zero_when_there_are_no_rows() {
        // "Nothing left to triage" is a claim about every account, and one of
        // them did not answer. With nothing underneath, it takes the plate.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 0, 4291, None);
        assert_eq!(
            derived,
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            })
        );
        assert_eq!(derived.unwrap().placement(0), Placement::Full);
    }

    #[test]
    fn a_disabled_account_never_reaches_here_so_it_is_never_named() {
        // ADR 0005 Q10: `enabled = 0` drops out of Unified silently and
        // correctly, because the user asked for that. It is expressed as the
        // caller passing only enabled accounts -- an empty list is a view
        // with nothing to disclose rather than one that is degraded.
        assert_eq!(derive_aggregate(&[], 0, 0, None), None);
    }
}
