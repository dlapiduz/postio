//! The message list's row: canvas 1b's anatomy, drawn in one `snapshot()`.
//!
//! # Why this widget draws itself
//!
//! The row is the most-rendered thing in the application: a mailbox is tens
//! of thousands of them and the budget for an ordinary interaction is 16ms.
//! A row assembled from nested `GtkBox`es costs a dozen widgets each — CSS
//! nodes, measure and allocate passes, snapshot recursion — and that is the
//! usual reason a GTK list feels sluggish under the finger. This one is a
//! single `GtkWidget` with a hand-written `snapshot` and no children that
//! are ever laid out, so scrolling costs text and rectangles and nothing
//! else.
//!
//! # Where its colours and type come from
//!
//! Drawing by hand gives up the thing a widget tree gets for free: the
//! cascade. `crate::row` must not answer that by hard-coding steel and
//! Barlow, or dark mode and high contrast would quietly stop working here
//! while working everywhere else.
//!
//! So the row keeps one invisible probe label as a child and *reads* the
//! style off it: for each role — sender, subject, snippet, time, avatar —
//! it sets the probe's classes, asks GTK for the resolved colour and font,
//! and paints with those. Every value is still a token in `shell.css`,
//! every scheme still cascades, and the probe is never measured, allocated
//! or drawn. [`Palette`] is the result, recomputed only when the style
//! actually changes.
//!
//! # The anatomy
//!
//! Canvas 1b, left to right and top to bottom: an avatar chip of initials,
//! then a column carrying sender · thread-count badge · attachment ·
//! time, the subject, the snippet, and — on the focused row only — the key
//! hints that teach the keyboard. Unread is the canvas' own treatment,
//! weight and full-strength ink rather than a dot.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{DateTime, Datelike, Local, Utc};
use gtk::{gdk, glib, graphene, gsk, pango};
use postio_config::Density;
use postio_core::{CommandId, Keymap};
use postio_model::address::EmailAddress;

use crate::list::Row;

/// The commands the focused row hints at, and the labels the canvas gives
/// them — canvas order, not registry order.
const HINT_COMMANDS: [(CommandId, &str); 3] = [
    (CommandId::Reply, "reply"),
    (CommandId::Archive, "archive"),
    (CommandId::Thread, "thread"),
];

/// The hints for a keymap alone, `Thread` included: the rebinding behaviour
/// [`hints_for_row`] shares with every entry in [`HINT_COMMANDS`], tested
/// here without a row to keep that half of the contract separate from the
/// thread-count filter.
#[cfg(test)]
fn hints_for(keymap: &Keymap) -> Vec<(String, &'static str)> {
    filtered_hints(keymap, true)
}

/// [`hints_for`], minus `Thread` when the row has nothing to thread — the
/// same test the badge uses (`thread_count > 1`, in `build`), so the badge
/// and the hint can never disagree. A row that has not arrived yet (`None`)
/// has nothing to thread either.
fn hints_for_row(keymap: &Keymap, row: Option<&Row>) -> Vec<(String, &'static str)> {
    filtered_hints(keymap, row.is_some_and(|row| row.thread_count > 1))
}

fn filtered_hints(keymap: &Keymap, threaded: bool) -> Vec<(String, &'static str)> {
    HINT_COMMANDS
        .iter()
        .filter(|(command, _)| threaded || *command != CommandId::Thread)
        .filter_map(|(command, label)| {
            keymap
                .binding(*command)
                .map(|key| (key.to_string(), *label))
        })
        .collect()
}

/// [`hints_for`] against the registry's own bindings, for the tests that
/// check a rebind reaches the hint without caring what the row's default is.
#[cfg(test)]
fn default_hints() -> Vec<(String, &'static str)> {
    hints_for(&Keymap::resolve(&Default::default()))
}

/// The initials the avatar chip shows for `from`.
///
/// Two letters: the initials of the first two words of a display name, or
/// the first two letters of a single word. With no display name the local
/// part stands in, which is what makes a mailing list read as `LK` rather
/// than as a shrug.
pub fn initials(from: Option<&EmailAddress>) -> String {
    let Some(from) = from else {
        return "?".to_string();
    };
    let source = match &from.name {
        Some(name) if !name.trim().is_empty() => name.as_str(),
        _ => from.local_part().unwrap_or(""),
    };
    let words: Vec<&str> = source
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => return "?".to_string(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    letters.to_uppercase()
}

/// The timestamp column: relative for today, absolute beyond.
///
/// Canvas 1b draws `09:14` and `Thu`. Past the week it becomes a date, and
/// past the year it carries the year, because "12 Aug" two years ago is a
/// lie the eye believes.
pub fn timestamp(received: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = received.with_timezone(&now.timezone());
    let days = (now.date_naive() - local.date_naive()).num_days();
    match days {
        0 => local.format("%H:%M").to_string(),
        1..=6 => local.format("%a").to_string(),
        _ if local.year() == now.year() => local.format("%-d %b").to_string(),
        _ => local.format("%-d %b %y").to_string(),
    }
}

/// What a screen reader says for `row`.
///
/// One sentence carrying everything the row draws, in reading order, so the
/// keyboard and the eye learn the same list. Nothing here is decorative: a
/// badge or a paperclip that is only a picture is a row a screen reader
/// cannot triage.
pub fn accessible_label(row: &Row) -> String {
    let mut parts = Vec::new();
    if !row.seen {
        parts.push("Unread".to_string());
    }
    if row.flagged {
        parts.push("Flagged".to_string());
    }
    if row.draft {
        parts.push("Draft".to_string());
    }
    // A thread row is a conversation, and a screen reader has to hear that
    // before it hears a name — otherwise "from Ada, 6 in thread" reads as a
    // message from Ada that happens to have replies.
    if row.is_thread() {
        parts.push(format!(
            "conversation with {}",
            participants_line(&row.participants)
        ));
    } else {
        parts.push(format!(
            "from {}",
            row.from
                .as_ref()
                .map(|from| from.display().to_string())
                .unwrap_or_else(|| "unknown sender".to_string())
        ));
    }
    parts.push(
        row.subject
            .clone()
            .filter(|subject| !subject.trim().is_empty())
            .unwrap_or_else(|| "no subject".to_string()),
    );
    parts.push(timestamp(row.received_at, Local::now()));
    if row.thread_count > 1 {
        parts.push(format!("{} in thread", row.thread_count));
    }
    if row.answered {
        parts.push("Answered".to_string());
    }
    if row.has_attachments {
        parts.push("has an attachment".to_string());
    }
    parts.join(", ")
}

/// Canvas 1b's row geometry for one density, in logical pixels.
///
/// Type and colour come from the cascade ([`Palette`]); this is the layout
/// the snapshot arranges them in, which a hand-drawn widget owns the way a
/// `GtkBox` owns its spacing. The airy numbers are measured straight off the
/// canvas; the other two tighten the same anatomy rather than changing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Space above and below the row's content.
    pub pad_y: f32,
    /// How far in the content starts, accent edge included, so a row does
    /// not shift sideways when the selection lands on it.
    pub inset: f32,
    /// The avatar chip, square.
    pub avatar: f32,
    /// Between the avatar and the text column.
    pub gap: f32,
    /// Between the sender line and the subject.
    pub subject_gap: f32,
    /// Between the snippet and the key hints the focused row reveals.
    pub hints_gap: f32,
    /// Whether the snippet line is drawn at all.
    pub snippet: bool,
}

impl Metrics {
    /// The geometry `density` asks for.
    pub fn for_density(density: Density) -> Self {
        match density {
            Density::Airy => Metrics {
                pad_y: 11.0,
                inset: 21.0,
                avatar: 30.0,
                gap: 12.0,
                subject_gap: 3.0,
                hints_gap: 7.0,
                snippet: true,
            },
            Density::Comfortable => Metrics {
                pad_y: 8.0,
                inset: 18.0,
                avatar: 26.0,
                gap: 10.0,
                subject_gap: 2.0,
                hints_gap: 5.0,
                snippet: true,
            },
            // The tightest setting is for triage, where the question is how
            // many subjects fit on screen. The snippet is the line that
            // costs the most and answers it least.
            Density::Compact => Metrics {
                pad_y: 5.0,
                inset: 15.0,
                avatar: 22.0,
                gap: 9.0,
                subject_gap: 1.0,
                hints_gap: 4.0,
                snippet: false,
            },
        }
    }
}

/// One role's resolved paint: what the cascade says this text is.
#[derive(Clone, Debug)]
struct Ink {
    color: gdk::RGBA,
    font: pango::FontDescription,
}

/// Which of the four variants a role is drawn in.
///
/// Selected and focused are different states — selection is what an action
/// will hit, focus is where the keyboard is — and only selection changes the
/// ink. Focus changes what the row *reveals*.
fn tone(selected: bool, unread: bool) -> usize {
    usize::from(selected) * 2 + usize::from(unread)
}

/// The rule a row draws above itself, if any.
///
/// The ordinary hairline reads fine against a plain ground; against a
/// selected or cursor row's own tint it can wash out, in exactly the schemes
/// where the tint and the hairline sit closest (canvas 3c: the checked and
/// cursor grounds are a step from the hairline in light, and share dark's
/// palette almost exactly). [`Edge::Strong`] is the higher-contrast rule for
/// that ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Hairline,
    Strong,
}

/// Which [`Edge`] a row draws above itself.
///
/// `None` above the first row: the list header draws that boundary itself
/// (`.postio-list-header`'s own bottom rule), on purpose, so it holds
/// whether or not the first row happens to be selected — a boundary that
/// only shows up sometimes is not a boundary.
fn top_edge(selected: bool, cursor: bool, first: bool) -> Option<Edge> {
    if first {
        return None;
    }
    Some(if selected || cursor {
        Edge::Strong
    } else {
        Edge::Hairline
    })
}

/// Everything the snapshot paints with, read off the cascade.
///
/// Built from an invisible probe label rather than written down here, so a
/// scheme change moves this widget exactly as far as it moves every other
/// one. See the module docs.
struct Palette {
    sender: [Ink; 4],
    subject: [Ink; 4],
    snippet: [Ink; 4],
    time: [Ink; 4],
    avatar: [Ink; 4],
    badge: Ink,
    hint: Ink,
    key: Ink,
    hairline: gdk::RGBA,
    /// The higher-contrast rule [`top_edge`] picks over a selected or cursor
    /// row's own tint.
    hairline_strong: gdk::RGBA,
    selected_edge: gdk::RGBA,
    key_edge: gdk::RGBA,
    selected_bg: gdk::RGBA,
    /// The ground under a row that is in the selection, as against the one
    /// the cursor is on.
    checked_bg: gdk::RGBA,
    /// The check itself, on the accent chip it sits in.
    checked_mark: gdk::RGBA,
    hover_bg: gdk::RGBA,
    unread_chip: gdk::RGBA,
    clip: Option<gtk::IconPaintable>,
    check: Option<gtk::IconPaintable>,
    /// The persistent flagged/answered/draft marks, at the paperclip's size
    /// — a status mark, not a hover target, so it stays the small one.
    flag_mark: Option<gtk::IconPaintable>,
    answered_mark: Option<gtk::IconPaintable>,
    draft_mark: Option<gtk::IconPaintable>,
    /// The hover actions, in [`RowAction::ALL`] order, with the flag glyph
    /// in both of its states.
    archive: Option<gtk::IconPaintable>,
    flagged: Option<gtk::IconPaintable>,
    unflagged: Option<gtk::IconPaintable>,
    trash: Option<gtk::IconPaintable>,
}

impl Palette {
    /// Read every role off `probe`, whose parent puts it under the same
    /// `:root` classes the rest of the window is under.
    fn read(probe: &gtk::Label) -> Self {
        let ink = |classes: &[&str]| {
            probe.set_css_classes(classes);
            Ink {
                color: probe.color(),
                font: probe
                    .create_pango_context()
                    .font_description()
                    .unwrap_or_default(),
            }
        };
        let four = |role: &str| {
            [
                ink(&[role]),
                ink(&[role, "unread"]),
                ink(&[role, "selected"]),
                ink(&[role, "selected", "unread"]),
            ]
        };
        let paint = |classes: &[&str]| {
            probe.set_css_classes(classes);
            probe.color()
        };

        let palette = Palette {
            sender: four("postio-row-sender"),
            subject: four("postio-row-subject"),
            snippet: four("postio-row-snippet"),
            time: four("postio-row-time"),
            avatar: four("postio-row-avatar"),
            badge: ink(&["postio-row-badge"]),
            hint: ink(&["postio-row-hint"]),
            key: ink(&["postio-key"]),
            hairline: paint(&["postio-row-edge", "hairline"]),
            hairline_strong: paint(&["postio-row-edge", "hairline", "strong"]),
            selected_edge: paint(&["postio-row-edge", "selected"]),
            key_edge: paint(&["postio-row-edge", "key"]),
            selected_bg: paint(&["postio-row-ground", "selected"]),
            checked_bg: paint(&["postio-row-ground", "checked"]),
            checked_mark: paint(&["postio-row-ground", "check-mark"]),
            hover_bg: paint(&["postio-row-ground", "hover"]),
            unread_chip: paint(&["postio-row-ground", "unread"]),
            clip: probe.display().pipe_icon("mail-attachment-symbolic"),
            check: probe.display().pipe_icon("object-select-symbolic"),
            flag_mark: probe.display().pipe_icon("starred-symbolic"),
            answered_mark: probe.display().pipe_icon("mail-replied-symbolic"),
            draft_mark: probe.display().pipe_icon("document-edit-symbolic"),
            archive: probe.display().action_icon(RowAction::Archive.icon(false)),
            flagged: probe.display().action_icon(RowAction::Flag.icon(true)),
            unflagged: probe.display().action_icon(RowAction::Flag.icon(false)),
            trash: probe.display().action_icon(RowAction::Delete.icon(false)),
        };
        probe.set_css_classes(&[]);
        palette
    }
}

/// The attachment paperclip, from the icon theme.
trait IconLookup {
    fn pipe_icon(&self, name: &str) -> Option<gtk::IconPaintable>;

    /// A hover action's glyph, at the size one is drawn.
    fn action_icon(&self, name: &str) -> Option<gtk::IconPaintable>;
}

impl IconLookup for gdk::Display {
    fn pipe_icon(&self, name: &str) -> Option<gtk::IconPaintable> {
        Some(gtk::IconTheme::for_display(self).lookup_icon(
            name,
            &[],
            CLIP,
            1,
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        ))
    }

    fn action_icon(&self, name: &str) -> Option<gtk::IconPaintable> {
        Some(gtk::IconTheme::for_display(self).lookup_icon(
            name,
            &[],
            ACTION as i32,
            1,
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        ))
    }
}

/// The attachment glyph's box, square.
const CLIP: i32 = 12;

/// A hover action's glyph box, square.
///
/// Bigger than the paperclip because this one is a target, not a mark: 16px
/// of glyph inside a row that is at least 40px tall gives a hit area a mouse
/// finds without aiming.
const ACTION: f32 = 16.0;

/// Between one hover action and the next.
const ACTION_GAP: f32 = 10.0;

/// What the row offers under the pointer, in the order they are drawn.
///
/// The three verbs triage is made of, and the same three the bulk bar
/// carries — one row or twenty, the mouse says the same thing. Each is a
/// registry command, never a local implementation: `a`, `s` and `d` mean
/// exactly this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    /// Archive this message — `a`.
    Archive,
    /// Flag or unflag it — `s`.
    Flag,
    /// Move it to the trash — `d`.
    Delete,
}

impl RowAction {
    /// Every action, left to right.
    pub const ALL: [RowAction; 3] = [RowAction::Archive, RowAction::Flag, RowAction::Delete];

    /// The icon that says what it does.
    fn icon(self, flagged: bool) -> &'static str {
        match self {
            RowAction::Archive => "postio-archive-symbolic",
            // The state, not the verb: a flagged message offers to unflag,
            // and the glyph has to say which way it would go.
            RowAction::Flag if flagged => "starred-symbolic",
            RowAction::Flag => "non-starred-symbolic",
            RowAction::Delete => "user-trash-symbolic",
        }
    }

    /// What a screen reader would call it, for whoever offers it another way.
    pub fn title(self) -> &'static str {
        match self {
            RowAction::Archive => "Archive",
            RowAction::Flag => "Flag",
            RowAction::Delete => "Delete",
        }
    }
}

/// Between the pieces on the sender line.
const RUN: f32 = 8.0;

/// The accent edge a selected row wears, and the hairline between rows.
const EDGE: f32 = 3.0;

/// A key cap's padding and corner, matching `.postio-key`.
const CAP: (f32, f32, f32) = (4.0, 1.0, 2.0);

/// The laid-out row: the pango layouts and where they go.
///
/// Rebuilt whenever the data, the width, the state or the style changes,
/// which on a scrolling list means once per row as it comes into view —
/// the same cost any list pays, and nothing per frame.
struct Laid {
    avatar: pango::Layout,
    sender: pango::Layout,
    time: pango::Layout,
    badge: Option<pango::Layout>,
    subject: pango::Layout,
    snippet: Option<pango::Layout>,
    hints: Vec<(pango::Layout, pango::Layout)>,
    /// Baseline of the sender line, from the top of the content.
    line1: f32,
    /// Top of each of the following lines, from the top of the content.
    subject_y: f32,
    snippet_y: f32,
    hints_y: f32,
    height: f32,
    width: i32,
    tone: usize,
    focused: bool,
}

mod imp {
    use super::*;

    pub struct MessageRowView {
        pub(super) row: RefCell<Option<Row>>,
        pub(super) density: Cell<Density>,
        /// The live keymap the focused row's key hints are generated from,
        /// together with `row` — see [`super::MessageRowView::hints`]. Kept
        /// as the keymap rather than the derived hints so a row change picks
        /// up the same hints a keymap change would, with no ordering
        /// dependency between `set_row` and `set_keymap` on bind.
        pub(super) keymap: RefCell<Keymap>,
        pub(super) first: Cell<bool>,
        /// Whether an action would hit this row.
        pub(super) selected: Cell<bool>,
        /// Whether this is the row the keyboard is *on* — the list's cursor,
        /// which `GtkSingleSelection` calls its selection and this widget
        /// does not. See [`crate::selection`] for why the two are separate.
        pub(super) cursor: Cell<bool>,
        /// Where this row sits in the list, for turning a click into a range.
        pub(super) index: Cell<u32>,
        /// Whether the row offers its actions under the pointer at all —
        /// `[ui].show_hover_actions`.
        pub(super) actions: Cell<bool>,
        /// Whether the focused row may reveal its key hints at all —
        /// `[ui].show_key_hints`. Every binding stays in force either way;
        /// this only stops the row from naming them (#422).
        pub(super) hints_enabled: Cell<bool>,
        pub(super) hovered: Cell<bool>,
        /// Whether the keyboard is on this row.
        ///
        /// Stored rather than asked for, because `measure` reads it and a
        /// measurement that depends on where the focus happens to be *during*
        /// a layout pass is one that changes between passes. GTK's size
        /// negotiation does not converge on that, and a list that will not
        /// converge simply stops painting.
        pub(super) focused: Cell<bool>,
        /// The invisible label the palette is read off. Never measured,
        /// never allocated, never drawn — only asked what the cascade says.
        pub(super) probe: gtk::Label,
        /// A second probe, permanently wearing one class, whose colour is
        /// read every draw. gtk4-rs does not expose `css_changed`, and the
        /// scheme can move without this widget being touched at all — so
        /// the row watches one token that differs in every scheme (light,
        /// dark, and each in high contrast) and rebuilds when it moves.
        /// One resolved-style read per draw, off a node whose style is
        /// already valid.
        pub(super) sentinel: gtk::Label,
        pub(super) palette: RefCell<Option<Rc<Palette>>>,
        pub(super) laid: RefCell<Option<Laid>>,
        /// The window whose focus this row is watching, and the handler
        /// watching it.
        pub(super) watch: RefCell<Option<(gtk::Window, glib::SignalHandlerId)>>,
    }

    impl Default for MessageRowView {
        fn default() -> Self {
            MessageRowView {
                row: RefCell::new(None),
                density: Cell::new(Density::default()),
                keymap: RefCell::new(Keymap::resolve(&Default::default())),
                first: Cell::new(false),
                selected: Cell::new(false),
                cursor: Cell::new(false),
                index: Cell::new(0),
                actions: Cell::new(true),
                hints_enabled: Cell::new(true),
                hovered: Cell::new(false),
                focused: Cell::new(false),
                probe: gtk::Label::new(None),
                sentinel: gtk::Label::new(None),
                palette: RefCell::new(None),
                laid: RefCell::new(None),
                watch: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageRowView {
        const NAME: &'static str = "PostioMessageRowView";
        type Type = super::MessageRowView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for MessageRowView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_css_class("postio-row");
            // Focusable so the widget works on its own — in a test, a bench,
            // or any surface that is not a `GtkListView`. Inside one,
            // `crate::list_view` turns this off: there the list item takes
            // the keyboard, and `shows_hints` follows it up the tree.
            obj.set_focusable(true);
            // The picture of a row, not the row: `GtkListItemWidget` around
            // it carries the `ListItem` role and the name, because that is
            // what a screen reader navigates the list by. Two nested list
            // items would be one more than there are rows.
            obj.set_accessible_role(gtk::AccessibleRole::Presentation);

            for probe in [&self.probe, &self.sentinel] {
                probe.set_child_visible(false);
                probe.set_parent(&*obj);
            }
            self.sentinel
                .set_css_classes(&["postio-row-edge", "hairline"]);

            // Hover is watched here rather than read off a state flag: the
            // pointer is over the list item, not over this widget, and the
            // prelight never reaches down. `postio-8ge` builds the rest of
            // mouse parity on top of this.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(
                #[weak(rename_to = row)]
                obj,
                move |_, _, _| row.set_hovered(true)
            ));
            motion.connect_leave(glib::clone!(
                #[weak(rename_to = row)]
                obj,
                move |_| row.set_hovered(false)
            ));
            obj.add_controller(motion);
        }

        fn dispose(&self) {
            self.probe.unparent();
            self.sentinel.unparent();
        }
    }

    impl WidgetImpl for MessageRowView {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let obj = self.obj();
            let metrics = Metrics::for_density(self.density.get());
            if orientation == gtk::Orientation::Horizontal {
                // A row never asks for more width than it is given: the
                // subject and the snippet ellipsize instead. What it does
                // insist on is room for the avatar and a few characters.
                let least = (metrics.inset * 2.0 + metrics.avatar + metrics.gap) as i32 + 96;
                return (least, least, -1, -1);
            }
            let width = if for_size > 0 { for_size } else { 360 };
            let height = obj.lay_out(width).height.ceil() as i32;
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            // The probe is not part of the layout; touching the width is
            // enough to notice that the text has to be re-ellipsized.
            let stale = self
                .laid
                .borrow()
                .as_ref()
                .is_none_or(|laid| laid.width != width);
            if stale {
                self.laid.replace(None);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }

        /// Text scaling, the font, the display. All of them change what the
        /// probe would say, so nothing read off it survives.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.obj().restyle();
        }

        /// Selection, hover and focus all change what the row draws, and
        /// focus changes how tall it is.
        fn state_flags_changed(&self, previous: &gtk::StateFlags) {
            self.parent_state_flags_changed(previous);
            self.laid.replace(None);
            self.obj().queue_resize();
            self.obj().refresh_focus();
        }

        /// The focused row is the row the keyboard is on, which is a fact
        /// about the window rather than about this widget — and one no
        /// state flag on this widget reports while the window is not the
        /// active one. So the row watches the window's focus directly, and
        /// keeps showing its hints when you alt-tab away and come back.
        fn root(&self) {
            self.parent_root();
            let obj = self.obj();
            let Some(window) = obj.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let id = window.connect_notify_local(
                Some("focus-widget"),
                glib::clone!(
                    #[weak]
                    obj,
                    move |_, _| obj.refresh_focus()
                ),
            );
            self.watch.replace(Some((window, id)));
        }

        fn unroot(&self) {
            if let Some((window, id)) = self.watch.take() {
                window.disconnect(id);
            }
            self.parent_unroot();
        }
    }
}

glib::wrapper! {
    /// One message in the list, drawn in a single `snapshot()`.
    pub struct MessageRowView(ObjectSubclass<imp::MessageRowView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MessageRowView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MessageRowView {
    /// An empty row, waiting to be bound.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show `row`, or a skeleton while its page is still on its way.
    pub fn set_row(&self, row: Option<Row>) {
        let imp = self.imp();
        if *imp.row.borrow() == row {
            return;
        }
        imp.row.replace(row);
        self.update_property(&[gtk::accessible::Property::Label(&self.spoken())]);
        imp.laid.replace(None);
        self.queue_resize();
    }

    /// The message currently on screen, if any.
    pub fn row(&self) -> Option<Row> {
        self.imp().row.borrow().clone()
    }

    /// The sentence a screen reader is given for this row.
    ///
    /// Exactly what `set_row` pushes as the accessible label. Public because
    /// gtk4-rs offers no way to read a property back, and "the row announces
    /// what it draws" is not a promise worth leaving untested.
    pub fn spoken(&self) -> String {
        self.imp()
            .row
            .borrow()
            .as_ref()
            .map(accessible_label)
            .unwrap_or_default()
    }

    /// Which of the three row heights this row is drawn at.
    pub fn set_density(&self, density: Density) {
        if self.imp().density.replace(density) != density {
            self.imp().laid.replace(None);
            self.queue_resize();
        }
    }

    /// The density currently in force.
    pub fn density(&self) -> Density {
        self.imp().density.get()
    }

    /// The key and label pairs the focused row would reveal right now.
    ///
    /// A function of both the keymap and the row: `Thread` drops out when
    /// the row has nothing to thread (`thread_count <= 1`), matching the
    /// badge's own test so the two can never disagree.
    ///
    /// Public for the same reason [`Self::spoken`] is: a hint the row draws
    /// but nothing can read back is a hint nothing can prove correct.
    pub fn hints(&self) -> Vec<(String, &'static str)> {
        let imp = self.imp();
        hints_for_row(&imp.keymap.borrow(), imp.row.borrow().as_ref())
    }

    /// Regenerate the focused row's key hints from the live keymap.
    ///
    /// A rebind changes what the hints say without a restart, the same
    /// promise `postio-gtk::config` already keeps for the resolver, the
    /// palette and the cheat sheet.
    pub fn set_keymap(&self, keymap: &Keymap) {
        let imp = self.imp();
        if *imp.keymap.borrow() != *keymap {
            imp.keymap.replace(keymap.clone());
            imp.laid.replace(None);
            self.queue_resize();
        }
    }

    /// Mark the row as part of the selection — what an action will hit.
    ///
    /// Selection lives on the `GtkListItem` around the row and its state
    /// flag never reaches down here, so `crate::list_view` passes it in.
    /// That is not a workaround: selection is a fact about the *model*, and
    /// a row reading it off its own widget state would be reading a copy.
    pub fn set_selected(&self, selected: bool) {
        if self.imp().selected.replace(selected) != selected {
            self.imp().laid.replace(None);
            self.queue_draw();
        }
    }

    /// Whether an action would hit this row.
    pub fn is_selected(&self) -> bool {
        self.imp().selected.get()
    }

    /// Mark the row as the one the keyboard is on.
    ///
    /// `GtkSingleSelection`'s "selected" item, which is this list's *cursor*
    /// and not its selection — see [`crate::selection`]. It is a different
    /// fact with a different treatment: the cursor wears canvas 1b's tint
    /// and steel edge, a selected row wears a check.
    pub fn set_cursor(&self, cursor: bool) {
        if self.imp().cursor.replace(cursor) != cursor {
            self.imp().laid.replace(None);
            self.queue_draw();
        }
    }

    /// Whether the keyboard is on this row.
    pub fn is_cursor(&self) -> bool {
        self.imp().cursor.get()
    }

    /// Where this row sits in the list.
    ///
    /// Handed down on bind, because a click has to become a *position* — a
    /// range runs from one place in the list to another — and a widget that
    /// recycles through a hundred rows cannot be asked which one it is
    /// currently drawing.
    pub fn set_index(&self, index: u32) {
        self.imp().index.set(index);
    }

    /// Where this row sits in the list.
    pub fn index(&self) -> u32 {
        self.imp().index.get()
    }

    /// Whether the row offers its actions when the pointer is over it.
    ///
    /// `[ui].show_hover_actions`. Off means the timestamp keeps its place and
    /// the mouse reaches the same verbs through the context menu — never
    /// through nothing.
    pub fn set_show_actions(&self, show: bool) {
        if self.imp().actions.replace(show) != show {
            self.queue_draw();
        }
    }

    /// Whether the actions are being offered right now: the pointer is over
    /// the row, the row has something in it, and the setting allows it.
    pub fn offers_actions(&self) -> bool {
        self.imp().actions.get() && self.imp().hovered.get() && self.imp().row.borrow().is_some()
    }

    /// Which action is under `(x, y)`, in this row's coordinates.
    ///
    /// `None` when the actions are not being offered, or when the point is
    /// somewhere else on the row.
    pub fn action_at(&self, x: f64, y: f64) -> Option<RowAction> {
        if !self.offers_actions() {
            return None;
        }
        let (x, y) = (x as f32, y as f32);
        let metrics = Metrics::for_density(self.imp().density.get());
        let line = metrics.pad_y + metrics.avatar / 2.0;
        if !(line - ACTION..line + ACTION).contains(&y) {
            return None;
        }
        RowAction::ALL
            .into_iter()
            .enumerate()
            .find(|(slot, _)| {
                let left = self.action_x(*slot as f32, metrics.inset);
                (left..left + ACTION).contains(&x)
            })
            .map(|(_, action)| action)
    }

    /// Where the glyph in `slot` starts, counting from the row's right edge
    /// so the set stays put as the row grows.
    fn action_x(&self, slot: f32, inset: f32) -> f32 {
        let right = self.width() as f32 - inset;
        let total =
            ACTION * RowAction::ALL.len() as f32 + ACTION_GAP * (RowAction::ALL.len() as f32 - 1.0);
        right - total + slot * (ACTION + ACTION_GAP)
    }

    /// Whether `(x, y)`, in this row's coordinates, is inside the square the
    /// avatar and the check share.
    ///
    /// The check is a click target, so where it is has to be answerable — and
    /// only this widget knows, because only this widget lays the row out.
    pub fn is_in_check(&self, x: f64, y: f64) -> bool {
        let metrics = Metrics::for_density(self.imp().density.get());
        let (x, y) = (x as f32, y as f32);
        (metrics.inset..metrics.inset + metrics.avatar).contains(&x)
            && (metrics.pad_y..metrics.pad_y + metrics.avatar).contains(&y)
    }

    /// Whether the pointer is over this row.
    pub fn set_hovered(&self, hovered: bool) {
        if self.imp().hovered.replace(hovered) != hovered {
            self.queue_draw();
        }
    }

    /// Whether this is the first row in the list, which draws no rule above
    /// itself because there is nothing above it to be ruled off from.
    pub fn set_first(&self, first: bool) {
        if self.imp().first.replace(first) != first {
            self.queue_draw();
        }
    }

    /// Whether the row may reveal key hints at all.
    ///
    /// `[ui].show_key_hints`. Off means no row ever shows one, focused or
    /// not — every binding still works, this only stops the row from
    /// naming it, for someone who already knows the keyboard (#422).
    pub fn set_show_key_hints(&self, show: bool) {
        if self.imp().hints_enabled.replace(show) != show {
            self.queue_draw();
        }
    }

    /// Whether the key hints are showing — the focused row, and only it.
    ///
    /// `is_focus` rather than `has_focus`: the question is which row the
    /// keyboard would act on, and that stays true while the window is in the
    /// background. A row that forgot its hints on alt-tab would be teaching
    /// the keyboard only while you were not using it.
    pub fn shows_hints(&self) -> bool {
        self.imp().hints_enabled.get() && self.holds_keyboard() && self.imp().row.borrow().is_some()
    }

    /// Whether the keyboard is on this row, asked rather than remembered.
    ///
    /// Either place counts: inside a `GtkListView` the focus lands on the
    /// list item wrapping this widget, and anywhere else — a test, a bench,
    /// the row used as a plain widget — on the widget itself.
    ///
    /// Live, because the remembered answer could be another row's (#753).
    /// The signals that maintain it fire when focus *moves*, and recycling
    /// is the case where focus did not move and the row underneath the
    /// widget changed: scrolling hands a widget that was on the focused row
    /// to a row with no claim on it, and the stale flag rode along. Asking
    /// costs two pointer comparisons, which is cheaper than the bookkeeping
    /// that would keep a cache honest through recycling.
    fn holds_keyboard(&self) -> bool {
        self.is_focus() || self.parent().is_some_and(|parent| parent.is_focus())
    }

    /// Note that the keyboard has arrived or left, and re-lay the row.
    ///
    /// [`shows_hints`](Self::shows_hints) reads the live answer, so this
    /// exists for its side effect rather than for the flag: the hints take
    /// vertical space, so a row that gains or loses them has to be measured
    /// again. The remembered value is only how that change is detected.
    fn refresh_focus(&self) {
        let focused = self.holds_keyboard();
        if self.imp().focused.replace(focused) == focused {
            return;
        }
        self.imp().laid.replace(None);
        self.queue_resize();
    }

    /// The row's height at the width it has, for a test that wants to check
    /// the three densities against each other.
    pub fn measured_height(&self, width: i32) -> f32 {
        self.lay_out(width).height
    }

    /// Throw away everything read off the cascade and measure again.
    fn restyle(&self) {
        let imp = self.imp();
        imp.palette.replace(None);
        imp.laid.replace(None);
        self.queue_resize();
    }

    fn palette(&self) -> Rc<Palette> {
        let imp = self.imp();
        let hairline = imp.sentinel.color();
        if let Some(palette) = imp.palette.borrow().clone()
            && palette.hairline == hairline
        {
            return palette;
        }
        imp.laid.replace(None);
        let palette = Rc::new(Palette::read(&imp.probe));
        imp.palette.replace(Some(palette.clone()));
        palette
    }

    /// Lay the row out for `width`, reusing the last one where it still
    /// holds. Returns a summary; the layouts themselves stay cached.
    fn lay_out(&self, width: i32) -> Summary {
        let imp = self.imp();
        let focused = self.shows_hints();
        let selected = imp.selected.get();
        let unread = imp.row.borrow().as_ref().is_some_and(|row| !row.seen);
        let tone = tone(selected, unread);

        if let Some(laid) = imp.laid.borrow().as_ref()
            && laid.width == width
            && laid.tone == tone
            && laid.focused == focused
        {
            return Summary {
                height: laid.height,
            };
        }

        let palette = self.palette();
        let metrics = Metrics::for_density(imp.density.get());
        let laid = self.build(width, &metrics, &palette, tone, focused);
        let height = laid.height;
        imp.laid.replace(Some(laid));
        Summary { height }
    }

    fn build(
        &self,
        width: i32,
        metrics: &Metrics,
        palette: &Palette,
        tone: usize,
        focused: bool,
    ) -> Laid {
        let context = self.pango_context();
        let line = |ink: &Ink, text: &str| {
            let layout = pango::Layout::new(&context);
            layout.set_font_description(Some(&ink.font));
            layout.set_text(text);
            layout
        };
        let row = self.imp().row.borrow().clone();
        let unread = row.as_ref().is_some_and(|row| !row.seen);

        let column_x = metrics.inset + metrics.avatar + metrics.gap;
        let column = (width as f32 - column_x - metrics.inset).max(40.0);

        // The sender line, right to left: the time is never elided, the
        // badge and the paperclip take what they need, and the sender gets
        // whatever is left.
        let time = line(
            &palette.time[tone],
            &row.as_ref()
                .map(|row| timestamp(row.received_at, Local::now()))
                .unwrap_or_default(),
        );
        let badge = row
            .as_ref()
            .filter(|row| row.thread_count > 1)
            .map(|row| line(&palette.badge, &row.thread_count.to_string()));
        let mut taken = time.pixel_size().0 as f32 + RUN;
        if let Some(badge) = &badge {
            taken += badge.pixel_size().0 as f32 + CAP.0 * 2.0 + RUN;
        }
        if row.as_ref().is_some_and(|row| row.has_attachments) {
            taken += CLIP as f32 + RUN;
        }
        if row.as_ref().is_some_and(|row| row.draft) {
            taken += CLIP as f32 + RUN;
        }
        if row.as_ref().is_some_and(|row| row.answered) {
            taken += CLIP as f32 + RUN;
        }
        if row.as_ref().is_some_and(|row| row.flagged) {
            taken += CLIP as f32 + RUN;
        }

        let sender = line(
            &palette.sender[tone],
            row.as_ref()
                .map(initials_source)
                .unwrap_or_default()
                .as_str(),
        );
        sender.set_ellipsize(pango::EllipsizeMode::End);
        sender.set_width(((column - taken).max(24.0) * pango::SCALE as f32) as i32);

        let elide = |ink: &Ink, text: &str| {
            let layout = line(ink, text);
            layout.set_ellipsize(pango::EllipsizeMode::End);
            layout.set_width((column * pango::SCALE as f32) as i32);
            layout
        };
        let subject = elide(
            &palette.subject[tone],
            row.as_ref()
                .and_then(|row| row.subject.clone())
                .filter(|subject| !subject.trim().is_empty())
                .unwrap_or_else(|| "(no subject)".to_string())
                .as_str(),
        );
        let snippet = metrics.snippet.then(|| {
            elide(
                &palette.snippet[tone],
                row.as_ref()
                    .and_then(|row| row.preview.clone())
                    .unwrap_or_default()
                    .as_str(),
            )
        });

        let avatar = line(
            &palette.avatar[tone],
            &initials(row.as_ref().and_then(|row| row.from.as_ref())),
        );

        // Key hints are the focused row's alone: the app teaches its own
        // keyboard without the list carrying the clutter on every line.
        let hints = if focused {
            hints_for_row(&self.imp().keymap.borrow(), row.as_ref())
                .iter()
                .map(|(key, label)| (line(&palette.key, key), line(&palette.hint, label)))
                .collect()
        } else {
            Vec::new()
        };

        // Baselines, so the sender, the badge and the time sit on one line
        // rather than on three near-misses.
        let baseline = |layout: &pango::Layout| layout.baseline() as f32 / pango::SCALE as f32;
        let mut line1 = baseline(&sender).max(baseline(&time));
        if let Some(badge) = &badge {
            line1 = line1.max(baseline(badge));
        }
        let below = |layout: &pango::Layout| layout.pixel_size().1 as f32 - baseline(layout);
        let mut line1_h = line1 + below(&sender).max(below(&time));
        if let Some(badge) = &badge {
            line1_h = line1_h.max(line1 + below(badge));
        }

        let subject_y = line1_h + metrics.subject_gap;
        let snippet_y = subject_y + subject.pixel_size().1 as f32;
        let mut content = snippet_y;
        if let Some(snippet) = &snippet {
            content += snippet.pixel_size().1 as f32;
        }
        let hints_y = content + metrics.hints_gap;
        if !hints.is_empty() {
            let cap = hints
                .iter()
                .map(|(key, label)| {
                    (key.pixel_size().1 as f32 + CAP.1 * 2.0).max(label.pixel_size().1 as f32)
                })
                .fold(0.0f32, f32::max);
            content = hints_y + cap;
        }

        let height = metrics.pad_y * 2.0 + content.max(metrics.avatar);
        let _ = unread;
        Laid {
            avatar,
            sender,
            time,
            badge,
            subject,
            snippet,
            hints,
            line1,
            subject_y,
            snippet_y,
            hints_y,
            height,
            width,
            tone,
            focused,
        }
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let width = self.width() as f32;
        if width <= 0.0 {
            return;
        }
        self.lay_out(self.width());
        let palette = self.palette();
        let metrics = Metrics::for_density(imp.density.get());
        let laid = imp.laid.borrow();
        let Some(laid) = laid.as_ref() else {
            return;
        };
        let row = imp.row.borrow();
        let height = self.height() as f32;
        let selected = imp.selected.get();
        let cursor = imp.cursor.get();
        let hovered = imp.hovered.get();
        let unread = row.as_ref().is_some_and(|row| !row.seen);

        let rect = |x: f32, y: f32, w: f32, h: f32| graphene::Rect::new(x, y, w, h);
        let fill = |color: &gdk::RGBA, x, y, w, h| snapshot.append_color(color, &rect(x, y, w, h));

        // Three facts, three devices, so they stay legible apart when a row
        // is more than one of them at once (`postio-qhz.1`, #753):
        //
        //   cursor    where the keyboard is — the 3px accent edge, always
        //   selected  what an action will hit — its own deeper ground, and
        //             a steel check where the avatar was
        //   focused   the keyboard is *here*, in this window — the key hints
        //
        // The pairing used to be different, and that was the bug. The ground
        // carried *both* cursor and selection, as two steps of one colour —
        // 2% apart in light and the identical token in dark — leaving the
        // check glyph as the only real tiebreaker. Which gave two ways to
        // see nothing at all: an icon theme without `object-select-symbolic`
        // resolves the check to `None` and draws neither glyph nor initials,
        // and this branch was `if selected … else if cursor`, so a row that
        // was both never drew the cursor's tint. Bulk-selecting, the row the
        // keyboard was on and the rows it had already taken were the same
        // pixels.
        //
        // So the two facts get two *devices* rather than two shades of one:
        // the edge is the cursor's, the ground is the selection's, and a row
        // that is both shows both. Neither depends on an icon resolving, or
        // on telling two tints apart in dark. The check stays where it
        // renders, as reinforcement rather than as the whole distinction.
        //
        // The edge is also this widget's focus ring: it paints its own
        // pixels, so no CSS `outline` reaches it. It follows the *cursor*
        // rather than `shows_hints()` — gating it on the hints flag meant
        // `[ui] show_key_hints = false` silently deleted the focus
        // indicator, and gating it on window focus meant clicking into the
        // reading pane did too. The canvas draws it on the current row
        // unconditionally (`Design/Mail Client.dc.html:76`); only the key
        // caps are the flag's business.
        if selected {
            fill(&palette.checked_bg, 0.0, 0.0, width, height);
        } else if cursor {
            fill(&palette.selected_bg, 0.0, 0.0, width, height);
        } else if hovered {
            fill(&palette.hover_bg, 0.0, 0.0, width, height);
        }
        if cursor {
            fill(&palette.selected_edge, 0.0, 0.0, EDGE, height);
        }

        // One rule between rows, and none above the first — the header
        // draws that boundary itself. A selected or cursor row still gets
        // one, in the higher-contrast variant, so the tint reads as one row
        // among rows rather than as paste-over.
        match top_edge(selected, cursor, imp.first.get()) {
            Some(Edge::Hairline) => fill(&palette.hairline, 0.0, 0.0, width, 1.0),
            Some(Edge::Strong) => fill(&palette.hairline_strong, 0.0, 0.0, width, 1.0),
            None => {}
        }

        // Nothing bound yet: the page is on its way. A skeleton says "this
        // is a row and it is coming" without pretending to be data.
        let Some(row) = row.as_ref() else {
            let mut ghost = palette.hairline;
            ghost.set_alpha(ghost.alpha() * 0.6);
            let y = metrics.pad_y;
            fill(&ghost, metrics.inset, y, metrics.avatar, metrics.avatar);
            let column_x = metrics.inset + metrics.avatar + metrics.gap;
            let column = width - column_x - metrics.inset;
            fill(&ghost, column_x, y + 2.0, column * 0.34, 9.0);
            fill(&ghost, column_x, y + 19.0, column * 0.78, 9.0);
            return;
        };

        let text = |layout: &pango::Layout, color: &gdk::RGBA, x: f32, y: f32| {
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x, y));
            snapshot.append_layout(layout, color);
            snapshot.restore();
        };

        // ── the avatar chip, or the check that replaces it ───────────
        //
        // The check takes the chip's own square rather than adding a column:
        // a checkbox that appears beside the avatar would move every row's
        // text sideways the moment a selection started, and a list that
        // reflows while you are selecting in it is a list you cannot aim at.
        // The same square is also the mouse's way in — hovering an
        // unselected row offers the outline, so selecting with the pointer
        // does not require knowing that Ctrl-click exists.
        let chip = rect(metrics.inset, metrics.pad_y, metrics.avatar, metrics.avatar);
        let offering = hovered && !selected;
        if selected {
            snapshot.append_color(&palette.selected_edge, &chip);
        } else if unread {
            snapshot.append_color(&palette.unread_chip, &chip);
        }
        snapshot.append_border(
            &gsk::RoundedRect::from_rect(chip, 0.0),
            &[1.0; 4],
            &[if selected {
                palette.selected_edge
            } else {
                palette.hairline
            }; 4],
        );

        match (selected || offering, &palette.check) {
            (true, Some(check)) => {
                let size = (metrics.avatar * 0.62).round();
                let inset = ((metrics.avatar - size) / 2.0).round();
                let mark = if selected {
                    palette.checked_mark
                } else {
                    palette.hairline
                };
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    metrics.inset + inset,
                    metrics.pad_y + inset,
                ));
                check.snapshot_symbolic(snapshot, size as f64, size as f64, &[mark]);
                snapshot.restore();
            }
            // No check glyph in the icon theme, and no initials either while
            // the row is selected: a chip that still said "AL" would be
            // saying the one thing the selection has to override.
            (true, None) => {}
            (false, _) => {
                let (aw, ah) = laid.avatar.pixel_size();
                text(
                    &laid.avatar,
                    &palette.avatar[laid.tone].color,
                    metrics.inset + (metrics.avatar - aw as f32) / 2.0,
                    metrics.pad_y + (metrics.avatar - ah as f32) / 2.0,
                );
            }
        }

        // ── the sender line ──────────────────────────────────────────
        let column_x = metrics.inset + metrics.avatar + metrics.gap;
        let right = width - metrics.inset;
        let top = metrics.pad_y;
        let baseline = |layout: &pango::Layout| layout.baseline() as f32 / pango::SCALE as f32;
        let on_line = |layout: &pango::Layout| top + laid.line1 - baseline(layout);

        // The actions take the timestamp's place rather than crowding in
        // beside it: a row is 404px wide at the canvas' proportions, and a
        // subject that shortened every time the pointer crossed it would be
        // the list rearranging itself under the mouse. The time is the one
        // thing on the line that can be read again a moment later.
        let offering_actions = self.offers_actions();
        let time_w = if offering_actions {
            for (slot, action) in RowAction::ALL.into_iter().enumerate() {
                let glyph = match action {
                    RowAction::Archive => &palette.archive,
                    RowAction::Flag if row.flagged => &palette.flagged,
                    RowAction::Flag => &palette.unflagged,
                    RowAction::Delete => &palette.trash,
                };
                let Some(glyph) = glyph else { continue };
                let x = self.action_x(slot as f32, metrics.inset);
                let y = metrics.pad_y + (metrics.avatar - ACTION) / 2.0;
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x, y));
                glyph.snapshot_symbolic(
                    snapshot,
                    ACTION as f64,
                    ACTION as f64,
                    &[palette.hint.color],
                );
                snapshot.restore();
            }
            ACTION * RowAction::ALL.len() as f32 + ACTION_GAP * (RowAction::ALL.len() as f32 - 1.0)
        } else {
            let time_w = laid.time.pixel_size().0 as f32;
            text(
                &laid.time,
                &palette.time[laid.tone].color,
                right - time_w,
                on_line(&laid.time),
            );
            time_w
        };
        let mut cursor = right - time_w;

        if row.has_attachments
            && let Some(clip) = &palette.clip
        {
            cursor -= RUN + CLIP as f32;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                cursor,
                top + laid.line1 - CLIP as f32 * 0.82,
            ));
            clip.snapshot_symbolic(
                snapshot,
                CLIP as f64,
                CLIP as f64,
                &[palette.time[laid.tone].color],
            );
            snapshot.restore();
        }

        // Draft, answered, flagged: the same paperclip-sized mark, left of
        // it in that order — `postio-apz`. A status the eye should catch in
        // passing, not a hover target, so it is drawn at `CLIP`'s size like
        // the attachment glyph rather than `ACTION`'s.
        let mut mark = |present: bool, glyph: &Option<gtk::IconPaintable>| {
            let (Some(glyph), true) = (glyph, present) else {
                return;
            };
            cursor -= RUN + CLIP as f32;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                cursor,
                top + laid.line1 - CLIP as f32 * 0.82,
            ));
            glyph.snapshot_symbolic(
                snapshot,
                CLIP as f64,
                CLIP as f64,
                &[palette.time[laid.tone].color],
            );
            snapshot.restore();
        };
        mark(row.draft, &palette.draft_mark);
        mark(row.answered, &palette.answered_mark);
        mark(row.flagged, &palette.flag_mark);

        text(
            &laid.sender,
            &palette.sender[laid.tone].color,
            column_x,
            on_line(&laid.sender),
        );

        // The thread-count badge sits immediately after the sender, an
        // outline tag in mono — the canvas' own `14`.
        if let Some(badge) = &laid.badge {
            let (bw, bh) = badge.pixel_size();
            let sender_w = laid.sender.pixel_size().0 as f32;
            let x = (column_x + sender_w + RUN).min(cursor - RUN - bw as f32 - CAP.0 * 2.0);
            if x > column_x {
                let y = on_line(badge) - CAP.1;
                snapshot.append_border(
                    &gsk::RoundedRect::from_rect(
                        rect(x, y, bw as f32 + CAP.0 * 2.0, bh as f32 + CAP.1 * 2.0),
                        CAP.2,
                    ),
                    &[1.0; 4],
                    &[palette.hairline; 4],
                );
                text(badge, &palette.badge.color, x + CAP.0, on_line(badge));
            }
        }

        // ── subject and snippet ──────────────────────────────────────
        text(
            &laid.subject,
            &palette.subject[laid.tone].color,
            column_x,
            top + laid.subject_y,
        );
        if let Some(snippet) = &laid.snippet {
            text(
                snippet,
                &palette.snippet[laid.tone].color,
                column_x,
                top + laid.snippet_y,
            );
        }

        // ── the key hints, on the focused row and nowhere else ───────
        let mut x = column_x;
        for (key, label) in &laid.hints {
            let (kw, kh) = key.pixel_size();
            let cap_w = kw as f32 + CAP.0 * 2.0;
            let cap_h = kh as f32 + CAP.1 * 2.0;
            let y = top + laid.hints_y;
            snapshot.append_border(
                &gsk::RoundedRect::from_rect(rect(x, y, cap_w, cap_h), CAP.2),
                &[1.0; 4],
                &[palette.key_edge; 4],
            );
            text(key, &palette.key.color, x + CAP.0, y + CAP.1);
            x += cap_w + 6.0;
            let (lw, lh) = label.pixel_size();
            text(label, &palette.hint.color, x, y + (cap_h - lh as f32) / 2.0);
            x += lw as f32 + 10.0;
        }
    }
}

/// What the sender column shows.
///
/// On a message row that is the sender. On a thread row it is the people in
/// the conversation, because that is what the row stands for (ADR 0015 Q2) —
/// naming only the newest sender would make a six-person thread look like a
/// message from whoever happened to reply last.
fn initials_source(row: &Row) -> String {
    if row.is_thread() {
        let line = participants_line(&row.participants);
        if !line.is_empty() {
            return line;
        }
    }
    row.from
        .as_ref()
        .map(|from| from.display().to_string())
        .unwrap_or_else(|| "unknown sender".to_string())
}

/// How many names fit before the line starts eliding.
const NAMES_SHOWN: usize = 3;

/// The people in a conversation, short and newest-biased.
///
/// Short names, because the column is one line beside a subject and a
/// snippet: a conversation between four people spelled out in full would push
/// everything else off the row. First name where there is a display name, the
/// address's local part where there is not — the same information a person
/// uses to recognise a thread at a glance.
///
/// **Newest-biased when it elides.** Participants arrive in first-seen order,
/// so the interesting end is the far one: who started the conversation still
/// identifies it, and who spoke most recently is what changed since you last
/// looked. Both survive; the middle is what goes.
fn participants_line(participants: &[EmailAddress]) -> String {
    let short = |address: &EmailAddress| -> String {
        let display = address.display().to_string();
        // A display name is "Ada Norwood"; an address falls back to its local
        // part, which is what a sender without a name has to identify them.
        let name = display
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .to_string();
        if name.contains('@') {
            return name.split('@').next().unwrap_or(&name).to_string();
        }
        if name.is_empty() { display } else { name }
    };

    // Distinct first, on the full name: one person writing five times is one
    // name, and two different people can share a first name.
    let mut distinct: Vec<&EmailAddress> = Vec::new();
    for address in participants {
        if !distinct
            .iter()
            .any(|seen| seen.display() == address.display())
        {
            distinct.push(address);
        }
    }

    // One participant is not a crowd, and shortening it loses information for
    // nothing: "Site Office" becomes "Site" and an address with no display
    // name becomes half of itself. A conversation with one voice reads
    // exactly like the message row it replaced, which is what the canvas
    // draws.
    if distinct.len() == 1 {
        return distinct[0].display().to_string();
    }

    let mut names: Vec<String> = Vec::new();
    for address in distinct {
        let name = short(address);
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    match names.len() {
        0 => String::new(),
        n if n <= NAMES_SHOWN => names.join(", "),
        _ => {
            let last = names.len();
            format!(
                "{} .. {}",
                names[0],
                names[last - (NAMES_SHOWN - 1)..].join(", ")
            )
        }
    }
}

/// The one number `lay_out` hands back; the layouts stay cached.
struct Summary {
    height: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone, Utc};
    use postio_model::ids::MessageId;

    // -- the participants line (ADR 0015 Q2, #307) ------------------------

    fn people(names: &[(&str, &str)]) -> Vec<EmailAddress> {
        names
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect()
    }

    #[test]
    fn one_participant_keeps_their_whole_name() {
        // A conversation with one voice has to read exactly like the message
        // row it replaced — every row in a folder is a thread row now, so
        // shortening here would shorten the ordinary case. "Site Office"
        // must not become "Site".
        assert_eq!(
            participants_line(&people(&[("Ada Norwood", "ada@example.com")])),
            "Ada Norwood"
        );
        assert_eq!(
            participants_line(&people(&[("Site Office", "site@example.com")])),
            "Site Office"
        );
    }

    #[test]
    fn a_short_conversation_names_everyone_in_it() {
        assert_eq!(
            participants_line(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Tove Bergstrom", "tove@example.com"),
            ])),
            "Ada, Quinn, Tove"
        );
    }

    #[test]
    fn a_long_conversation_keeps_both_ends_and_drops_the_middle() {
        // Newest-biased: whoever started it still identifies the
        // conversation, and whoever spoke last is what changed since you
        // looked. The middle is what nobody scans for.
        assert_eq!(
            participants_line(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Jonas Vek", "jonas@example.org"),
                ("Tove Bergstrom", "tove@example.com"),
                ("Priya Raman", "priya@example.org"),
            ])),
            "Ada .. Tove, Priya"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_is_named_once() {
        assert_eq!(
            participants_line(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
            ])),
            "Ada, Quinn"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_and_alone_is_not_a_crowd() {
        // Deduplication happens before the "is this one voice" test, or a
        // thread where one person replied to themselves would read as two.
        assert_eq!(
            participants_line(&people(&[
                ("Site Office", "site@example.com"),
                ("Site Office", "site@example.com"),
            ])),
            "Site Office"
        );
    }

    #[test]
    fn a_lone_sender_with_no_display_name_keeps_their_whole_address() {
        assert_eq!(
            participants_line(&[EmailAddress::new(None::<&str>, "ada.norwood@example.com")]),
            "ada.norwood@example.com"
        );
    }

    #[test]
    fn a_crowd_with_no_display_names_is_named_by_local_parts() {
        // Several addresses do not fit; their local parts do, and are what
        // distinguishes them.
        assert_eq!(
            participants_line(&[
                EmailAddress::new(None::<&str>, "ada@example.com"),
                EmailAddress::new(None::<&str>, "quinn@example.net"),
            ]),
            "ada, quinn"
        );
    }

    #[test]
    fn no_participants_is_empty_rather_than_a_placeholder() {
        // A message row has none, and `initials_source` falls back to the
        // sender rather than drawing something that looks like a thread.
        assert_eq!(participants_line(&[]), "");
    }

    fn addr(name: Option<&str>, address: &str) -> EmailAddress {
        EmailAddress::new(name, address)
    }

    #[test]
    fn initials_are_the_canvas_two_letters() {
        assert_eq!(
            initials(Some(&addr(Some("Lena Tomlin"), "lena@example.com"))),
            "LT"
        );
        assert_eq!(
            initials(Some(&addr(Some("Nadia Okafor"), "nadia@example.com"))),
            "NO"
        );
        assert_eq!(
            initials(Some(&addr(Some("lkml"), "lkml@example.org"))),
            "LK"
        );
        assert_eq!(initials(Some(&addr(None, "buildbot@example.net"))), "BU");
        assert_eq!(initials(None), "?");
    }

    #[test]
    fn compact_drops_the_snippet_and_every_density_is_shorter_than_the_last() {
        let airy = Metrics::for_density(Density::Airy);
        let comfortable = Metrics::for_density(Density::Comfortable);
        let compact = Metrics::for_density(Density::Compact);
        assert!(airy.snippet && comfortable.snippet);
        assert!(
            !compact.snippet,
            "the tightest density trades the snippet for rows"
        );
        assert!(airy.pad_y > comfortable.pad_y && comfortable.pad_y > compact.pad_y);
        assert!(airy.avatar > compact.avatar);
    }

    #[test]
    fn hints_read_the_live_keymap_not_a_hard_coded_key() {
        let defaults = default_hints();
        assert_eq!(
            defaults,
            vec![
                ("e".to_string(), "reply"),
                ("a".to_string(), "archive"),
                ("t".to_string(), "thread"),
            ],
            "the registry's own bindings, canvas order"
        );

        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_string(), "x".to_string());
        let rebound = hints_for(&Keymap::resolve(&overrides));
        assert_eq!(
            rebound,
            vec![
                ("e".to_string(), "reply"),
                ("x".to_string(), "archive"),
                ("t".to_string(), "thread"),
            ],
            "a rebind in [keys] must reach the hint, not just the resolver"
        );
    }

    #[test]
    fn a_selected_or_cursor_row_still_draws_a_boundary_above_itself() {
        // Not first: an ordinary row gets the ordinary hairline.
        assert_eq!(top_edge(false, false, false), Some(Edge::Hairline));
        // Selected or the cursor: the tint would wash a plain hairline out,
        // so the higher-contrast rule draws instead of nothing.
        assert_eq!(top_edge(true, false, false), Some(Edge::Strong));
        assert_eq!(top_edge(false, true, false), Some(Edge::Strong));
        assert_eq!(top_edge(true, true, false), Some(Edge::Strong));
        // The first row draws no edge of its own regardless of state — the
        // header/rows boundary is the header's job, not the row's, so it
        // cannot depend on which row happens to be selected.
        assert_eq!(top_edge(false, false, true), None);
        assert_eq!(top_edge(true, false, true), None);
        assert_eq!(top_edge(false, true, true), None);
    }

    #[test]
    fn a_row_with_no_thread_drops_the_thread_hint() {
        // Matches the badge's own threshold (`row.rs`'s `build`, filtering
        // on `thread_count > 1`): the badge and the hint must never disagree
        // about whether there is a thread here to open.
        let keymap = Keymap::resolve(&Default::default());
        let mut row = Row {
            id: MessageId::new(1),
            thread: None,
            from: None,
            subject: None,
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 14, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        };
        assert_eq!(
            hints_for_row(&keymap, Some(&row)),
            vec![("e".to_string(), "reply"), ("a".to_string(), "archive"),],
            "one message in the thread is not a thread to open"
        );

        row.thread_count = 2;
        assert_eq!(
            hints_for_row(&keymap, Some(&row)),
            vec![
                ("e".to_string(), "reply"),
                ("a".to_string(), "archive"),
                ("t".to_string(), "thread"),
            ],
            "more than one message in the thread, so the hint returns"
        );

        assert_eq!(
            hints_for_row(&keymap, None),
            vec![("e".to_string(), "reply"), ("a".to_string(), "archive")],
            "no row bound yet, nothing to thread either"
        );
    }

    #[test]
    fn a_command_that_lost_its_key_drops_its_hint_rather_than_naming_the_wrong_one() {
        // Taking `a` for something else in the same context leaves Archive
        // with no key at all — reachable only from the palette. A hint that
        // kept printing "a archive" here would be teaching a key that does
        // something else.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("forward".to_string(), "a".to_string());
        let hints = hints_for(&Keymap::resolve(&overrides));
        assert_eq!(
            hints,
            vec![("e".to_string(), "reply"), ("t".to_string(), "thread")],
            "archive lost its key to forward, so its hint disappears rather than lying"
        );
    }

    #[test]
    fn today_is_a_time_this_week_is_a_weekday_and_older_is_a_date() {
        // Built in the local zone and handed over as UTC: the column shows
        // the reader's own clock, so a test that fixed both ends in UTC
        // would only pass in one timezone.
        let local = |y, m, d, h, min| Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap();
        let now = local(2026, 8, 23, 14, 0);
        let at = |y, m, d, h, min| local(y, m, d, h, min).with_timezone(&Utc);

        assert_eq!(timestamp(at(2026, 8, 23, 9, 14), now), "09:14");
        assert_eq!(timestamp(at(2026, 8, 20, 9, 14), now), "Thu");
        assert_eq!(timestamp(at(2026, 8, 2, 9, 14), now), "2 Aug");
        assert_eq!(timestamp(at(2024, 8, 12, 9, 14), now), "12 Aug 24");
    }

    #[test]
    fn a_screen_reader_hears_everything_the_row_draws() {
        let mut row = Row {
            id: MessageId::new(1),
            thread: None,
            from: Some(addr(Some("Lena Tomlin"), "lena@example.com")),
            subject: Some("Re: maildir index rebuild".into()),
            preview: Some("Confirmed on 0.4.1".into()),
            received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 14, 0).unwrap(),
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: true,
            thread_count: 14,
            participants: Vec::new(),
        };
        let label = accessible_label(&row);
        assert!(label.contains("Unread"), "{label}");
        assert!(label.contains("Lena Tomlin"), "{label}");
        assert!(label.contains("Re: maildir index rebuild"), "{label}");
        assert!(label.contains("14 in thread"), "{label}");
        assert!(label.contains("attachment"), "{label}");

        row.seen = true;
        row.has_attachments = false;
        row.thread_count = 1;
        let read = accessible_label(&row);
        assert!(!read.contains("Unread"), "{read}");
        assert!(!read.contains("in thread"), "{read}");
        assert!(!read.contains("attachment"), "{read}");
    }

    #[test]
    fn flagged_answered_and_draft_each_get_a_word() {
        // `postio-apz`: the row drew none of these. A glyph is not enough on
        // its own — see `snapshot`'s `mark` closure for the picture half —
        // this is the half a screen reader gets.
        let base = Row {
            id: MessageId::new(1),
            thread: None,
            from: Some(addr(Some("Lena Tomlin"), "lena@example.com")),
            subject: Some("Re: maildir index rebuild".into()),
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 14, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        };

        let none = accessible_label(&base);
        assert!(!none.contains("Flagged"), "{none}");
        assert!(!none.contains("Answered"), "{none}");
        assert!(!none.contains("Draft"), "{none}");

        let flagged = accessible_label(&Row {
            flagged: true,
            ..base.clone()
        });
        assert!(flagged.contains("Flagged"), "{flagged}");

        let answered = accessible_label(&Row {
            answered: true,
            ..base.clone()
        });
        assert!(answered.contains("Answered"), "{answered}");

        let draft = accessible_label(&Row {
            draft: true,
            ..base
        });
        assert!(draft.contains("Draft"), "{draft}");
    }

    #[test]
    fn a_row_with_nothing_in_it_still_says_something() {
        let row = Row {
            id: MessageId::new(1),
            thread: None,
            from: None,
            subject: None,
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 14, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        };
        assert!(!accessible_label(&row).is_empty());
    }
}
