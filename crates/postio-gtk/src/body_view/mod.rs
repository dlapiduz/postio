//! The reading surface (spec 006 research R8): a `gtk::Scrollable` that
//! paints a `postio-render` snapshot as tiles. It never touches the engine:
//! a render thread lays the message out, and this draws what comes back.
//!
//! Everything interactive -- selection, find, links, the rail -- is a pure
//! function over the snapshot (research R7), added by the stories that need
//! it; this is the surface they share.

mod a11y;
pub mod find;
mod interact;
mod tiles;
pub mod zoom;

use std::cell::{Cell, RefCell};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_render::fonts::{Bundled, FontSet};
use postio_render::{RenderRequest, RenderedDocument, Renderer, Resources, Theme, Viewport};

pub use postio_render::DEFAULT_RENDER_DEADLINE;

/// What follows an external link in place of the desktop's launcher.
type Launcher = Box<dyn Fn(&str)>;

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

pub(super) mod imp {
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
        /// A character, and the height in the view it should come back to,
        /// when the next snapshot arrives: what a theme change or a zoom
        /// keeps in place.
        pub(super) anchor: Cell<Option<(usize, f64)>>,
        /// The zoom, in percent: one of `postio_config::ZOOM_STEPS`.
        pub(super) zoom: Cell<u16>,
        /// The next snapshot starts at the top: a new message, not a redraw.
        pub(super) to_top: Cell<bool>,
        /// The scope `current-message` last reported.
        pub(super) current: RefCell<Option<String>>,
        /// A pinch in progress: its scale so far, drawn over the snapshot
        /// until it ends and one render takes its place.
        pub(super) pinch: Cell<Option<f64>>,
        /// Where the pointer last was, for Ctrl+scroll's anchor.
        pub(super) pointer: Cell<Option<gtk::graphene::Point>>,
        /// The selection, as a range of the text index (FR-017).
        pub(super) selection: RefCell<Option<std::ops::Range<usize>>>,
        /// The folds the user has flipped from how the document sets them.
        pub(super) toggled_folds: RefCell<Vec<String>>,
        /// The link keyboard focus is on, by its index in the snapshot.
        pub(super) focused_link: Cell<Option<usize>>,
        /// What follows an external link; the desktop's launcher unless a
        /// test replaced it.
        pub(super) launcher: RefCell<Option<Launcher>>,
        /// The find in progress (FR-018).
        pub(super) find: RefCell<super::find::FindState>,
        /// Where a drag began, in the view's coordinates.
        pub(super) drag_start: Cell<Option<gtk::graphene::Point>>,
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
                zoom: Cell::new(100),
                pointer: Cell::new(None),
                pinch: Cell::new(None),
                current: RefCell::default(),
                to_top: Cell::new(false),
                darkened: RefCell::default(),
                selection: RefCell::default(),
                drag_start: Cell::new(None),
                find: RefCell::default(),
                toggled_folds: RefCell::default(),
                focused_link: Cell::new(None),
                launcher: RefCell::default(),
                theme_handlers: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BodyView {
        const NAME: &'static str = "PostioBodyView";
        type Type = super::BodyView;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable, gtk::AccessibleText);

        fn class_init(klass: &mut Self::Class) {
            // A document, read as text (FR-020).
            klass.set_accessible_role(gtk::AccessibleRole::Document);
            // The fallback notice's way to what was actually sent; the
            // reader connects it to the original-source view.
            klass.install_action("clipboard.copy", None, |view, _, _| view.copy());
            klass.install_action("body.view-source", None, |view, _, _| {
                view.emit_by_name::<()>("view-source", &[]);
            });
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for BodyView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_focusable(true);
            super::interact::install(&self.obj());
            super::zoom::install(&self.obj());
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
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("view-source").build(),
                    // The zoom changed; its new value is `zoom()`.
                    glib::subclass::Signal::builder("zoom-changed").build(),
                    // A snapshot reached the screen.
                    glib::subclass::Signal::builder("rendered").build(),
                    // The message with most of the view changed: its scope.
                    glib::subclass::Signal::builder("current-message")
                        .param_types([String::static_type()])
                        .build(),
                    // A verb link was followed: the message's scope, and the
                    // verb (`reply`, `forward`, `continue`, `allow`).
                    glib::subclass::Signal::builder("message-verb")
                        .param_types([String::static_type(), String::static_type()])
                        .build(),
                ]
            })
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
                        view.report_current_message();
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
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl BodyView {
    /// A view whose renders fall back after `deadline`: production passes
    /// [`DEFAULT_RENDER_DEADLINE`], tests a scaled or tiny one.
    pub fn new(deadline: Duration) -> BodyView {
        let view: BodyView = glib::Object::new();
        view.imp().deadline.set(deadline);
        view
    }

    /// Show `content` from its top: a different message, not a redraw of
    /// the one on screen.
    pub fn set_content_from_top(&self, content: Content) {
        self.imp().to_top.set(true);
        self.set_content(content);
    }

    /// Show `content` in place of what is on screen, keeping the reader's
    /// place: the character at the top of the view stays where it is, so a
    /// late body or a shown image above it does not move the words being
    /// read.
    pub fn set_content(&self, content: Content) {
        if !self.imp().to_top.get() {
            self.keep_place();
        }
        self.imp().content.replace(Some(content));
        self.request_render();
    }

    /// Remember the character at the top of the view, and how far above the
    /// view's top it starts, for the next snapshot to put back.
    fn keep_place(&self) {
        let imp = self.imp();
        let Some(document) = imp.document.borrow().clone() else {
            return;
        };
        let top = imp.vadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
        let offset = document.text.char_at_top(top);
        let at = document
            .text
            .clusters
            .iter()
            .filter(|c| c.range.end > offset)
            .min_by_key(|c| c.range.start)
            .map_or(0.0, |c| c.rect.y0 - top);
        imp.anchor.set(Some((offset, at)));
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

    /// Open a closed fold, or close an open one, and re-render.
    fn toggle_fold(&self, id: &str) {
        {
            let mut toggled = self.imp().toggled_folds.borrow_mut();
            match toggled.iter().position(|t| t == id) {
                Some(at) => {
                    toggled.remove(at);
                }
                None => toggled.push(id.to_owned()),
            }
        }
        self.request_render();
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
    pub fn darken_title(&self) -> Option<&'static str> {
        let (_, presentation) = self.darkenable()?;
        Some(match presentation {
            postio_render::Presentation::Darkened => "Show as sent",
            _ => "Darken this message",
        })
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

    /// Select from `from` to `to`, in the view's coordinates: what a drag
    /// does.
    #[doc(hidden)]
    pub fn drag_select(&self, from: gtk::graphene::Point, to: gtk::graphene::Point) {
        self.select_points(from, to, true);
    }

    /// Select the word (`presses == 2`) or line (`3`) at `at`.
    #[doc(hidden)]
    pub fn click_select(&self, at: gtk::graphene::Point, presses: i32) {
        let Some(document) = self.document() else {
            return;
        };
        if presses == 1 {
            let point = self.document_point(at);
            // A link first: the reader's verbs sit inside a message's
            // header, which is its fold's summary.
            if let Some(link) = document.link_at(point) {
                self.follow(&link.target.clone());
                return;
            }
            if let Some(fold) = document.fold_at(point) {
                self.toggle_fold(&fold.id.clone());
                return;
            }
        }
        let Some(offset) = document.text.hit(self.document_point(at)) else {
            self.set_selection(None);
            return;
        };
        let range = match presses {
            1 => {
                self.set_selection(None);
                return;
            }
            2 => document.text.word_at(offset),
            3 => document.text.line_at(offset),
            _ => {
                self.set_selection(None);
                return;
            }
        };
        self.set_selection((!range.is_empty()).then_some(range));
        self.offer_primary();
    }

    /// The selected range of the text index.
    pub fn selection(&self) -> Option<std::ops::Range<usize>> {
        self.imp().selection.borrow().clone()
    }

    /// The selection's highlight rectangles, in document coordinates.
    #[doc(hidden)]
    pub fn selection_rects(&self) -> Vec<postio_render::Rect> {
        match (self.selection(), self.document()) {
            (Some(range), Some(document)) => document.text.rects(range),
            _ => Vec::new(),
        }
    }

    /// Follow external links with `launch` instead of the desktop's
    /// launcher: how a test sees what would have opened.
    #[doc(hidden)]
    pub fn set_launcher(&self, launch: impl Fn(&str) + 'static) {
        self.imp().launcher.replace(Some(Box::new(launch)));
    }

    /// Call `f` with a message's scope and verb (`reply`, `forward`,
    /// `continue`, `allow`) when a verb link in it is followed.
    pub fn connect_message_verb(
        &self,
        f: impl Fn(&Self, &str, &str) + 'static,
    ) -> Option<glib::SignalHandlerId> {
        Some(self.connect_local("message-verb", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            let scope = values[1].get::<String>().expect("a scope");
            let verb = values[2].get::<String>().expect("a verb");
            f(&view, &scope, &verb);
            None
        }))
    }

    /// The pointer is at `at`, in the view's coordinates.
    #[doc(hidden)]
    pub fn hover(&self, at: gtk::graphene::Point) {
        let point = self.document_point(at);
        let target = self
            .document()
            .and_then(|doc| doc.link_at(point).map(|link| link.target.describe()));
        self.set_tooltip_text(target.as_deref());
        self.set_cursor_from_name(Some(if target.is_some() { "pointer" } else { "text" }));
    }

    /// Move keyboard focus to the next link (`forward`) or the previous.
    #[doc(hidden)]
    pub fn focus_next_link(&self, forward: bool) {
        let Some(document) = self.document() else {
            return;
        };
        let count = document.links.len();
        if count == 0 {
            return;
        }
        let next = match (self.imp().focused_link.get(), forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(at), true) => (at + 1) % count,
            (Some(at), false) => (at + count - 1) % count,
        };
        self.imp().focused_link.set(Some(next));
        self.scroll_into_view(document.links[next].rect);
        self.queue_draw();
    }

    /// Where the focused link goes, as its tooltip says it.
    #[doc(hidden)]
    pub fn focused_link_target(&self) -> Option<String> {
        let document = self.document()?;
        let link = document.links.get(self.imp().focused_link.get()?)?;
        Some(link.target.describe())
    }

    /// Follow the focused link.
    #[doc(hidden)]
    pub fn activate_focused_link(&self) {
        let Some(document) = self.document() else {
            return;
        };
        if let Some(link) = self
            .imp()
            .focused_link
            .get()
            .and_then(|at| document.links.get(at))
        {
            self.follow(&link.target);
        }
    }

    /// Follow `target`: open it outside, dispatch its verb, or scroll to it.
    ///
    // POSTIO-CONSENT: the system browser opens only when the user follows
    // a link -- a click on it, or Return on the link keyboard focus is on --
    // never on hover, on render or on load, and never for a URL the
    // snapshot did not show as an http, https or mailto link.
    fn follow(&self, target: &postio_render::LinkTarget) {
        use postio_render::LinkTarget;
        match target {
            LinkTarget::External(url) => {
                if let Some(launch) = self.imp().launcher.borrow().as_ref() {
                    launch(url.as_str());
                    return;
                }
                let root = self.root().and_downcast::<gtk::Window>();
                gtk::UriLauncher::new(url.as_str()).launch(
                    root.as_ref(),
                    None::<&gtk::gio::Cancellable>,
                    |_| {},
                );
            }
            LinkTarget::Verb { scope, verb } => {
                self.emit_by_name::<()>("message-verb", &[scope, &verb.name().to_owned()]);
            }
            LinkTarget::Fragment { scope, id } => {
                let Some(document) = self.document() else {
                    return;
                };
                let Some(adjustment) = self.imp().vadjustment.borrow().clone() else {
                    return;
                };
                // The element with that id, or failing that its message's top.
                let top = document
                    .anchors
                    .iter()
                    .find(|(anchor, _)| anchor == id)
                    .map(|(_, y)| *y)
                    .or_else(|| {
                        document
                            .messages
                            .iter()
                            .find(|m| &m.scope == scope)
                            .map(|m| m.rect.y0)
                    });
                if let Some(top) = top {
                    adjustment.set_value(top);
                }
            }
        }
    }

    /// Scroll so `rect`, in document coordinates, is in view.
    pub(super) fn scroll_into_view(&self, rect: postio_render::Rect) {
        let Some(adjustment) = self.imp().vadjustment.borrow().clone() else {
            return;
        };
        let (top, page) = (adjustment.value(), adjustment.page_size());
        if rect.y0 < top {
            adjustment.set_value(rect.y0);
        } else if rect.y1 > top + page {
            adjustment.set_value(rect.y1 - page);
        }
    }

    /// The message the rail marks as current: the one with the most of it
    /// on screen (001 FR-035).
    pub fn current_message(&self) -> Option<String> {
        let document = self.document()?;
        let adjustment = self.imp().vadjustment.borrow().clone()?;
        let extents: Vec<postio_ui::reader::rail::Extent> = document
            .messages
            .iter()
            .map(|m| postio_ui::reader::rail::Extent {
                top: m.rect.y0,
                height: m.rect.height(),
            })
            .collect();
        let at =
            postio_ui::reader::rail::current(&extents, adjustment.value(), adjustment.page_size())?;
        Some(document.messages[at].scope.clone())
    }

    /// Tell `current-message` listeners when the message with most of the
    /// view changes (the rail, 001 FR-035).
    fn report_current_message(&self) {
        let now = self.current_message();
        if now.is_some() && *self.imp().current.borrow() != now {
            self.imp().current.replace(now.clone());
            self.emit_by_name::<()>("current-message", &[&now.unwrap_or_default()]);
        }
    }

    /// Call `f` with the scope of the message the rail should mark, as the
    /// view scrolls.
    pub fn connect_current_message(
        &self,
        f: impl Fn(&Self, &str) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_local("current-message", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            let scope = values[1].get::<String>().expect("a scope");
            f(&view, &scope);
            None
        })
    }

    /// Call `f` each time a snapshot reaches the screen.
    pub fn connect_rendered(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("rendered", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            f(&view);
            None
        })
    }

    /// Scroll `scope`'s message to the top of the view.
    pub fn scroll_to_message(&self, scope: &str) {
        let (Some(document), Some(adjustment)) =
            (self.document(), self.imp().vadjustment.borrow().clone())
        else {
            return;
        };
        if let Some(message) = document.messages.iter().find(|m| m.scope == scope) {
            adjustment.set_value(message.rect.y0);
        }
    }

    /// Scroll one page down (`forward`) or up: the view's own height.
    pub fn page(&self, forward: bool) {
        let Some(adjustment) = self.imp().vadjustment.borrow().clone() else {
            return;
        };
        let step = if forward {
            adjustment.page_size()
        } else {
            -adjustment.page_size()
        };
        adjustment.set_value(adjustment.value() + step);
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
                zoom: f64::from(imp.zoom.get()) / 100.0,
            },
            theme: Theme {
                dark: style.is_dark(),
                high_contrast: style.is_high_contrast(),
            },
            darkened: imp.darkened.borrow().clone(),
            toggled_folds: imp.toggled_folds.borrow().clone(),
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
        self.keep_place();
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
        let old_len = imp
            .document
            .borrow()
            .as_ref()
            .map_or(0, |doc| doc.text.text.chars().count());
        let document = Arc::new(document);
        imp.tiles.borrow_mut().reset(document.clone());
        let anchor = imp.anchor.take().and_then(|(offset, at)| {
            document
                .text
                .clusters
                .iter()
                .filter(|c| c.range.end > offset)
                .min_by_key(|c| c.range.start)
                .map(|c| (c.rect.y0 - at).max(0.0))
        });
        imp.document.replace(Some(document));
        self.configure_adjustments();
        if imp.to_top.take() {
            if let Some(adjustment) = imp.vadjustment.borrow().as_ref() {
                adjustment.set_value(0.0);
            }
        } else if let (Some(y), Some(adjustment)) = (anchor, imp.vadjustment.borrow().as_ref()) {
            adjustment.set_value(y);
        }
        // The text is the same, so a find carries over to the new snapshot.
        self.refresh_find();
        self.announce_contents(old_len);
        self.emit_by_name::<()>("rendered", &[]);
        self.report_current_message();
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
        // A pinch scales what is already drawn; the render comes at its end.
        let pinch = imp.pinch.get();
        if let Some(scale) = pinch {
            snapshot.save();
            snapshot.scale(scale as f32, scale as f32);
        }
        let view = self.downgrade();
        imp.tiles
            .borrow_mut()
            .draw(snapshot, &document, left, top, height, move || {
                if let Some(view) = view.upgrade() {
                    view.queue_draw();
                }
            });
        self.draw_find(snapshot, left, top);
        self.draw_selection(snapshot, left, top);
        if pinch.is_some() {
            snapshot.restore();
        }
        snapshot.pop();
    }
}
