//! Every custom widget, as a screen reader meets it.
//!
//! docs/PRODUCT.md §20 makes accessibility first-class, and the expensive way to
//! discover it was not is to open Orca after the fact. GTK ships the audit as
//! a test API — `gtk_test_accessible_has_property` and friends — so the tree
//! can be walked and checked without a screen reader in the loop, which is
//! what this does. Orca still has to be run by a person; what it should never
//! be is the *first* thing that notices a row with no name.
//!
//! `prefers-reduced-motion` is not checked here because there is nothing to
//! check: the stylesheets carry no transition at all, and
//! `gtk_shell.rs::nothing_in_the_stylesheet_outruns_the_motion_budget`
//! already reads them back to keep it that way. A second copy of that gate
//! would be one more thing to keep in step, not one more thing guarded.
//!
//! One test function, for the reason `gtk_style.rs` gives. Skips without a
//! display. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRelation, AccessibleRole};
use postio_gtk::feed::{
    Feeds, MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::finder;
use postio_gtk::list::Row;
use postio_gtk::list_state::State;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// Roles a screen reader announces by name. A control it cannot name is a
/// control it cannot offer.
const NEEDS_A_NAME: &[AccessibleRole] = &[
    AccessibleRole::Button,
    AccessibleRole::Checkbox,
    AccessibleRole::ComboBox,
    AccessibleRole::Link,
    AccessibleRole::ListItem,
    AccessibleRole::MenuItem,
    AccessibleRole::Row,
    AccessibleRole::SearchBox,
    AccessibleRole::Switch,
    AccessibleRole::Tab,
    AccessibleRole::TextBox,
    AccessibleRole::ToggleButton,
];

/// Roles that say "I am a widget" and nothing a screen reader can use.
const SAYS_NOTHING: &[AccessibleRole] = &[AccessibleRole::Generic, AccessibleRole::Widget];

struct Sample;

impl MailboxSource for Sample {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let folder = |id: i64, path: &str, role, unread| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total: 940,
                unread,
                flagged: 0,
            };
            mailbox.last_synced_at = Some(Utc::now() - chrono::Duration::seconds(12));
            mailbox
        };
        let folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox, 12),
            folder(2, "Archive", MailboxRole::Archive, 0),
            folder(3, "lkml", MailboxRole::Regular, 204),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Sample {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = 300;
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: MessageId::new(position as i64 + 1),
                    // Real rows carry a thread, and `t` refuses to drill into
                    // one that does not — a fixture without it quietly makes
                    // the thread column unreachable.
                    thread: Some(postio_model::ids::ThreadId::new(position as i64 + 1)),
                    from: Some(postio_model::address::EmailAddress::new(
                        Some("Ada Lovelace"),
                        "ada@example.com",
                    )),
                    subject: Some(format!("a subject, number {position}")),
                    preview: Some("a snippet under it".into()),
                    received_at: Utc
                        .timestamp_opt(1_700_000_000 - position as i64, 0)
                        .unwrap(),
                    seen: position % 2 == 0,
                    flagged: false,
                    answered: false,
                    draft: false,
                    has_attachments: false,
                    thread_count: 1,
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

/// A mailbox with one folder and never any rows in it.
///
/// `list_state.rs`'s four named states only appear over an empty list, and
/// [`Sample`] above always answers with up to 300 rows — so they need a feed
/// of their own rather than the populated window every other assertion in
/// this file shares.
struct Empty;

impl MailboxSource for Empty {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let mut mailbox = Mailbox::new(account, "INBOX", Some('/'));
        mailbox.id = MailboxId::new(1);
        mailbox.role = MailboxRole::Inbox;
        Box::pin(async move { Ok(vec![mailbox]) })
    }
}

impl MessageSource for Empty {
    fn fetch(&self, _request: PageRequest) -> PageFuture {
        Box::pin(async move {
            Ok(Page {
                total: 0,
                rows: Vec::new(),
            })
        })
    }
}

/// One of `list_state.rs`'s named states, and how to reach it.
struct ListState {
    name: &'static str,
    enter: fn(&Window, &Feeds),
    /// Whether `derive()` actually produced this state, using the exact
    /// accessor `Window::refresh_list_state` reads — not a guess made from
    /// the widget tree, the same discipline `Surface::shown` uses.
    matches: fn(&Window) -> bool,
}

/// The four states `list_state.rs` builds, in the order a user could
/// plausibly meet them: mail arrives to nothing left, the link drops, the
/// link fails outright, a search comes up empty.
fn named_list_states() -> Vec<ListState> {
    vec![
        ListState {
            name: "inbox zero",
            enter: |_window, feeds| {
                feeds.apply(&postio_core::Event::ConnectionChanged {
                    account: AccountId::new(2),
                    state: postio_core::ConnectionState::Online,
                });
            },
            matches: |window| matches!(window.list_state().state(), Some(State::InboxZero { .. })),
        },
        ListState {
            name: "offline",
            enter: |_window, feeds| {
                feeds.apply(&postio_core::Event::ConnectionChanged {
                    account: AccountId::new(2),
                    state: postio_core::ConnectionState::Offline,
                });
            },
            matches: |window| matches!(window.list_state().state(), Some(State::Offline { .. })),
        },
        ListState {
            name: "sync failure",
            enter: |_window, feeds| {
                feeds.apply(&postio_core::Event::ConnectionChanged {
                    account: AccountId::new(2),
                    state: postio_core::ConnectionState::Failing {
                        reason: postio_core::FailureReason::Auth,
                    },
                });
            },
            matches: |window| matches!(window.list_state().state(), Some(State::Failing { .. })),
        },
        ListState {
            name: "a search with no results",
            enter: |window, feeds| {
                // Reconnected first, so this state is reached for the reason
                // its own name gives -- a search that matched nothing --
                // rather than riding in on whatever connection state the
                // previous case left behind.
                feeds.apply(&postio_core::Event::ConnectionChanged {
                    account: AccountId::new(2),
                    state: postio_core::ConnectionState::Online,
                });
                window.set_searching(Some("a query with no matches"));
            },
            matches: |window| matches!(window.list_state().state(), Some(State::NoMatches { .. })),
        },
    ]
}

#[test]
fn every_widget_a_screen_reader_meets_has_a_role_and_a_name() {
    // Before GTK initialises, or it has already chosen a backend. See
    // `require_an_accessibility_backend` for why this is the difference
    // between an audit and a formality.
    //
    // SAFETY: the first statement of the only test in this binary, so no
    // other thread exists yet to observe the environment changing.
    unsafe { std::env::set_var("GTK_A11Y", "test") };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let sample = Rc::new(Sample);
    let feeds = window.install_feeds(
        AccountId::new(1),
        "lena@example.com",
        sample.clone(),
        sample,
    );
    feeds.apply(&postio_core::Event::ConnectionChanged {
        account: AccountId::new(1),
        state: postio_core::ConnectionState::Online,
    });
    // Wait for the page to land and the list to build rows from it. Pumping
    // once only drains what is already pending, which is a race the live
    // display used to win by being slow — see the note on `pump_until`.
    pump_until(|| page_landed(&window) && row_items(&window) > 0);

    require_an_accessibility_backend();

    // ── the panes are landmarks a screen reader can navigate by ──────────
    assert!(gtk::test_accessible_has_role(
        &window.shell().sidebar(),
        AccessibleRole::Navigation
    ));
    assert!(gtk::test_accessible_has_role(
        &window.shell().list(),
        AccessibleRole::List
    ));
    assert!(gtk::test_accessible_has_role(
        &window.shell().reader(),
        AccessibleRole::Article
    ));

    // ── every row announces what it draws ────────────────────────────────
    // The row widget paints its own text, so GTK has nothing to compute a
    // name from: without the sentence being handed over, a screen reader
    // walks a list of anonymous items.
    let rows = row_widgets(&window);
    assert!(!rows.is_empty(), "no rows to check — the list never filled");
    for item in &rows {
        assert!(
            named(item),
            "a visible row in the list has no name for a screen reader"
        );
    }

    // ── 200% text stays usable ───────────────────────────────────────────
    // Not a look-and-see: if the row's type came from constants rather than
    // from the cascade, its height would not move at all and the text would
    // simply overflow it.
    let settings = gtk::Settings::default().expect("a settings object");
    let normal = settings.gtk_xft_dpi();
    let row_height = |window: &Window| {
        let tallest = std::cell::Cell::new(0.0f32);
        window
            .list()
            .each_row(|row| tallest.set(tallest.get().max(row.measured_height(404))));
        tallest.get()
    };
    let before = row_height(&window);
    settings.set_gtk_xft_dpi(normal * 2);
    pump();
    let scaled = row_height(&window);
    settings.set_gtk_xft_dpi(normal);
    pump();
    assert!(
        scaled > before * 1.4,
        "rows are {before}px at 100% and {scaled}px at 200% — the type is not \
         coming from the cascade"
    );

    // ── and nothing in the tree is nameless or roleless ──────────────────
    expect_usable(&window, "the three panes");

    // ── including the surfaces that only exist once opened ───────────────
    // The audit above walks whatever happens to be on screen, and in a
    // default window that is three panes. Every other surface in the
    // application — the ones reached by a key, which is most of them — was
    // never looked at by anything. A pane that announces nothing is no
    // better for being one keystroke away.
    for surface in surfaces() {
        (surface.open)(&window);
        pump();
        // An audit of a surface that never opened is an audit of the three
        // panes again, and it passes. This repository has shipped a pane
        // nothing could reach while every test was green, so each surface
        // has to say how you can tell it is there — using the same predicate
        // the application's own `Back` handler uses.
        assert!(
            (surface.shown)(&window),
            "{}: it did not open, so the audit below would have walked the \
             same tree as before and passed for free",
            surface.name
        );
        expect_usable(&window, surface.name);
        (surface.close)(&window);
        pump();
        assert!(
            !(surface.shown)(&window),
            "{}: it did not close, so every surface audited after it would \
             be audited through this one",
            surface.name
        );
    }

    // ── the named list states, over a mailbox with nothing in it ─────────
    // `list_state.rs`'s four named states -- inbox zero, offline, sync
    // failure, a search with no results -- only ever appear over an empty
    // list, so `Sample`'s 300 rows above can never reach them. A window of
    // their own, fed by `Empty`, is what does.
    let empty_window = Window::default();
    empty_window.present();
    let empty = Rc::new(Empty);
    let empty_feeds =
        empty_window.install_feeds(AccountId::new(2), "empty@example.com", empty.clone(), empty);
    pump_until(|| empty_window.list_state().state().is_some());

    for list_state in named_list_states() {
        (list_state.enter)(&empty_window, &empty_feeds);
        pump();
        assert!(
            (list_state.matches)(&empty_window),
            "{}: derive() did not produce the expected state, so the audit \
             below would have checked whatever was on screen before it",
            list_state.name
        );
        // `expect_usable` walks the tree for roles it recognises, and the
        // pane's own `Status` role is not one of them — nor could it be:
        // a live region says something by *changing* its own Label, which
        // is exactly the property a generic tree walk cannot tell apart
        // from a name computed from a visible child, the way a button's
        // is. Ask the one question that matters for a live region
        // directly, on the exact widget a screen reader is watching.
        let pane = empty_window.list_state();
        let widget = pane.upcast_ref::<gtk::Widget>();
        assert!(
            gtk::test_accessible_has_property(widget, AccessibleProperty::Label)
                && !labelled_empty(widget),
            "{}: the list state pane's Label property is unset or empty, so \
             a screen reader watching this live region hears nothing",
            list_state.name
        );
        expect_usable(&empty_window, list_state.name);
    }

    empty_window.destroy();

    // ── the one exception is still an exception ──────────────────────────
    // If libadwaita starts naming its dismiss button, `upstream_gap` stops
    // matching and this fails — which is the point. An allowance nobody
    // re-checks outlives the problem it was written for.
    assert!(
        UPSTREAM_GAPS_HIT.with(|hit| hit.get()) > 0,
        "no widget needed the AdwToastWidget allowance — if libadwaita now \
         names its dismiss button, delete `upstream_gap` rather than leaving \
         a dead excuse in the audit"
    );

    window.destroy();
}

/// A surface that is not on screen until something opens it.
struct Surface {
    name: &'static str,
    open: fn(&Window),
    /// Whether it is on screen — the application's own notion of open, not
    /// a guess made from the widget tree.
    shown: fn(&Window) -> bool,
    close: fn(&Window),
}

/// Every surface reachable in the running application, in the state a user
/// meets it in.
///
/// Opened through the same `Window` methods the command handlers call, so a
/// surface that stops being reachable stops compiling here rather than
/// quietly dropping out of the audit.
///
/// The named list states `list_state.rs` builds are not here: they only
/// appear over an *empty* list, so they need a feed of their own rather than
/// the populated one this test installs, and get it further down in
/// [`named_list_states`].
fn surfaces() -> Vec<Surface> {
    vec![
        Surface {
            name: "the cheat sheet",
            open: |window| window.open_cheatsheet(),
            shown: |window| window.cheatsheet().is_visible(),
            close: |window| window.close_cheatsheet(),
        },
        Surface {
            name: "settings",
            open: |window| window.open_settings(),
            shown: |window| window.settings().is_visible(),
            close: |window| window.close_settings(),
        },
        Surface {
            name: "the finder, searching",
            open: |window| window.open_finder(finder::Mode::Search),
            shown: |window| window.finder().is_open(),
            close: |window| window.close_finder(),
        },
        Surface {
            name: "the finder, running a command",
            open: |window| window.open_finder(finder::Mode::Command),
            shown: |window| window.finder().is_open(),
            close: |window| window.close_finder(),
        },
        Surface {
            name: "the finder, jumping to a folder",
            open: |window| window.open_finder(finder::Mode::Mailbox),
            shown: |window| window.finder().is_open(),
            close: |window| window.close_finder(),
        },
        Surface {
            name: "the finder, finding a correspondent",
            open: |window| window.open_finder(finder::Mode::Contact),
            shown: |window| window.finder().is_open(),
            close: |window| window.close_finder(),
        },
        Surface {
            name: "the parts panel",
            open: |window| {
                window.open_parts(
                    "text/plain",
                    &[postio_model::Attachment {
                        id: postio_model::ids::AttachmentId::new(1),
                        message_id: MessageId::new(1),
                        filename: Some("minutes.pdf".into()),
                        mime_type: "application/pdf".into(),
                        size: 8_192,
                        content_id: None,
                        disposition: postio_model::attachment::Disposition::Attachment,
                        part_id: Some("2".into()),
                        blob_id: None,
                    }],
                )
            },
            shown: |window| window.parts().is_visible(),
            close: |window| window.close_parts(),
        },
        Surface {
            name: "a thread drilled into",
            open: |window| {
                // The cursor is what `t` drills into, so put it somewhere
                // first — reaching past it into the model would test a path
                // no keystroke takes.
                window.list().first_row();
                let row = window
                    .list()
                    .cursor_row()
                    .expect("the list has rows, so it has a cursor row");
                window.open_thread(&row);
            },
            shown: |window| window.thread_open(),
            close: |window| window.close_thread(),
        },
        Surface {
            name: "the composer",
            open: |window| {
                window
                    .composer()
                    .open(postio_model::Draft::new(AccountId::new(1)))
            },
            shown: |window| window.composer().is_open(),
            close: |window| {
                window.composer().discard();
            },
        },
        Surface {
            name: "an undoable action's toast",
            open: |window| window.show_action_completed("Archived 1 message", true),
            shown: |window| showing(window, "AdwToastWidget"),
            close: |window| {
                if let Some(toast) = find(window.upcast_ref::<gtk::Widget>(), "AdwToastWidget") {
                    toast.set_visible(false);
                }
            },
        },
    ]
}

/// Whether a widget of this type is on screen.
///
/// For surfaces the `Window` keeps no handle to — a toast belongs to the
/// overlay that shows it, and asking the tree is the only way to ask.
fn showing(window: &Window, type_name: &str) -> bool {
    find(window.upcast_ref::<gtk::Widget>(), type_name).is_some()
}

fn find(widget: &gtk::Widget, type_name: &str) -> Option<gtk::Widget> {
    if widget.is_visible() && widget.type_().name() == type_name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, type_name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

/// Audit everything currently on screen, and say which surface it was.
fn expect_usable(window: &Window, surface: &str) {
    let mut problems = Vec::new();
    audit(
        window.upcast_ref::<gtk::Widget>(),
        "window",
        false,
        &mut problems,
    );
    assert!(
        problems.is_empty(),
        "{surface}: {} widget(s) a screen reader cannot use:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

/// Walk the tree and collect what a screen reader could not use.
///
/// `inside` is whether some ancestor is already a named control. A
/// `GtkMenuButton` is a named button wrapping an unnamed `GtkToggleButton`
/// of its own; that inner one is an implementation detail and announcing it
/// separately is not what anyone wants. So a control inside a named control
/// is left alone.
fn audit(widget: &gtk::Widget, path: &str, inside: bool, problems: &mut Vec<String>) {
    let role = widget.accessible_role();
    let here = format!("{path} > {}", widget.type_().name());
    let named = named(widget);
    let classes = widget.css_classes().join(".");

    if widget.is_visible() && !inside && !upstream_gap(widget) {
        if NEEDS_A_NAME.contains(&role) && !named {
            problems.push(format!("{here}[{classes}]: a {role:?} with no name"));
        }
        // Anything the keyboard can land on has to say something when it
        // gets there — a role, or failing that a name.
        if widget.is_focusable() && SAYS_NOTHING.contains(&role) && !named {
            problems.push(format!(
                "{here}[{classes}]: focusable, and announces nothing"
            ));
        }
    }

    let inside = inside || (named && NEEDS_A_NAME.contains(&role));
    let mut child = widget.first_child();
    while let Some(current) = child {
        audit(&current, &here, inside, problems);
        child = current.next_sibling();
    }
}

/// Widgets this application does not build and cannot name.
///
/// `AdwToastWidget` grows its own dismiss button, icon-only, carrying a
/// tooltip and no accessible label — and a tooltip is not a name: GTK leaves
/// the LABEL property undefined, verified against plain GTK outside this
/// codebase. libadwaita exposes no API for it and the button is reachable
/// only by walking into another library's internals, which would break on
/// its next release. Postio's own button on that toast — *Undo* — is named,
/// and that is the one that does something.
///
/// Kept as a list of one rather than a loosened rule, because
/// `every_exception_is_still_needed` re-checks it: when libadwaita names the
/// button, this entry stops being reached and the audit says so instead of
/// carrying a dead excuse forever.
fn upstream_gap(widget: &gtk::Widget) -> bool {
    let gap = widget
        .parent()
        .is_some_and(|parent| parent.type_().name() == "AdwToastWidget")
        && widget
            .downcast_ref::<gtk::Button>()
            .and_then(|button| button.icon_name())
            .is_some_and(|icon| icon == "window-close-symbolic");
    if gap {
        UPSTREAM_GAPS_HIT.with(|hit| hit.set(hit.get() + 1));
    }
    gap
}

thread_local! {
    /// How many times [`upstream_gap`] excused a widget, so the excuse can be
    /// shown to still be load-bearing.
    static UPSTREAM_GAPS_HIT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Fail unless GTK is actually recording accessible properties.
///
/// GTK builds a `GtkATContext` per widget lazily, and only when an
/// accessibility backend is running. Under `GTK_A11Y=none` — which is what a
/// headless session with no a11y bus gets by default — there is no context,
/// so `gtk_test_accessible_has_property` answers "not set" for every widget
/// on screen no matter what the code did. Nothing here would then be
/// measuring the application.
///
/// That is not hypothetical: it is why this file's row assertion failed
/// headless and passed on the maintainer's desktop, where at-spi is running.
/// The cause was read as a timing race for long enough to be worth a guard
/// rather than a comment.
///
/// So the harness proves it can see accessibility at all before the audit is
/// allowed to draw any conclusion, using the one widget in the tree whose
/// name is set unconditionally.
fn require_an_accessibility_backend() {
    let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    probe.update_property(&[gtk::accessible::Property::Label("probe")]);
    assert!(
        gtk::test_accessible_has_property(&probe, AccessibleProperty::Label),
        "a name set on a widget did not read back, so GTK is recording no \
         accessible properties and every assertion below would pass or fail \
         for reasons unrelated to the code. Run with GTK_A11Y=test — this \
         test sets it, so something has overridden it."
    );
}

/// Whether the accessible name of `widget` is set *and* is the empty string.
///
/// `gtk_test_accessible_has_property` answers whether a property was ever
/// set, not whether it says anything, so a widget labelled `""` reads as
/// named. That is not a hair-split: the list's own unbind path sets `""`
/// deliberately to clear a recycled row, so "named with nothing" is a state
/// this tree really reaches, and treating it as named made the row assertion
/// unable to fail at all.
///
/// GTK has no getter for a property value, only `check_property`, which
/// compares and returns NULL on a match — so the question has to be asked as
/// "is it equal to empty".
fn labelled_empty(widget: &gtk::Widget) -> bool {
    use glib::translate::ToGlibPtr;
    unsafe {
        let message = gtk4_sys::gtk_test_accessible_check_property(
            ToGlibPtr::<*mut gtk4_sys::GtkWidget>::to_glib_none(widget).0
                as *mut gtk4_sys::GtkAccessible,
            gtk4_sys::GTK_ACCESSIBLE_PROPERTY_LABEL,
            c"".as_ptr(),
        );
        if message.is_null() {
            return true;
        }
        glib::ffi::g_free(message as *mut _);
        false
    }
}

/// Whether a screen reader would have something to call this widget.
///
/// A set `Label` property or a `LabelledBy` relation is the explicit answer;
/// a control whose own child is a label is the implicit one GTK computes for
/// itself, and is just as good. An empty label is none of them.
fn named(widget: &gtk::Widget) -> bool {
    if gtk::test_accessible_has_property(widget, AccessibleProperty::Label) {
        return !labelled_empty(widget);
    }
    if gtk::test_accessible_has_relation(widget, AccessibleRelation::LabelledBy) {
        return true;
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().is_some_and(|label| !label.is_empty())
    {
        return true;
    }
    has_text(widget)
}

/// Whether any label inside `widget` carries text.
fn has_text(widget: &gtk::Widget) -> bool {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        return !label.text().is_empty();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if has_text(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

/// The `GtkListView` inside the message list pane.
fn list_view(window: &Window) -> gtk::Widget {
    fn find(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is::<gtk::ListView>() {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.list().upcast_ref::<gtk::Widget>()).expect("the list view")
}

/// The visible rows the `GtkListView` has built, whatever they announce.
///
/// Deliberately blind to naming: this is what the wait condition counts and
/// what the assertion then inspects, and if it filtered on having a name the
/// assertion below could never fail. See `pump_until`.
fn row_widgets(window: &Window) -> Vec<gtk::Widget> {
    let mut rows = Vec::new();
    let mut child = list_view(window).first_child();
    while let Some(current) = child {
        if current.is_visible() && gtk::test_accessible_has_role(&current, AccessibleRole::ListItem)
        {
            rows.push(current.clone());
        }
        child = current.next_sibling();
    }
    rows
}

/// Whether the feed's first page has actually arrived.
///
/// `n_items` alone is not it: the model reports the mailbox total as soon as
/// the *count* is known, while `row_at` still answers `None` for every
/// position until the page carrying it lands. A row bound in that window has
/// no data to speak, so waiting on the count would wait for the wrong thing.
fn page_landed(window: &Window) -> bool {
    window
        .list()
        .model()
        .item(0)
        .and_downcast::<postio_gtk::list::MessageRow>()
        .and_then(|item| item.row())
        .is_some()
}

fn row_items(window: &Window) -> usize {
    row_widgets(window).len()
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..80 {
        while context.iteration(false) {}
    }
}

/// Pump until `ready` holds, rather than for a fixed number of turns.
///
/// `pump` spins a fixed budget and hopes it was enough. Measured, that budget
/// is about 550ms of wall time on this machine, which is why substituting it
/// here still passes today and why the race postio-9112 predicted does not
/// currently reproduce — the headless failure it was filed for turned out to
/// be the missing accessibility backend instead. This is the correct shape
/// regardless: it states the precondition the assertions depend on instead of
/// leaving it to a budget nobody has re-measured since.
///
/// Bounded, so it cannot mask a failure. If the condition never holds this
/// returns anyway and the caller's assertion fails on an empty list, rather
/// than hanging until CI kills the job.
///
/// **The condition must not be the assertion.** Waiting for a *named* row and
/// then asserting the row is named is a test that cannot fail — the more
/// expensive version of this same bug, and one this file has already had.
fn pump_until(ready: impl Fn() -> bool) {
    let context = gtk::glib::MainContext::default();
    for _ in 0..2000 {
        while context.iteration(false) {}
        if ready() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
