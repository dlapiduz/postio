//! Selection and copy over the snapshot (spec 006 FR-017, research R7):
//! positions in the text index, never the engine's own selection, drawn as
//! overlay rectangles so a selection change rasterises nothing.

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::BodyView;

/// The selection highlight: the accent, translucent over the page.
const HIGHLIGHT: gdk::RGBA = gdk::RGBA::new(0.36, 0.56, 0.80, 0.35);

/// Wire the pointer to the selection.
pub(super) fn install(view: &BodyView) {
    let drag = gtk::GestureDrag::new();
    drag.connect_drag_begin({
        let view = view.downgrade();
        move |_, x, y| {
            if let Some(view) = view.upgrade() {
                let at = gtk::graphene::Point::new(x as f32, y as f32);
                view.imp().drag_start.set(Some(at));
            }
        }
    });
    drag.connect_drag_update({
        let view = view.downgrade();
        move |_, dx, dy| {
            let Some(view) = view.upgrade() else { return };
            let Some(start) = view.imp().drag_start.get() else {
                return;
            };
            let to = gtk::graphene::Point::new(start.x() + dx as f32, start.y() + dy as f32);
            view.select_points(start, to, false);
            autoscroll(&view, to.y());
        }
    });
    drag.connect_drag_end({
        let view = view.downgrade();
        move |_, _, _| {
            if let Some(view) = view.upgrade() {
                view.imp().drag_start.set(None);
                view.offer_primary();
            }
        }
    });
    view.add_controller(drag);

    let click = gtk::GestureClick::new();
    click.connect_pressed({
        let view = view.downgrade();
        move |_, presses, x, y| {
            if let Some(view) = view.upgrade() {
                view.click_select(gtk::graphene::Point::new(x as f32, y as f32), presses);
            }
        }
    });
    view.add_controller(click);

    let shortcuts = gtk::ShortcutController::new();
    for trigger in ["<Control>c", "<Control>Insert"] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(gtk::NamedAction::new("clipboard.copy")),
        ));
    }
    view.add_controller(shortcuts);
}

/// Scroll a drag that reaches the view's edge.
fn autoscroll(view: &BodyView, y: f32) {
    let Some(adjustment) = view.imp().vadjustment.borrow().clone() else {
        return;
    };
    let height = view.height() as f32;
    let step = if y < 24.0 {
        -24.0_f64
    } else if y > height - 24.0 {
        24.0
    } else {
        return;
    };
    adjustment.set_value(adjustment.value() + step);
}

impl BodyView {
    /// A point in the view as a point in the document.
    pub(super) fn document_point(&self, at: gtk::graphene::Point) -> postio_render::Point {
        let imp = self.imp();
        let value = |a: &std::cell::RefCell<Option<gtk::Adjustment>>| {
            a.borrow().as_ref().map_or(0.0, |a| a.value())
        };
        postio_render::Point::new(
            f64::from(at.x()) + value(&imp.hadjustment),
            f64::from(at.y()) + value(&imp.vadjustment),
        )
    }

    /// Select from `from` to `to`, and offer it as the primary selection
    /// when `settled`.
    pub(super) fn select_points(
        &self,
        from: gtk::graphene::Point,
        to: gtk::graphene::Point,
        settled: bool,
    ) {
        let Some(document) = self.document() else {
            return;
        };
        let (Some(a), Some(b)) = (
            document.text.hit(self.document_point(from)),
            document.text.hit(self.document_point(to)),
        ) else {
            return;
        };
        self.set_selection((a != b).then(|| a.min(b)..a.max(b)));
        if settled {
            self.offer_primary();
        }
    }

    pub(super) fn set_selection(&self, range: Option<std::ops::Range<usize>>) {
        self.imp().selection.replace(range);
        self.queue_draw();
    }

    /// The selection as X11 and Wayland's primary selection.
    pub(super) fn offer_primary(&self) {
        if let Some(text) = self.selected_text() {
            self.primary_clipboard().set_text(&text);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let range = self.selection()?;
        Some(self.document()?.text.slice(range).to_owned())
    }

    /// Copy the selection to the clipboard.
    pub(super) fn copy(&self) {
        if let Some(text) = self.selected_text() {
            self.clipboard().set_text(&text);
        }
    }

    /// Draw the selection over the tiles.
    pub(super) fn draw_selection(&self, snapshot: &gtk::Snapshot, left: f64, top: f64) {
        for rect in self.selection_rects() {
            snapshot.append_color(
                &HIGHLIGHT,
                &gtk::graphene::Rect::new(
                    (rect.x0 - left) as f32,
                    (rect.y0 - top) as f32,
                    rect.width() as f32,
                    rect.height() as f32,
                ),
            );
        }
    }
}
