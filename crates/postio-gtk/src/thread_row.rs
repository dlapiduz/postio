//! One message in the thread column, drawn in a single `snapshot()`.
//!
//! # Why this is not four labels in a box
//!
//! It was, and `postio-p44` measured what that cost. A `GtkListView` keeps a
//! read-ahead window of about 205 rows ready — not a screenful, and not the
//! whole model either (a 5,000-item model builds the same 205). Filling that
//! window is a one-off cost paid when a list is first populated, and it is
//! paid in *widgets*:
//!
//! | Row shape | Filling the window |
//! |---|---|
//! | four labels in a `GtkBox` | 18.3 ms |
//! | one widget | 6.8 ms |
//!
//! Measured on this machine at load average 2, so those are real numbers
//! rather than a loaded machine's noise. 2.7x, and the difference between
//! sitting inside a 16 ms frame and not.
//!
//! `crate::row::MessageRowView` already does this for the message list, which
//! is why the message list was never the problem `postio-p44` thought it was.
//! This is the same trade one surface over: give up the cascade for the row's
//! internals, and hand it back through a probe that reads the colours and
//! fonts out of CSS.
//!
//! # How the styling still comes from CSS
//!
//! Nothing here hard-codes a colour. An invisible [`gtk::Label`] is parented
//! to the row and has its classes swapped; reading `color()` off it each time
//! gives the value the stylesheet would have applied, under whatever theme,
//! density and contrast the window is currently in. Painted backgrounds are
//! read the same way, from classes whose `color` is the value to paint —
//! `crate::row` established the convention and this follows it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, pango};

use crate::list::Row;

/// The row's own inset, matching what the `GtkBox` version had in CSS.
const INSET_LEFT: f32 = 15.0;
const INSET_RIGHT: f32 = 18.0;
const PADDING_Y: f32 = 6.0;
/// The steel edge a selected row wears, and the transparent one every other
/// row reserves so the text never shifts sideways under the cursor.
const EDGE: f32 = 3.0;
/// The gap between the row's four columns, from canvas 3a.
const GAP: f32 = 10.0;
/// How wide the index column is. Two digits and a little air; a thread with
/// more than ninety-nine messages simply runs wider here.
const INDEX_WIDTH: f32 = 16.0;
/// How wide the sender column is. The canvas' 104px.
const SENDER_WIDTH: f32 = 104.0;
/// The shortest a row may be, so a thread reads as a transcript rather than
/// as a dense table.
const MIN_HEIGHT: f32 = 32.0;

/// Which set of inks a row is drawn in.
///
/// The two facts that change a row's colours, as an index into the four
/// variants each role has: read/unread, and whether the cursor is on it.
fn tone(selected: bool, unread: bool) -> usize {
    usize::from(unread) + if selected { 2 } else { 0 }
}

/// One text role, at one tone: what to draw it in and what to draw it with.
#[derive(Clone)]
struct Ink {
    color: gdk::RGBA,
    font: pango::FontDescription,
}

/// Every colour and font the row draws with, read out of CSS.
struct Palette {
    index: [Ink; 4],
    sender: [Ink; 4],
    line: [Ink; 4],
    when: [Ink; 4],
    hairline: gdk::RGBA,
    edge: gdk::RGBA,
    ground: gdk::RGBA,
    hover: gdk::RGBA,
}

impl Palette {
    /// Read every role off `probe`, whose parent puts it under the same
    /// `:root` classes the rest of the window is under — so dark and
    /// high-contrast come out right without this knowing they exist.
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

        Palette {
            index: four("postio-thread-index"),
            sender: four("postio-thread-sender"),
            line: four("postio-thread-line"),
            when: four("postio-thread-when"),
            hairline: paint(&["postio-thread-edge", "hairline"]),
            edge: paint(&["postio-thread-edge", "selected"]),
            ground: paint(&["postio-thread-ground", "selected"]),
            hover: paint(&["postio-thread-ground", "hover"]),
        }
    }
}

/// The four text runs, measured for one width and one tone.
struct Laid {
    width: i32,
    tone: usize,
    height: f32,
    index: pango::Layout,
    sender: pango::Layout,
    line: pango::Layout,
    when: pango::Layout,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ThreadRowView {
        pub(super) row: RefCell<Option<Row>>,
        pub(super) index: Cell<u32>,
        pub(super) selected: Cell<bool>,
        pub(super) hovered: Cell<bool>,
        /// Invisible, and the whole reason this widget still follows the
        /// stylesheet. See the module docs.
        pub(super) probe: gtk::Label,
        pub(super) laid: RefCell<Option<Laid>>,
        pub(super) palette: RefCell<Option<Rc<Palette>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThreadRowView {
        const NAME: &'static str = "PostioThreadRowView";
        type Type = super::ThreadRowView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ThreadRowView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_css_class("postio-thread-row");
            // The picture of a row, not the row: the `GtkListItemWidget`
            // around it carries the `ListItem` role and the name, which is
            // what a screen reader navigates the column by.
            obj.set_accessible_role(gtk::AccessibleRole::Presentation);
            obj.set_focusable(false);
            self.probe.set_child_visible(false);
            self.probe.set_parent(&*obj);

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
        }
    }

    impl WidgetImpl for ThreadRowView {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                // A row never asks for more width than it is given; the
                // sender and the subject ellipsize instead. What it insists
                // on is room for the numbers at either end and a few
                // characters between them.
                let least = (INSET_LEFT + INSET_RIGHT + INDEX_WIDTH + GAP * 3.0) as i32 + 80;
                return (least, least, -1, -1);
            }
            let width = if for_size > 0 { for_size } else { 404 };
            let height = self.obj().lay_out(width).ceil() as i32;
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
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

        /// Text scaling, the font, the display: all of them change what the
        /// probe would say, so nothing read off it survives.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.obj().restyle();
        }

        fn state_flags_changed(&self, previous: &gtk::StateFlags) {
            self.parent_state_flags_changed(previous);
            self.laid.replace(None);
            self.obj().queue_draw();
        }
    }
}

glib::wrapper! {
    /// One message in the thread column, drawn in a single `snapshot()`.
    pub struct ThreadRowView(ObjectSubclass<imp::ThreadRowView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ThreadRowView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ThreadRowView {
    /// An empty row, waiting to be bound.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show `row` as the `index`-th message of the thread.
    pub fn set_row(&self, row: Option<Row>, index: u32) {
        let imp = self.imp();
        if *imp.row.borrow() == row && imp.index.get() == index {
            return;
        }
        imp.row.replace(row);
        imp.index.set(index);
        imp.laid.replace(None);
        self.queue_resize();
    }

    /// The message on this row, if it has been bound.
    pub fn row(&self) -> Option<Row> {
        self.imp().row.borrow().clone()
    }

    /// Whether the cursor is on this row.
    /// Whether this row is drawn as the selected one.
    ///
    /// The drawn state, not a caller's intention: the conversation pane
    /// (#308) needs to answer "can the user see which message is current",
    /// and "we called `set_selected`" is a different claim.
    pub fn is_selected(&self) -> bool {
        self.imp().selected.get()
    }

    pub fn set_selected(&self, selected: bool) {
        if self.imp().selected.replace(selected) != selected {
            self.imp().laid.replace(None);
            self.queue_draw();
        }
    }

    /// Whether the pointer is over it.
    ///
    /// Watched with a controller rather than read off a state flag: the
    /// prelight lands on the list item around this, and never reaches down.
    pub fn set_hovered(&self, hovered: bool) {
        if self.imp().hovered.replace(hovered) != hovered {
            self.queue_draw();
        }
    }

    /// How the row reads to a screen reader.
    pub fn spoken(&self) -> String {
        let Some(row) = self.imp().row.borrow().clone() else {
            return String::new();
        };
        format!(
            "{}. {}",
            self.imp().index.get(),
            crate::row::accessible_label(&row)
        )
    }

    /// Throw away everything read from CSS, because CSS may have changed.
    pub fn restyle(&self) {
        self.imp().palette.replace(None);
        self.imp().laid.replace(None);
        self.queue_resize();
    }

    fn palette(&self) -> Rc<Palette> {
        let imp = self.imp();
        if let Some(palette) = imp.palette.borrow().as_ref() {
            return Rc::clone(palette);
        }
        let palette = Rc::new(Palette::read(&imp.probe));
        imp.palette.replace(Some(Rc::clone(&palette)));
        palette
    }

    /// Measure the four runs for `width`, reusing the last measurement when
    /// nothing that would change it has changed.
    fn lay_out(&self, width: i32) -> f32 {
        let imp = self.imp();
        let unread = imp.row.borrow().as_ref().is_some_and(|row| !row.seen);
        let tone = tone(imp.selected.get(), unread);

        if let Some(laid) = imp.laid.borrow().as_ref()
            && laid.width == width
            && laid.tone == tone
        {
            return laid.height;
        }

        let palette = self.palette();
        let laid = self.build(width, &palette, tone);
        let height = laid.height;
        imp.laid.replace(Some(laid));
        height
    }

    fn build(&self, width: i32, palette: &Palette, tone: usize) -> Laid {
        let context = self.pango_context();
        let run = |ink: &Ink, text: &str| {
            let layout = pango::Layout::new(&context);
            layout.set_font_description(Some(&ink.font));
            layout.set_text(text);
            layout
        };
        let row = self.imp().row.borrow().clone();

        let index = run(
            &palette.index[tone],
            &match self.imp().index.get() {
                0 => String::new(),
                n => n.to_string(),
            },
        );
        index.set_alignment(pango::Alignment::Right);
        index.set_width((INDEX_WIDTH * pango::SCALE as f32) as i32);

        let sender = run(
            &palette.sender[tone],
            &row.as_ref().map(sender_name).unwrap_or_default(),
        );
        sender.set_ellipsize(pango::EllipsizeMode::End);
        sender.set_width((SENDER_WIDTH * pango::SCALE as f32) as i32);

        let when = run(
            &palette.when[tone],
            &row.as_ref()
                .map(|row| crate::row::timestamp(row.received_at, chrono::Local::now()))
                .unwrap_or_default(),
        );

        // The subject takes what is left after everything with a fixed width,
        // which is what makes it the thing that ellipsizes.
        let fixed = INSET_LEFT
            + INSET_RIGHT
            + INDEX_WIDTH
            + SENDER_WIDTH
            + GAP * 3.0
            + when.pixel_size().0 as f32;
        let column = (width as f32 - fixed).max(40.0);
        let line = run(
            &palette.line[tone],
            row.as_ref()
                .and_then(|row| row.subject.as_deref())
                .map(str::trim)
                .filter(|subject| !subject.is_empty())
                .unwrap_or("(no subject)"),
        );
        line.set_ellipsize(pango::EllipsizeMode::End);
        line.set_width((column * pango::SCALE as f32) as i32);

        let tallest = [&index, &sender, &line, &when]
            .iter()
            .map(|layout| layout.pixel_size().1 as f32)
            .fold(0.0, f32::max);
        let height = (tallest + PADDING_Y * 2.0).max(MIN_HEIGHT);

        Laid {
            width,
            tone,
            height,
            index,
            sender,
            line,
            when,
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
        let laid = imp.laid.borrow();
        let Some(laid) = laid.as_ref() else {
            return;
        };
        let height = self.height() as f32;
        let selected = imp.selected.get();

        let rect = |x: f32, y: f32, w: f32, h: f32| graphene::Rect::new(x, y, w, h);

        // The cursor row: the canvas' accent tint, and the 3px steel edge the
        // folder list and the message list both use. One selection idiom
        // across the application, painted here rather than cascaded because
        // this widget draws its own pixels.
        if selected {
            snapshot.append_color(&palette.ground, &rect(0.0, 0.0, width, height));
            snapshot.append_color(&palette.edge, &rect(0.0, 0.0, EDGE, height));
        } else if imp.hovered.get() {
            snapshot.append_color(&palette.hover, &rect(0.0, 0.0, width, height));
        }

        // The hairline under every row, which is what makes the column read
        // as a transcript rather than as a second inbox.
        snapshot.append_color(&palette.hairline, &rect(0.0, height - 1.0, width, 1.0));

        let text = |layout: &pango::Layout, ink: &Ink, x: f32| {
            let y = ((height - layout.pixel_size().1 as f32) / 2.0).max(0.0);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x, y));
            snapshot.append_layout(layout, &ink.color);
            snapshot.restore();
        };

        let tone = laid.tone;
        let mut x = INSET_LEFT;
        text(&laid.index, &palette.index[tone], x);
        x += INDEX_WIDTH + GAP;
        text(&laid.sender, &palette.sender[tone], x);
        x += SENDER_WIDTH + GAP;
        text(&laid.line, &palette.line[tone], x);

        let when_width = laid.when.pixel_size().0 as f32;
        text(
            &laid.when,
            &palette.when[tone],
            (width - INSET_RIGHT - when_width).max(x),
        );
    }
}

/// Who a message is from, as one line: the display name, the address when
/// there is no name, or a dash when there is neither.
///
/// Shared with the box-drawing tree so both say the same thing about the
/// same message.
pub fn sender_name(row: &Row) -> String {
    match row.from.as_ref() {
        Some(from) => from
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| from.address.clone()),
        None => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{TimeZone, Utc};
    use postio_model::EmailAddress;
    use postio_model::ids::{MessageId, ThreadId};

    fn message(from: Option<EmailAddress>) -> Row {
        Row {
            id: MessageId::new(1),
            thread: Some(ThreadId::new(1)),
            from,
            subject: Some("Re: index rebuild".to_owned()),
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 0, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 3,
            participants: Vec::new(),
        }
    }

    #[test]
    fn a_sender_with_no_name_is_shown_by_address() {
        let row = message(Some(EmailAddress::new(
            None::<String>,
            "buildbot@example.net",
        )));
        assert_eq!(sender_name(&row), "buildbot@example.net");
    }

    #[test]
    fn a_message_from_nobody_still_has_something_in_the_column() {
        assert_eq!(sender_name(&message(None)), "—");
    }

    #[test]
    fn the_tone_index_covers_every_combination_exactly_once() {
        let mut seen: Vec<usize> = [(false, false), (false, true), (true, false), (true, true)]
            .into_iter()
            .map(|(selected, unread)| tone(selected, unread))
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3], "four states, four inks, no collisions");
    }
}
