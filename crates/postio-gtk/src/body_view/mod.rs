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
        /// The generation still being rendered, if any: what the deadline
        /// gives up on.
        pub(super) pending: Cell<Option<u64>>,
        /// The width the current request or snapshot was laid out at.
        pub(super) laid_out_width: Cell<i32>,
        /// What is on screen.
        pub(super) document: RefCell<Option<Arc<RenderedDocument>>>,
        pub(super) tiles: RefCell<tiles::Tiles>,
        /// The character to bring back to the top when the next snapshot
        /// arrives: a theme change keeps the reader's place.
        pub(super) anchor: Cell<Option<usize>>,
        /// The messages the user darkened (FR-013a): for this session only,
        /// never stored.
        pub(super) darkened: RefCell<Vec<String>>,
        /// The style manager's handlers, disconnected on dispose.
        pub(super) theme_handlers: RefCell<Vec<glib::SignalHandlerId>>,
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
                pending: Cell::new(None),
                laid_out_width: Cell::new(0),
                document: RefCell::default(),
                tiles: RefCell::default(),
                anchor: Cell::new(None),
                darkened: RefCell::default(),
                theme_handlers: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BodyView {
        const NAME: &'static str = "PostioBodyView";
        type Type = super::BodyView;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            // The fallback notice's way to what was actually sent; the
            // reader connects it to the original-source view.
            klass.install_action("body.view-source", None, |view, _, _| {
                view.emit_by_name::<()>("view-source", &[]);
            });
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for BodyView {
        fn constructed(&self) {
            self.parent_constructed();
            // The theme source (FR-011): the app's dark and high-contrast
            // state, and nothing else.
            let style = adw::StyleManager::default();
            let mut handlers = Vec::new();
            for property in ["dark", "high-contrast"] {
                let view = self.obj().downgrade();
                handlers.push(style.connect_notify_local(Some(property), move |_, _| {
                    if let Some(view) = view.upgrade() {
                        view.theme_changed();
                    }
                }));
            }
            self.theme_handlers.replace(handlers);
        }

        fn dispose(&self) {
            let style = adw::StyleManager::default();
            for handler in self.theme_handlers.take() {
                style.disconnect(handler);
            }
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("view-source").build()])
        }
    }

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

    /// The bytes of tile textures held now.
    #[doc(hidden)]
    pub fn tile_bytes(&self) -> usize {
        self.imp().tiles.borrow().bytes()
    }

    /// Whether the view has drawn at its current scroll position with every
    /// tile it needed.
    #[doc(hidden)]
    pub fn tiles_settled(&self) -> bool {
        let top = self
            .imp()
            .vadjustment
            .borrow()
            .as_ref()
            .map_or(0.0, |a| a.value());
        self.imp().tiles.borrow().settled(top)
    }

    /// Call `f` when the fallback notice's "View source" is chosen.
    pub fn connect_view_source(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("view-source", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            f(&view);
            None
        })
    }

    /// Darken the message on screen, or show it as sent again (FR-013a).
    /// False when there is nothing to darken: not dark, or not paper.
    pub fn toggle_darken(&self) -> bool {
        let Some((scope, _)) = self.darkenable() else {
            return false;
        };
        {
            let mut darkened = self.imp().darkened.borrow_mut();
            match darkened.iter().position(|s| *s == scope) {
                Some(at) => {
                    darkened.remove(at);
                }
                None => darkened.push(scope),
            }
        }
        self.request_render();
        true
    }

    /// `darken_message`'s title for the message on screen, or `None` when
    /// the command does not apply to it.
    pub fn darken_title(&self) -> Option<String> {
        let (_, presentation) = self.darkenable()?;
        Some(
            match presentation {
                postio_render::Presentation::Darkened => "Show as sent",
                _ => "Darken this message",
            }
            .to_owned(),
        )
    }

    /// The message on screen `darken_message` acts on, and how it is shown:
    /// the one a third of the way down the view, if it is paper.
    fn darkenable(&self) -> Option<(String, postio_render::Presentation)> {
        use postio_render::Presentation::{Darkened, Paper};
        let imp = self.imp();
        let document = imp.document.borrow().clone()?;
        let top = imp.vadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
        let line = top + f64::from(self.height()) / 3.0;
        let message = document
            .messages
            .iter()
            .find(|m| m.rect.y0 <= line && line < m.rect.y1)
            .or(document.messages.first())?;
        matches!(message.presentation, Paper | Darkened)
            .then(|| (message.scope.clone(), message.presentation))
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
            darkened: imp.darkened.borrow().clone(),
            toggled_folds: Vec::new(),
            reader_view: Vec::new(),
        };
        let fallback = (request.plain_text.clone(), request.theme, request.viewport);
        let result = self.renderer().request(request);
        imp.pending.set(Some(generation));
        // The deadline (FR-023): if the render is still out when it passes,
        // give up on it and show the plain text instead.
        let view = self.downgrade();
        glib::timeout_add_local_once(imp.deadline.get(), move || {
            let Some(view) = view.upgrade() else { return };
            let imp = view.imp();
            if imp.pending.get() != Some(generation) {
                return;
            }
            view.renderer().abandon(generation);
            let next = generation + 1;
            imp.generation.set(next);
            imp.pending.set(None);
            let (text, theme, viewport) = &fallback;
            let document = view.renderer().fallback(
                text,
                theme,
                *viewport,
                postio_render::FallbackReason::Deadline,
                next,
            );
            view.show(document);
        });
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

    /// The theme changed: re-render, keeping the character at the top.
    fn theme_changed(&self) {
        let imp = self.imp();
        if let Some(document) = imp.document.borrow().as_ref() {
            let top = imp.vadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
            imp.anchor.set(Some(document.text.char_at_top(top)));
        }
        self.request_render();
    }

    fn show(&self, document: RenderedDocument) {
        let imp = self.imp();
        if document.generation != imp.generation.get() {
            return;
        }
        if imp.pending.get() == Some(document.generation) {
            imp.pending.set(None);
        }
        let counts = &document.counts;
        postio_ui::reader::cost::note_snapshot(postio_ui::reader::cost::SnapshotCounts {
            style_passes: u64::from(counts.style_passes),
            nodes: u64::from(counts.nodes),
            repaired_runs: u64::from(counts.repaired_runs),
            resources_unresolved: u64::from(counts.resources_unresolved),
            images_placeholdered: u64::from(counts.images_placeholdered),
            display_list_commands: u64::from(counts.display_list_commands),
        });
        let document = Arc::new(document);
        imp.tiles.borrow_mut().reset(document.clone());
        let anchor = imp.anchor.take().and_then(|offset| {
            document
                .text
                .clusters
                .iter()
                .filter(|c| c.range.end > offset)
                .min_by_key(|c| c.range.start)
                .map(|c| c.rect.y0)
        });
        imp.document.replace(Some(document));
        self.configure_adjustments();
        if let (Some(y), Some(adjustment)) = (anchor, imp.vadjustment.borrow().as_ref()) {
            adjustment.set_value(y);
        }
        self.queue_draw();
    }

    fn configure_adjustments(&self) {
        let imp = self.imp();
        let height = f64::from(self.height());
        let upper = imp
            .document
            .borrow()
            .as_ref()
            .map_or(height, |doc| doc.size.height.max(height));
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
