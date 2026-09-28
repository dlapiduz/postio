//! One row of Focus's list, drawn in one `snapshot()`.
//!
//! Nested boxes per row are the usual reason a GTK list feels slow
//! (`/gtk-design` §4), and a jump rebinds GTK's whole resident window, about
//! 205 rows, in one frame (spike S3, T009). So a row is one widget that lays
//! out its text with Pango and draws it, and binding one is replacing what it
//! holds.
//!
//! # The one-line row (contracts/focus-surface.md, "Rows")
//!
//! Left to right: the gutter (24 px), the sender (from x = 56), the subject
//! with up to two label pills and then the first line, dimmed; and on the
//! right the attachment mark, the count badge and the time. Unread is bold.
//! Everything is drawn from the row's data as it arrived: the sender, the
//! subject and the first line verbatim (FR-011), and never read from a
//! message body (FR-020).
//!
//! At a narrow width the first line gives way first, then the labels,
//! before the sender, the subject and the time (spec Edge Cases, "Long
//! text"); nothing wraps, and the height never changes (FR-013).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_core::Keymap;
use postio_ui::focus_row::{MAX_PILLS, count_badge, marker_line};
use postio_ui::label_colour::{Rgb, label_colour};

use super::item::FocusRow;
use super::model::RowObject;

/// What a row drew in its last snapshot: every text it laid out, in order,
/// each label pill with its colour, and whether it was drawn bold. What a
/// person sees, for a test to read back.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// Every text laid out, in the order drawn.
    pub texts: Vec<String>,
    /// Each label pill: its name and its dot's colour.
    pub pills: Vec<(String, gdk::RGBA)>,
    /// Whether the sender and subject were bold: an unread conversation.
    pub bold: bool,
    /// Whether the row drew itself selected: the neutral ground and the
    /// checked box.
    pub picked: bool,
    /// The ink the row drew its text in.
    pub ink: gdk::RGBA,
    /// The accent a marker was drawn in, when the row has one.
    pub accent: Option<gdk::RGBA>,
    /// Each answering action drawn on the second line: its command, its
    /// words and where its button is (x, y, width, height), so a click there
    /// runs what its key runs.
    pub actions: Vec<(postio_core::CommandId, String, [f32; 4])>,
}

impl Drawn {
    /// Where the action button saying `words` was drawn: its centre, in the
    /// row's coordinates.
    pub fn action(&self, words: &str) -> Option<(f32, f32)> {
        self.actions
            .iter()
            .find(|(_, said, _)| said == words)
            .map(|(_, _, [x, y, width, height])| (x + width / 2.0, y + height / 2.0))
    }

    /// The action whose button covers `x`, `y`, if one does.
    pub fn action_at(&self, x: f32, y: f32) -> Option<postio_core::CommandId> {
        self.actions
            .iter()
            .find(|(_, _, [left, top, width, height])| {
                (*left..=left + width).contains(&x) && (*top..=top + height).contains(&y)
            })
            .map(|(command, _, _)| *command)
    }
}

impl Default for Drawn {
    fn default() -> Self {
        Drawn {
            texts: Vec::new(),
            pills: Vec::new(),
            bold: false,
            picked: false,
            ink: gdk::RGBA::TRANSPARENT,
            accent: None,
            actions: Vec::new(),
        }
    }
}

/// A one-line row's height, in pixels: the classic row's (research R3).
pub const ONE_LINE: i32 = 40;

/// A two-line row's height: a conversation with a marker.
pub const TWO_LINES: i32 = 72;

/// Where the sender column starts, and how wide it is (focus-surface.md).
const SENDER_X: f32 = 56.0;
const SENDER_WIDTH: f32 = 222.0;
/// Where the subject column starts.
pub const SUBJECT_X: f32 = 290.0;
/// The space the right edge keeps.
const TRAILING: f32 = 24.0;
/// The gap between two things on the line.
const GAP: f32 = 12.0;
/// A label pill's height, its dot, and the room around its name.
const PILL_HEIGHT: f32 = 20.0;
const PILL_DOT: f32 = 6.0;
const PILL_PAD: f32 = 7.0;
/// The attachment mark's size.
const ICON: f32 = 16.0;
/// Below this, a first line is not worth drawing: it would be a letter.
const LEAST_FIRST_LINE: f32 = 40.0;
/// Where a two-line row's lines sit (their centres), from screen 01.
const FIRST_LINE: f32 = 22.0;
const SECOND_LINE: f32 = 51.0;
/// The marker dot in the gutter.
const GUTTER_CENTRE: f32 = 30.0;
const DOT: f32 = 7.0;
/// An action's button on the second line, and the keycap in it.
const ACTION_HEIGHT: f32 = 24.0;
const ACTION_PAD: f32 = 9.0;
const KEYCAP_PAD: f32 = 4.0;

/// The keymap a list's rows read their keycaps from, shared by every row
/// and replaced when `[keys]` changes.
pub type SharedKeymap = Rc<RefCell<Keymap>>;

/// What a click on a row's drawn action runs: the command, for the row's
/// item.
pub type ActionHandler = Rc<dyn Fn(&FocusRow, postio_core::CommandId)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowWidget {
        pub item: RefCell<Option<FocusRow>>,
        pub bound: RefCell<Option<(RowObject, glib::SignalHandlerId)>>,
        pub drawn: RefCell<Drawn>,
        pub keymap: RefCell<Option<SharedKeymap>>,
        pub picked: RefCell<Option<postio_ui::selection::SelectionState>>,
        pub on_action: RefCell<Option<ActionHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowWidget {
        const NAME: &'static str = "PostioFocusRow";
        type Type = super::RowWidget;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("focusrow");
            class.set_accessible_role(gtk::AccessibleRole::Row);
        }
    }

    impl ObjectImpl for RowWidget {
        fn constructed(&self) {
            self.parent_constructed();
            // The answering actions on the second line are buttons a person
            // can press, as the keys beside them say (constitution II): a
            // press on one runs its command for this row, and nothing
            // else takes the click.
            let click = gtk::GestureClick::new();
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = row)]
                self.obj(),
                move |gesture, _, x, y| {
                    if row.press_at(x as f32, y as f32) {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            self.obj().add_controller(click);
        }

        fn dispose(&self) {
            self.obj().unbind();
        }
    }

    impl WidgetImpl for RowWidget {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => {
                    let height = self.obj().height();
                    (height, height, -1, -1)
                }
                _ => (0, 0, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    /// One row of Focus's list.
    pub struct RowWidget(ObjectSubclass<imp::RowWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for RowWidget {
    fn default() -> Self {
        glib::Object::new()
    }
}

/// The colours a row draws in, read from its own style: the ink, and the
/// quieter steps libadwaita takes of it; and the accent, from
/// `AdwStyleManager` -- for a marker, the one thing on a row FR-091 gives
/// it -- whose hue the label colours keep away from.
struct Palette {
    ink: gdk::RGBA,
    dim: gdk::RGBA,
    rule: gdk::RGBA,
    raised: gdk::RGBA,
    accent: gdk::RGBA,
    accent_hue: f64,
}

impl Palette {
    fn of(widget: &RowWidget) -> Self {
        let ink = widget.color();
        let faded = |by: f32| {
            let mut colour = ink;
            colour.set_alpha(ink.alpha() * by);
            colour
        };
        let manager = adw::StyleManager::default();
        let accent = manager.accent_color().to_standalone_rgba(manager.is_dark());
        Palette {
            ink,
            dim: faded(0.55),
            rule: faded(0.18),
            raised: faded(0.06),
            accent,
            accent_hue: rgb(&accent).hue(),
        }
    }
}

fn rgb(colour: &gdk::RGBA) -> Rgb {
    let byte = |channel: f32| (channel * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb::new(
        byte(colour.red()),
        byte(colour.green()),
        byte(colour.blue()),
    )
}

fn rgba(colour: Rgb) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(colour.r) / 255.0,
        f32::from(colour.g) / 255.0,
        f32::from(colour.b) / 255.0,
        1.0,
    )
}

impl RowWidget {
    /// Read keycaps from `keymap`, which the list shares with every row.
    pub fn set_keymap(&self, keymap: SharedKeymap) {
        self.imp().keymap.replace(Some(keymap));
        self.queue_draw();
    }

    /// Draw the selection's box from `picked`, which the list shares with
    /// every row.
    pub fn set_selection(&self, picked: postio_ui::selection::SelectionState) {
        self.imp().picked.replace(Some(picked));
        self.queue_draw();
    }

    /// Whether this row is in the selection: what `a` would archive.
    pub fn is_picked(&self) -> bool {
        let Some(id) = self.imp().item.borrow().as_ref().map(FocusRow::id) else {
            return false;
        };
        self.imp()
            .picked
            .borrow()
            .as_ref()
            .is_some_and(|picked| picked.contains(id))
    }

    /// Show what `row` stands for, and follow it as it changes.
    pub fn bind(&self, row: &RowObject) {
        self.unbind();
        let handler = row.connect_changed({
            let widget = self.downgrade();
            move |row| {
                if let Some(widget) = widget.upgrade() {
                    widget.show(row.item());
                }
            }
        });
        self.imp().bound.replace(Some((row.clone(), handler)));
        self.show(row.item());
    }

    /// Stop following the row this widget was showing.
    pub fn unbind(&self) {
        if let Some((row, handler)) = self.imp().bound.take() {
            row.disconnect(handler);
        }
    }

    /// What this row says, as a screen reader hears it and a test reads it:
    /// "Loading" while its page has not arrived.
    pub fn spoken(&self) -> String {
        self.imp()
            .item
            .borrow()
            .as_ref()
            .map_or_else(|| "Loading".to_owned(), spoken)
    }

    /// What the row drew in its last snapshot.
    pub fn drawn(&self) -> Drawn {
        self.imp().drawn.borrow().clone()
    }

    /// Run what a click on an action at `x`, `y` runs; whether one was
    /// there. Public, as the window's `handle_key` is, so a test presses
    /// the button a person presses without synthesizing a pointer event.
    pub fn press_at(&self, x: f32, y: f32) -> bool {
        let Some(command) = self.imp().drawn.borrow().action_at(x, y) else {
            return false;
        };
        let (Some(item), Some(handler)) = (
            self.imp().item.borrow().clone(),
            self.imp().on_action.borrow().clone(),
        ) else {
            return false;
        };
        handler(&item, command);
        true
    }

    /// Run `handler` when one of the row's drawn actions is pressed.
    pub fn set_on_action(&self, handler: ActionHandler) {
        self.imp().on_action.replace(Some(handler));
    }

    /// The item shown, if its page has arrived.
    pub fn item(&self) -> Option<FocusRow> {
        self.imp().item.borrow().clone()
    }

    fn show(&self, item: Option<FocusRow>) {
        let height_before = self.height();
        self.imp().item.replace(item);
        self.update_property(&[gtk::accessible::Property::Label(&self.spoken())]);
        if self.height() != height_before {
            self.queue_resize();
        }
        self.queue_draw();
    }

    /// The row's height, fixed by its kind (FR-013).
    pub fn height(&self) -> i32 {
        match self.imp().item.borrow().as_ref() {
            Some(item) if item.two_lines() => TWO_LINES,
            _ => ONE_LINE,
        }
    }

    /// A layout of `text` in the row's font, `bold` or not, `scale` of its
    /// size.
    fn layout(&self, text: &str, bold: bool, scale: f64) -> pango::Layout {
        let layout = self.create_pango_layout(Some(text));
        // Text from mail is data, never markup: set as plain text, with its
        // weight and size as attributes rather than tags inside it.
        let attributes = pango::AttrList::new();
        if bold {
            attributes.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
        }
        if (scale - 1.0).abs() > f64::EPSILON {
            attributes.insert(pango::AttrFloat::new_scale(scale));
        }
        layout.set_attributes(Some(&attributes));
        layout.set_single_paragraph_mode(true);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        layout
    }

    /// Draw `layout` at `x`, vertically centred on `middle`, within `room`
    /// pixels. Answers how wide it drew.
    fn put(
        &self,
        snapshot: &gtk::Snapshot,
        layout: &pango::Layout,
        x: f32,
        middle: f32,
        room: f32,
        colour: &gdk::RGBA,
    ) -> f32 {
        layout.set_width((room.max(0.0) * pango::SCALE as f32) as i32);
        let (width, height) = layout.pixel_size();
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, middle - height as f32 / 2.0));
        snapshot.append_layout(layout, colour);
        snapshot.restore();
        width as f32
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let Some(item) = self.imp().item.borrow().clone() else {
            self.imp().drawn.replace(Drawn::default());
            self.draw_skeleton(snapshot);
            return;
        };
        let row = item.row();
        let summary = &row.summary;
        let palette = Palette::of(self);
        let width = self.width() as f32;
        let two_lines = summary.marker.is_some();
        let middle = if two_lines {
            FIRST_LINE
        } else {
            ONE_LINE as f32 / 2.0
        };
        let bold = summary.has_unread();
        let mut drawn = Drawn {
            bold,
            picked: self.is_picked(),
            ink: palette.ink,
            ..Drawn::default()
        };
        if drawn.picked {
            // Selected: a neutral ground and a checked box in the gutter,
            // never the accent, which is the cursor's (FR-091).
            let height = self.height() as f32;
            snapshot.append_color(
                &palette.raised,
                &graphene::Rect::new(0.0, 0.0, width, height),
            );
            self.draw_icon(
                snapshot,
                "checkbox-checked-symbolic",
                GUTTER_CENTRE - ICON / 2.0,
                middle - ICON / 2.0,
                &palette.ink,
            );
        }

        // The trailing column first, from the right edge in, so the middle
        // knows how much room it has: the time, the count, the attachment.
        let time = postio_ui::row::timestamp(summary.last_at, chrono::Local::now());
        let time_layout = self.layout(&time, bold, 1.0);
        let (time_width, _) = time_layout.pixel_size();
        let mut trailing = width - TRAILING - time_width as f32;
        self.put(
            snapshot,
            &time_layout,
            trailing,
            middle,
            time_width as f32,
            &palette.ink,
        );
        let mut trailing_texts = vec![time];
        if let Some(count) = count_badge(summary.message_count) {
            let badge = self.layout(&count, false, 0.8);
            let (badge_width, badge_height) = badge.pixel_size();
            let boxed = badge_width as f32 + 10.0;
            trailing -= GAP + boxed;
            let frame = graphene::Rect::new(
                trailing,
                middle - (badge_height as f32 + 4.0) / 2.0,
                boxed,
                badge_height as f32 + 4.0,
            );
            let outline = gtk::gsk::RoundedRect::from_rect(frame, 4.0);
            snapshot.append_border(&outline, &[1.0; 4], &[palette.rule; 4]);
            self.put(
                snapshot,
                &badge,
                trailing + 5.0,
                middle,
                badge_width as f32,
                &palette.dim,
            );
            trailing_texts.push(count);
        }
        if summary.has_attachments {
            trailing -= GAP + ICON;
            self.draw_icon(
                snapshot,
                "mail-attachment-symbolic",
                trailing,
                middle - ICON / 2.0,
                &palette.dim,
            );
        }
        let end = trailing - GAP;

        // The sender, then the subject, each as it arrived.
        let sender = summary
            .representative
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default();
        let sender_layout = self.layout(&sender, bold, 1.0);
        self.put(
            snapshot,
            &sender_layout,
            SENDER_X,
            middle,
            SENDER_WIDTH.min(end - SENDER_X),
            &palette.ink,
        );
        drawn.texts.push(sender);

        let subject = summary.representative.subject.clone().unwrap_or_default();
        let subject_layout = self.layout(&subject, bold, 1.0);
        let mut x = SUBJECT_X;
        x += self.put(snapshot, &subject_layout, x, middle, end - x, &palette.ink);
        drawn.texts.push(subject);

        // Up to two pills, while they fit.
        for label in row.labels.iter().take(MAX_PILLS) {
            let name = self.layout(&label.name, false, 0.85);
            let (name_width, _) = name.pixel_size();
            let pill = PILL_PAD + PILL_DOT + 5.0 + name_width as f32 + PILL_PAD;
            if x + GAP + pill > end {
                break;
            }
            x += GAP;
            let colour = rgba(label_colour(
                &label.name,
                label.color.as_deref().and_then(Rgb::from_hex),
                palette.accent_hue,
            ));
            let frame = graphene::Rect::new(x, middle - PILL_HEIGHT / 2.0, pill, PILL_HEIGHT);
            let outline = gtk::gsk::RoundedRect::from_rect(frame, PILL_HEIGHT / 2.0);
            snapshot.append_border(&outline, &[1.0; 4], &[palette.rule; 4]);
            let dot =
                graphene::Rect::new(x + PILL_PAD, middle - PILL_DOT / 2.0, PILL_DOT, PILL_DOT);
            snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(dot, PILL_DOT / 2.0));
            snapshot.append_color(&colour, &dot);
            snapshot.pop();
            self.put(
                snapshot,
                &name,
                x + PILL_PAD + PILL_DOT + 5.0,
                middle,
                name_width as f32,
                &palette.ink,
            );
            drawn.pills.push((label.name.clone(), colour));
            x += pill;
        }

        // The first line, dimmed, in whatever room is left.
        if let Some(preview) = summary.representative.preview.as_deref()
            && end - (x + GAP) >= LEAST_FIRST_LINE
        {
            let preview_layout = self.layout(preview, false, 1.0);
            self.put(
                snapshot,
                &preview_layout,
                x + GAP,
                middle,
                end - (x + GAP),
                &palette.dim,
            );
            drawn.texts.push(preview.to_owned());
        }
        drawn.texts.extend(trailing_texts.into_iter().rev());
        if let Some(marker) = &summary.marker {
            self.draw_marker(snapshot, marker, &palette, &mut drawn);
        }
        self.imp().drawn.replace(drawn);
    }

    /// The second line of a marked row: the accent dot in the gutter, the
    /// kind chip, the date, the quoted sentence, and on the right the
    /// actions that answer it with their keys -- or what is true instead.
    fn draw_marker(
        &self,
        snapshot: &gtk::Snapshot,
        marker: &postio_model::listing::MarkerSummary,
        palette: &Palette,
        drawn: &mut Drawn,
    ) {
        let line = marker_line(marker, chrono::Utc::now(), &chrono::Local);
        let width = self.width() as f32;
        drawn.accent = Some(palette.accent);

        let dot = graphene::Rect::new(GUTTER_CENTRE - DOT / 2.0, FIRST_LINE - DOT / 2.0, DOT, DOT);
        snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(dot, DOT / 2.0));
        snapshot.append_color(&palette.accent, &dot);
        snapshot.pop();

        // The actions first, from the right edge in, so the quote knows its
        // room.
        let keymap = self.imp().keymap.borrow().clone();
        let mut right = width - TRAILING;
        let mut actions = Vec::new();
        for (command, words) in line.actions.iter().rev() {
            let key = keymap
                .as_ref()
                .and_then(|keymap| postio_ui::hints::key(&keymap.borrow(), *command));
            let label = self.layout(words, true, 0.92);
            let (label_width, _) = label.pixel_size();
            let cap = key.as_deref().map(|key| {
                let cap = self.mono(key, 0.72);
                let (cap_width, _) = cap.pixel_size();
                (cap, cap_width as f32 + 2.0 * KEYCAP_PAD)
            });
            let button = ACTION_PAD
                + label_width as f32
                + cap.as_ref().map_or(0.0, |(_, boxed)| 6.0 + boxed)
                + ACTION_PAD;
            right -= button;
            let frame = graphene::Rect::new(
                right,
                SECOND_LINE - ACTION_HEIGHT / 2.0,
                button,
                ACTION_HEIGHT,
            );
            drawn.actions.push((
                *command,
                (*words).to_owned(),
                [
                    right,
                    SECOND_LINE - ACTION_HEIGHT / 2.0,
                    button,
                    ACTION_HEIGHT,
                ],
            ));
            let outline = gtk::gsk::RoundedRect::from_rect(frame, 5.0);
            snapshot.push_rounded_clip(&outline);
            snapshot.append_color(&palette.raised, &frame);
            snapshot.pop();
            snapshot.append_border(&outline, &[1.0; 4], &[palette.rule; 4]);
            self.put(
                snapshot,
                &label,
                right + ACTION_PAD,
                SECOND_LINE,
                label_width as f32,
                &palette.ink,
            );
            let mut said = vec![(*words).to_owned()];
            if let (Some((cap, boxed)), Some(key)) = (cap, key) {
                let x = right + ACTION_PAD + label_width as f32 + 6.0;
                let (_, cap_height) = cap.pixel_size();
                let frame = graphene::Rect::new(
                    x,
                    SECOND_LINE - (cap_height as f32 + 2.0) / 2.0,
                    boxed,
                    cap_height as f32 + 2.0,
                );
                snapshot.append_border(
                    &gtk::gsk::RoundedRect::from_rect(frame, 3.0),
                    &[1.0; 4],
                    &[palette.rule; 4],
                );
                self.put(
                    snapshot,
                    &cap,
                    x + KEYCAP_PAD,
                    SECOND_LINE,
                    boxed,
                    &palette.dim,
                );
                said.push(key);
            }
            actions.push(said);
            right -= GAP;
        }
        if let Some(status) = line.status {
            let layout = self.layout(status, false, 0.92);
            let (status_width, _) = layout.pixel_size();
            right -= status_width as f32;
            self.put(
                snapshot,
                &layout,
                right,
                SECOND_LINE,
                status_width as f32,
                &palette.dim,
            );
            right -= GAP;
            drawn.texts.push(status.to_owned());
        }

        // The chip, the date and the quote, left to right, in the accent.
        let mut x = SUBJECT_X;
        let chip = self.layout(line.chip, true, 0.85);
        let (chip_width, chip_height) = chip.pixel_size();
        let boxed = chip_width as f32 + 12.0;
        let frame = graphene::Rect::new(
            x,
            SECOND_LINE - (chip_height as f32 + 4.0) / 2.0,
            boxed,
            chip_height as f32 + 4.0,
        );
        snapshot.append_border(
            &gtk::gsk::RoundedRect::from_rect(frame, 4.0),
            &[1.0; 4],
            &[palette.accent; 4],
        );
        self.put(
            snapshot,
            &chip,
            x + 6.0,
            SECOND_LINE,
            chip_width as f32,
            &palette.accent,
        );
        drawn.texts.push(line.chip.to_owned());
        x += boxed + 10.0;
        if let Some(date) = &line.date {
            let layout = self.layout(date, true, 0.95);
            x += self.put(
                snapshot,
                &layout,
                x,
                SECOND_LINE,
                (right - x).max(0.0),
                &palette.accent,
            ) + 10.0;
            drawn.texts.push(date.clone());
        }
        if let Some(quote) = &line.quote
            && right - x >= LEAST_FIRST_LINE
        {
            let quoted = format!("\u{201c}{quote}\u{201d}");
            let layout = self.layout(&quoted, false, 0.95);
            let attributes = layout.attributes().unwrap_or_default();
            attributes.insert(pango::AttrInt::new_style(pango::Style::Italic));
            layout.set_attributes(Some(&attributes));
            self.put(
                snapshot,
                &layout,
                x,
                SECOND_LINE,
                right - x,
                &palette.accent,
            );
            drawn.texts.push(quoted);
        }
        for said in actions.into_iter().rev() {
            drawn.texts.extend(said);
        }
    }

    /// A layout of `text` in the monospace face keys are set in (FR-093).
    fn mono(&self, text: &str, scale: f64) -> pango::Layout {
        let layout = self.layout(text, false, scale);
        let attributes = layout.attributes().unwrap_or_default();
        attributes.insert(pango::AttrString::new_family("Adwaita Mono"));
        layout.set_attributes(Some(&attributes));
        layout
    }

    /// Draw the symbolic icon `name`, `ICON` pixels square, at `x`, `y`, in
    /// `colour`.
    fn draw_icon(&self, snapshot: &gtk::Snapshot, name: &str, x: f32, y: f32, colour: &gdk::RGBA) {
        let theme = gtk::IconTheme::for_display(&self.display());
        let icon = theme.lookup_icon(
            name,
            &[],
            ICON as i32,
            self.scale_factor(),
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        );
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        icon.snapshot_symbolic(snapshot, f64::from(ICON), f64::from(ICON), &[*colour]);
        snapshot.restore();
    }

    /// A row whose page has not landed: two quiet bars where the sender and
    /// the subject will be, drawn from nothing but the row's own size, so a
    /// jump's 205 binds cost no read (spike S3).
    fn draw_skeleton(&self, snapshot: &gtk::Snapshot) {
        let mut shade = self.color();
        shade.set_alpha(shade.alpha() * 0.08);
        let middle = ONE_LINE as f32 / 2.0;
        let room = (self.width() as f32 - SUBJECT_X - TRAILING).max(0.0);
        for (x, width) in [(SENDER_X, SENDER_WIDTH * 0.6), (SUBJECT_X, room * 0.45)] {
            snapshot.append_color(&shade, &graphene::Rect::new(x, middle - 5.0, width, 10.0));
        }
    }
}

/// What a row says, in the order a screen reader should say it: the sender,
/// the subject, the first line, and whether it is unread (FR-096).
pub fn spoken(item: &FocusRow) -> String {
    let row = item.row();
    let summary = &row.summary;
    let mut parts = Vec::new();
    if let Some(from) = &summary.representative.from {
        parts.push(from.display().to_owned());
    }
    if let Some(subject) = &summary.representative.subject {
        parts.push(subject.clone());
    }
    if let Some(preview) = &summary.representative.preview {
        parts.push(preview.clone());
    }
    if summary.has_unread() {
        parts.push("unread".to_owned());
    }
    parts.join(", ")
}
