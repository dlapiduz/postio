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

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_ui::focus_row::{MAX_PILLS, count_badge};
use postio_ui::label_colour::{Rgb, label_colour};

use super::item::FocusRow;
use super::model::RowObject;

/// What a row drew in its last snapshot: every text it laid out, in order,
/// each label pill with its colour, and whether it was drawn bold. What a
/// person sees, for a test to read back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawn {
    /// Every text laid out, in the order drawn.
    pub texts: Vec<String>,
    /// Each label pill: its name and its dot's colour.
    pub pills: Vec<(String, gdk::RGBA)>,
    /// Whether the sender and subject were bold: an unread conversation.
    pub bold: bool,
}

/// A one-line row's height, in pixels: the classic row's (research R3).
pub const ONE_LINE: i32 = 40;

/// A two-line row's height: a conversation with a marker.
pub const TWO_LINES: i32 = 72;

/// Where the sender column starts, and how wide it is (focus-surface.md).
const SENDER_X: f32 = 56.0;
const SENDER_WIDTH: f32 = 222.0;
/// Where the subject column starts.
const SUBJECT_X: f32 = 290.0;
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

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowWidget {
        pub item: RefCell<Option<FocusRow>>,
        pub bound: RefCell<Option<(RowObject, glib::SignalHandlerId)>>,
        pub drawn: RefCell<Drawn>,
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
/// quieter steps libadwaita takes of it; and the accent's hue, from
/// `AdwStyleManager`, which the label colours keep away from (FR-091).
struct Palette {
    ink: gdk::RGBA,
    dim: gdk::RGBA,
    rule: gdk::RGBA,
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
        let accent = adw::StyleManager::default().accent_color_rgba();
        Palette {
            ink,
            dim: faded(0.55),
            rule: faded(0.18),
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
        let FocusRow::Conversation(row) = &item;
        let summary = &row.summary;
        let palette = Palette::of(self);
        let width = self.width() as f32;
        let middle = ONE_LINE as f32 / 2.0;
        let bold = summary.has_unread();
        let mut drawn = Drawn {
            bold,
            ..Drawn::default()
        };

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
        self.imp().drawn.replace(drawn);
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
    let FocusRow::Conversation(row) = item;
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
