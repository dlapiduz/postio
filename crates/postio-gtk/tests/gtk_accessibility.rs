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

use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRelation, AccessibleRole};
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
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
                    thread: None,
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

#[test]
fn every_widget_a_screen_reader_meets_has_a_role_and_a_name() {
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
    pump();

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
    let mut items = 0;
    let mut child = list_view(&window).first_child();
    while let Some(current) = child {
        if let Some(item) = current.downcast_ref::<gtk::Widget>()
            && item.is_visible()
            && gtk::test_accessible_has_role(item, AccessibleRole::ListItem)
        {
            assert!(
                named(item),
                "a visible row in the list has no name for a screen reader"
            );
            items += 1;
        }
        child = current.next_sibling();
    }
    assert!(items > 0, "no rows to check — the list never filled");

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
    let mut problems = Vec::new();
    audit(
        window.upcast_ref::<gtk::Widget>(),
        "window",
        false,
        &mut problems,
    );
    assert!(
        problems.is_empty(),
        "{} widget(s) a screen reader cannot use:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );

    window.destroy();
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

    if widget.is_visible() && !inside {
        if NEEDS_A_NAME.contains(&role) && !named {
            problems.push(format!("{here}: a {role:?} with no name"));
        }
        // Anything the keyboard can land on has to say something when it
        // gets there — a role, or failing that a name.
        if widget.is_focusable() && SAYS_NOTHING.contains(&role) && !named {
            problems.push(format!("{here}: focusable, and announces nothing"));
        }
    }

    let inside = inside || (named && NEEDS_A_NAME.contains(&role));
    let mut child = widget.first_child();
    while let Some(current) = child {
        audit(&current, &here, inside, problems);
        child = current.next_sibling();
    }
}

/// Whether a screen reader would have something to call this widget.
///
/// A set `Label` property or a `LabelledBy` relation is the explicit answer;
/// a control whose own child is a label is the implicit one GTK computes for
/// itself, and is just as good.
fn named(widget: &gtk::Widget) -> bool {
    if gtk::test_accessible_has_property(widget, AccessibleProperty::Label)
        || gtk::test_accessible_has_relation(widget, AccessibleRelation::LabelledBy)
    {
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

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..80 {
        while context.iteration(false) {}
    }
}
