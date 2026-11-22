//! Zoom the message only: browser-style steps that keep the reader's place.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use postio_config::ZOOM_STEPS;

use super::BodyView;

pub struct ZoomIndicator {
    button: gtk::Button,
}

impl ZoomIndicator {
    pub fn new(view: &BodyView) -> ZoomIndicator {
        let button = gtk::Button::builder()
            .tooltip_text("Actual size")
            .visible(false)
            .build();
        button.add_css_class("flat");
        let weak = view.downgrade();
        button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.zoom_reset();
            }
        });
        let shown = button.downgrade();
        view.connect_local("zoom-changed", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            if let Some(button) = shown.upgrade() {
                show(&button, view.zoom());
            }
            None
        });
        show(&button, view.zoom());
        ZoomIndicator { button }
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub fn label(&self) -> String {
        self.button
            .label()
            .map(|l| l.to_string())
            .unwrap_or_default()
    }

    pub fn reset(&self) {
        self.button.emit_clicked();
    }
}

fn show(button: &gtk::Button, zoom: u16) {
    button.set_label(&format!("{zoom}%"));
    button.set_visible(zoom != 100);
}

pub(super) fn install(view: &BodyView) {
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll({
        let view = view.downgrade();
        move |controller, _, dy| {
            let Some(view) = view.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let pointer = view
                .imp()
                .pointer
                .get()
                .unwrap_or(gtk::graphene::Point::new(0.0, 0.0));
            view.scroll_zoom(dy, pointer);
            glib::Propagation::Stop
        }
    });
    view.add_controller(scroll);

    let pinch = gtk::GestureZoom::new();
    pinch.connect_scale_changed({
        let view = view.downgrade();
        move |_, scale| {
            if let Some(view) = view.upgrade() {
                view.pinch_update(scale);
            }
        }
    });
    pinch.connect_end({
        let view = view.downgrade();
        move |_, _| {
            if let Some(view) = view.upgrade() {
                view.pinch_end();
            }
        }
    });
    view.add_controller(pinch);
}

impl BodyView {
    pub fn zoom(&self) -> u16 {
        self.imp().zoom.get()
    }

    pub fn zoom_in(&self) {
        self.step(true, None);
    }

    pub fn zoom_out(&self) {
        self.step(false, None);
    }

    pub fn zoom_reset(&self) {
        self.set_zoom(100, None);
    }

    pub fn set_zoom_percent(&self, percent: u16) {
        self.set_zoom(postio_config::nearest_zoom(percent), None);
    }

    #[doc(hidden)]
    pub fn scroll_zoom(&self, dy: f64, pointer: gtk::graphene::Point) {
        if dy != 0.0 {
            self.step(dy < 0.0, Some(pointer));
        }
    }

    #[doc(hidden)]
    pub fn pinch_update(&self, scale: f64) {
        self.imp().pinch.set(Some(scale));
        self.queue_draw();
    }

    #[doc(hidden)]
    pub fn pinch_end(&self) {
        let Some(scale) = self.imp().pinch.take() else {
            return;
        };
        let target = (f64::from(self.zoom()) * scale)
            .round()
            .clamp(0.0, f64::from(u16::MAX));
        self.set_zoom(
            postio_config::nearest_zoom(target as u16),
            self.imp().pointer.get(),
        );
        self.queue_draw();
    }

    #[doc(hidden)]
    pub fn pinching(&self) -> bool {
        self.imp().pinch.get().is_some()
    }

    fn step(&self, larger: bool, pointer: Option<gtk::graphene::Point>) {
        let now = self.zoom();
        let next = if larger {
            ZOOM_STEPS.iter().copied().find(|step| *step > now)
        } else {
            ZOOM_STEPS.iter().rev().copied().find(|step| *step < now)
        };
        if let Some(next) = next {
            self.set_zoom(next, pointer);
        }
    }

    fn set_zoom(&self, percent: u16, pointer: Option<gtk::graphene::Point>) {
        let imp = self.imp();
        if imp.zoom.get() == percent {
            return;
        }
        if let Some(document) = self.document() {
            let top = self.window().0;
            let anchor = match pointer {
                Some(at) => document
                    .text
                    .hit(self.document_point(at))
                    .map(|offset| (offset, f64::from(at.y()))),
                None => None,
            }
            .unwrap_or((document.text.char_at_top(top), 0.0));
            imp.anchor.set(Some(anchor));
        }
        imp.zoom.set(percent);
        self.emit_by_name::<()>("zoom-changed", &[]);
        self.request_render();
    }
}
