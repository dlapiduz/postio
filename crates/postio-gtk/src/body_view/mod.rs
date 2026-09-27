//! The reading surface (spec 006 research R8): a `gtk::Scrollable` that
//! paints a `postio-render` snapshot as tiles. It never touches the engine:
//! a render thread lays the message out, and this draws what comes back.
//!
//! Everything interactive -- selection, find, links, the rail -- is a pure
//! function over the snapshot (research R7), added by the stories that need
//! it; this is the surface they share.

mod tiles;

use std::cell::{Cell, RefCell};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_render::fonts::{Bundled, FontSet};
use postio_render::{RenderRequest, RenderedDocument, Renderer, Resources, Theme, Viewport};

pub use postio_render::DEFAULT_RENDER_DEADLINE;

/// What a `BodyView` shows: a composed document and everything it may load.
#[derive(Clone)]
pub struct Content {
    /// The composed HTML (`postio_ui::reader::document`).
    pub document: String,
    /// The parts and faces the document may name; nothing else loads.
    pub resources: Arc<Resources>,
    /// The plain-text alternative, drawn if the render falls back.
    pub plain_text: String,
    /// The input cap the sanitizer found the body over, if any.
    pub over_cap: Option<postio_body::Cap>,
}

/// The process's font set: the bundled faces, then everything installed.
/// Built once; [`prewarm_fonts`] starts it off the UI thread at startup.
pub fn font_set() -> &'static FontSet {
    static FONTS: OnceLock<FontSet> = OnceLock::new();
    FONTS.get_or_init(|| {
        FontSet::new(Bundled {
            faces: postio_ui::reader::document::FACES
                .iter()
                .map(|face| face.bytes)
                .collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

/// Build the font set on a thread of its own, so the first message opened
/// does not wait for font discovery.
pub fn prewarm_fonts() {
    std::thread::spawn(|| {
        font_set();
    });
}

mod imp {
    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::BodyView)]
    pub struct BodyView {
        #[property(override_interface = gtk::Scrollable, get, set = Self::set_hadjustment, nullable)]
        pub(super) hadjustment: RefCell<Option<gtk::Adjustment>>,
        #[property(override_interface = gtk::Scrollable, get, set = Self::set_vadjustment, nullable)]
        pub(super) vadjustment: RefCell<Option<gtk::Adjustment>>,
        #[property(override_interface = gtk::Scrollable, get, set, builder(gtk::ScrollablePolicy::Minimum))]
        pub(super) hscroll_policy: Cell<gtk::ScrollablePolicy>,
        #[property(override_interface = gtk::Scrollable, get, set, builder(gtk::ScrollablePolicy::Minimum))]
        pub(super) vscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub(super) renderer: OnceLock<Renderer>,
        pub(super) deadline: Cell<Duration>,
        pub(super) content: RefCell<Option<Content>>,
        /// The newest generation asked for.
        pub(super) generation: Cell<u64>,
        /// The width the current request or snapshot was laid out at.
        pub(super) laid_out_width: Cell<i32>,
        /// What is on screen.
        pub(super) document: RefCell<Option<Arc<RenderedDocument>>>,
        pub(super) tiles: RefCell<tiles::Tiles>,
    }

    impl Default for BodyView {
        fn default() -> Self {
            BodyView {
                hadjustment: RefCell::default(),
                vadjustment: RefCell::default(),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                renderer: OnceLock::new(),
                deadline: Cell::new(DEFAULT_RENDER_DEADLINE),
                content: RefCell::default(),
                generation: Cell::new(0),
                laid_out_width: Cell::new(0),
                document: RefCell::default(),
                tiles: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BodyView {
        const NAME: &'static str = "PostioBodyView";
        type Type = super::BodyView;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for BodyView {}

    impl WidgetImpl for BodyView {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if width != self.laid_out_width.get() && self.content.borrow().is_some() {
                self.obj().request_render();
            }
            self.obj().configure_adjustments();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }

    impl ScrollableImpl for BodyView {}

    impl BodyView {
        fn set_hadjustment(&self, adjustment: Option<gtk::Adjustment>) {
            self.hadjustment.replace(adjustment);
        }

        fn set_vadjustment(&self, adjustment: Option<gtk::Adjustment>) {
            if let Some(adjustment) = &adjustment {
                let view = self.obj().downgrade();
                adjustment.connect_value_changed(move |_| {
                    if let Some(view) = view.upgrade() {
                        view.queue_draw();
                    }
                });
            }
            self.vadjustment.replace(adjustment);
            self.obj().configure_adjustments();
        }
    }
}

glib::wrapper! {
    /// The reading surface: a snapshot drawn as tiles.
    pub struct BodyView(ObjectSubclass<imp::BodyView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl BodyView {
    /// A view whose renders fall back after `deadline`: production passes
    /// [`DEFAULT_RENDER_DEADLINE`], tests a scaled or tiny one.
    pub fn new(deadline: Duration) -> BodyView {
        let view: BodyView = glib::Object::new();
        view.imp().deadline.set(deadline);
        view
    }

    /// Show `content`, re-rendering at the current width.
    pub fn set_content(&self, content: Content) {
        self.imp().content.replace(Some(content));
        self.request_render();
    }

    /// The snapshot on screen, if one has arrived.
    pub fn document(&self) -> Option<Arc<RenderedDocument>> {
        self.imp().document.borrow().clone()
    }

    /// Drop every tile, as memory pressure would: what shows until they
    /// are drawn again is the snapshot's low-resolution copy.
    #[doc(hidden)]
    pub fn evict_tiles(&self) {
        if let Some(document) = self.imp().document.borrow().clone() {
            self.imp().tiles.borrow_mut().reset(document);
        }
        self.queue_draw();
    }

    fn renderer(&self) -> &Renderer {
        self.imp()
            .renderer
            .get_or_init(|| Renderer::new(font_set()))
    }

    /// The surface's fractional scale: 1.0, 1.25, 2.0. Painting at 1.0 and
    /// letting the compositor stretch it is what looked blurry on HiDPI.
    fn surface_scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
    }

    fn request_render(&self) {
        let imp = self.imp();
        let Some(content) = imp.content.borrow().clone() else {
            return;
        };
        let width = self.width();
        if width <= 0 {
            return;
        }
        imp.laid_out_width.set(width);
        let generation = imp.generation.get() + 1;
        imp.generation.set(generation);
        let style = adw::StyleManager::default();
        let request = RenderRequest {
            generation,
            document: content.document,
            plain_text: content.plain_text,
            over_cap: content.over_cap,
            resources: content.resources,
            viewport: Viewport {
                width: f64::from(width),
                hidpi_scale: self.surface_scale(),
                zoom: 1.0,
            },
            theme: Theme {
                dark: style.is_dark(),
                high_contrast: style.is_high_contrast(),
            },
            darkened: Vec::new(),
            toggled_folds: Vec::new(),
            reader_view: Vec::new(),
        };
        let result = self.renderer().request(request);
        let view = self.downgrade();
        glib::timeout_add_local(Duration::from_millis(4), move || {
            let Some(view) = view.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match result.try_recv() {
                Ok(document) => {
                    view.show(document);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn show(&self, document: RenderedDocument) {
        let imp = self.imp();
        if document.generation != imp.generation.get() {
            return;
        }
        let document = Arc::new(document);
        imp.tiles.borrow_mut().reset(document.clone());
        imp.document.replace(Some(document));
        self.configure_adjustments();
        self.queue_draw();
    }

    fn configure_adjustments(&self) {
        let imp = self.imp();
        let height = f64::from(self.height());
        let upper = imp
            .document
            .borrow()
            .as_ref()
            .map_or(height, |doc| doc.size.height);
        if let Some(adjustment) = imp.vadjustment.borrow().as_ref() {
            let value = adjustment.value().min((upper - height).max(0.0));
            adjustment.configure(value, 0.0, upper, 40.0, height * 0.9, height);
        }
        let width = f64::from(self.width());
        let wide = imp
            .document
            .borrow()
            .as_ref()
            .map_or(width, |doc| doc.size.width.max(width));
        if let Some(adjustment) = imp.hadjustment.borrow().as_ref() {
            let value = adjustment.value().min((wide - width).max(0.0));
            adjustment.configure(value, 0.0, wide, 40.0, width * 0.9, width);
        }
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let Some(document) = imp.document.borrow().clone() else {
            return;
        };
        let top = imp.vadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
        let left = imp.hadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
        let (width, height) = (f64::from(self.width()), f64::from(self.height()));
        snapshot.push_clip(&gtk::graphene::Rect::new(
            0.0,
            0.0,
            width as f32,
            height as f32,
        ));
        let view = self.downgrade();
        imp.tiles
            .borrow_mut()
            .draw(snapshot, &document, left, top, height, move || {
                if let Some(view) = view.upgrade() {
                    view.queue_draw();
                }
            });
        snapshot.pop();
    }
}
