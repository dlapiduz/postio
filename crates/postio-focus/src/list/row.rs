//! One row of Focus's list, drawn in one `snapshot()`.
//!
//! Nested boxes per row are the usual reason a GTK list feels slow
//! (`/gtk-design` §4), and a jump rebinds GTK's whole resident window, about
//! 205 rows, in one frame (spike S3, T009). So a row is one widget that lays
//! out its text with Pango and draws it, and binding one is replacing what it
//! holds.

use std::cell::RefCell;

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::item::FocusRow;
use super::model::RowObject;

/// A one-line row's height, in pixels: the classic row's (research R3).
pub const ONE_LINE: i32 = 40;

/// A two-line row's height: a conversation with a marker.
pub const TWO_LINES: i32 = 72;

/// Where the sender column starts, and how wide it is (focus-surface.md).
const SENDER_X: f64 = 56.0;
const SENDER_WIDTH: f64 = 222.0;
/// Where the subject column starts.
const SUBJECT_X: f64 = 290.0;
/// The space the right edge keeps.
const TRAILING: f64 = 24.0;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowWidget {
        pub item: RefCell<Option<FocusRow>>,
        pub bound: RefCell<Option<(RowObject, glib::SignalHandlerId)>>,
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

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let Some(item) = self.imp().item.borrow().clone() else {
            self.draw_skeleton(snapshot);
            return;
        };
        let FocusRow::Conversation(summary) = &item;
        let width = f64::from(self.width());
        let ink = self.color();
        let sender = summary
            .representative
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default();
        let subject = summary.representative.subject.clone().unwrap_or_default();
        let preview = summary.representative.preview.clone().unwrap_or_default();

        let line = |text: &str, x: f64, room: f64| {
            let layout = self.create_pango_layout(Some(text));
            layout.set_ellipsize(pango::EllipsizeMode::End);
            layout.set_width((room.max(0.0) * f64::from(pango::SCALE)) as i32);
            let (_, height) = layout.pixel_size();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                x as f32,
                ((f64::from(ONE_LINE) - f64::from(height)) / 2.0) as f32,
            ));
            snapshot.append_layout(&layout, &ink);
            snapshot.restore();
        };
        line(&sender, SENDER_X, SENDER_WIDTH);
        line(
            &format!("{subject}  {preview}"),
            SUBJECT_X,
            width - SUBJECT_X - TRAILING,
        );
    }
}

impl RowWidget {
    /// A row whose page has not landed: two quiet bars where the sender and
    /// the subject will be, drawn from nothing but the row's own size, so a
    /// jump's 205 binds cost no read (spike S3).
    fn draw_skeleton(&self, snapshot: &gtk::Snapshot) {
        let mut shade = self.color();
        shade.set_alpha(shade.alpha() * 0.08);
        let middle = (ONE_LINE as f32) / 2.0;
        let room = (self.width() as f32 - SUBJECT_X as f32 - TRAILING as f32).max(0.0);
        for (x, width) in [
            (SENDER_X as f32, (SENDER_WIDTH as f32) * 0.6),
            (SUBJECT_X as f32, room * 0.45),
        ] {
            snapshot.append_color(&shade, &graphene::Rect::new(x, middle - 5.0, width, 10.0));
        }
    }
}

/// What a row says, in the order a screen reader should say it: the sender,
/// the subject, the first line, and whether it is unread (FR-096).
pub fn spoken(item: &FocusRow) -> String {
    let FocusRow::Conversation(summary) = item;
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
