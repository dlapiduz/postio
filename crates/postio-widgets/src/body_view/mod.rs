//! The reading surface: a `gtk::Scrollable` that paints a `postio-render` snapshot as tiles. It
//! never touches the engine; a render thread lays the message out.

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

type Launcher = Box<dyn Fn(&str)>;

#[derive(Clone)]
pub struct Content {
    pub document: String,
    pub resources: Arc<Resources>,
    pub plain_text: String,
    pub fallback: Option<postio_render::Fallback>,
    pub over_cap: Option<postio_body::Cap>,
}

/// Built once; `prewarm_fonts` starts it off the UI thread.
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
        pub(super) generation: Cell<u64>,
        pub(super) pending: Cell<Option<u64>>,
        pub(super) laid_out_width: Cell<i32>,
        pub(super) document: RefCell<Option<Arc<RenderedDocument>>>,
        pub(super) tiles: RefCell<tiles::Tiles>,
        /// The character to bring back to the top, and its height in the view, when the next
        /// snapshot arrives.
        pub(super) anchor: Cell<Option<(usize, f64)>>,
        /// The zoom, in percent: one of `postio_config::ZOOM_STEPS`.
        pub(super) zoom: Cell<u16>,
        pub(super) to_top: Cell<bool>,
        pub(super) pinch: Cell<Option<f64>>,
        pub(super) pointer: Cell<Option<gtk::graphene::Point>>,
        pub(super) selection: RefCell<Option<std::ops::Range<usize>>>,
        pub(super) toggled_folds: RefCell<Vec<String>>,
        pub(super) focused_link: Cell<Option<usize>>,
        pub(super) launcher: RefCell<Option<Launcher>>,
        pub(super) find: RefCell<super::find::FindState>,
        pub(super) highlight: RefCell<Option<std::ops::Range<usize>>>,
        pub(super) reveal: RefCell<Option<(gtk::Adjustment, glib::SignalHandlerId)>>,
        pub(super) drag_start: Cell<Option<gtk::graphene::Point>>,
        pub(super) flow: RefCell<Option<glib::WeakRef<gtk::ScrolledWindow>>>,
        pub(super) keep: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        pub(super) ground: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        pub(super) palette: RefCell<Vec<(&'static str, glib::WeakRef<gtk::Widget>)>>,
        pub(super) ground_drawn: RefCell<Option<String>>,
        /// Messages the user darkened; this session only, never stored.
        pub(super) darkened: RefCell<Vec<String>>,
        pub(super) theme_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        #[cfg(feature = "test-hooks")]
        pub(super) held: RefCell<Option<Vec<Arc<postio_render::resources::Gate>>>>,
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
                to_top: Cell::new(false),
                darkened: RefCell::default(),
                flow: RefCell::default(),
                keep: RefCell::default(),
                ground: RefCell::default(),
                palette: RefCell::default(),
                ground_drawn: RefCell::default(),
                selection: RefCell::default(),
                drag_start: Cell::new(None),
                find: RefCell::default(),
                highlight: RefCell::default(),
                reveal: RefCell::default(),
                toggled_folds: RefCell::default(),
                focused_link: Cell::new(None),
                launcher: RefCell::default(),
                theme_handlers: RefCell::default(),
                #[cfg(feature = "test-hooks")]
                held: RefCell::default(),
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
            klass.set_accessible_role(gtk::AccessibleRole::Document);
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
                    glib::subclass::Signal::builder("zoom-changed").build(),
                    glib::subclass::Signal::builder("rendered").build(),
                ]
            })
        }
    }

    impl WidgetImpl for BodyView {
        /// A flowing view is as tall as its document so the scroller scrolls; alone it asks for
        /// nothing.
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Vertical && self.flow.borrow().is_some() {
                let height = self
                    .document
                    .borrow()
                    .as_ref()
                    .map_or(0, |doc| doc.size.height.ceil() as i32);
                return (height, height, -1, -1);
            }
            (0, 0, -1, -1)
        }

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
    pub struct BodyView(ObjectSubclass<imp::BodyView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl BodyView {
    pub fn new(deadline: Duration) -> BodyView {
        let view: BodyView = glib::Object::new();
        view.imp().deadline.set(deadline);
        view
    }

    /// Shows `content` from its top, with nothing of the last message drawn over it: a darkening is
    /// scoped to one message and a single message's scope is the empty string, so a kept one would
    /// darken every later one.
    pub fn set_content_from_top(&self, content: Content) {
        let imp = self.imp();
        imp.to_top.set(true);
        imp.darkened.borrow_mut().clear();
        imp.toggled_folds.borrow_mut().clear();
        imp.focused_link.set(None);
        imp.highlight.replace(None);
        self.forget_reveal();
        if let Some(sideways) = imp.hadjustment.borrow().as_ref() {
            sideways.set_value(0.0);
        }
        self.set_selection(None);
        self.set_content(content);
    }

    /// Shows `content`, keeping the character at the top of the view in place so a late body or
    /// image above it does not move the words being read.
    pub fn set_content(&self, content: Content) {
        if !self.imp().to_top.get() {
            self.keep_place();
        }
        self.imp().content.replace(Some(content));
        self.request_render();
    }

    fn keep_place(&self) {
        let imp = self.imp();
        let Some(document) = imp.document.borrow().clone() else {
            return;
        };
        let top = self.window().0;
        // At the very top there is no place to keep; anchoring the first line would carry the
        // column down.
        if top <= 0.0 {
            imp.anchor.set(None);
            return;
        }
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

    /// Holds every render on the render thread until `release_renders`: how a test makes a render
    /// outlast its deadline.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn hold_renders(&self) {
        self.imp().held.replace(Some(Vec::new()));
    }

    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn release_renders(&self) {
        for gate in self.imp().held.take().unwrap_or_default() {
            gate.release();
        }
    }

    #[doc(hidden)]
    pub fn tile_bytes(&self) -> usize {
        self.imp().tiles.borrow().bytes()
    }

    #[doc(hidden)]
    pub fn tiles_settled(&self) -> bool {
        self.imp().tiles.borrow().settled(self.window().0)
    }

    pub fn connect_view_source(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("view-source", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            f(&view);
            None
        })
    }

    pub fn folds(&self) -> Vec<(String, String, bool)> {
        let Some(document) = self.document() else {
            return Vec::new();
        };
        document
            .folds
            .iter()
            .map(|fold| {
                let inside: Vec<&postio_render::Cluster> = document
                    .text
                    .clusters
                    .iter()
                    .filter(|cluster| {
                        let middle = postio_render::Point::new(
                            (cluster.rect.x0 + cluster.rect.x1) / 2.0,
                            (cluster.rect.y0 + cluster.rect.y1) / 2.0,
                        );
                        fold.summary_rect.contains(middle)
                    })
                    .collect();
                let label = match (
                    inside.iter().map(|c| c.range.start).min(),
                    inside.iter().map(|c| c.range.end).max(),
                ) {
                    (Some(start), Some(end)) => document
                        .text
                        .slice(start..end)
                        .trim_start_matches(['\u{25b8}', '\u{25be}', '\u{25b6}', '\u{25bc}'])
                        .trim()
                        .to_owned(),
                    _ => String::new(),
                };
                (fold.id.clone(), label, fold.open)
            })
            .collect()
    }

    pub fn open_fold(&self, id: &str) {
        let closed = self.document().is_some_and(|document| {
            document
                .folds
                .iter()
                .any(|fold| fold.id == id && !fold.open)
        });
        if closed {
            self.toggle_fold(id);
        }
    }

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

    pub fn darken_title(&self) -> Option<&'static str> {
        let (_, presentation) = self.darkenable()?;
        Some(match presentation {
            postio_render::Presentation::Darkened => "Show as sent",
            _ => "Darken this message",
        })
    }

    fn darkenable(&self) -> Option<(String, postio_render::Presentation)> {
        use postio_render::Presentation::{Darkened, Paper};
        let imp = self.imp();
        let document = imp.document.borrow().clone()?;
        let (top, page) = self.window();
        let line = top + page / 3.0;
        let message = document
            .messages
            .iter()
            .find(|m| m.rect.y0 <= line && line < m.rect.y1)
            .or(document.messages.first())?;
        matches!(message.presentation, Paper | Darkened)
            .then(|| (message.scope.clone(), message.presentation))
    }

    #[doc(hidden)]
    pub fn drag_select(&self, from: gtk::graphene::Point, to: gtk::graphene::Point) {
        self.select_points(from, to, true);
    }

    #[doc(hidden)]
    pub fn click_select(&self, at: gtk::graphene::Point, presses: i32) {
        let Some(document) = self.document() else {
            return;
        };
        if presses == 1 {
            let point = self.document_point(at);
            // A link first: verbs sit inside a message's header, which is its fold's summary.
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

    pub fn selection(&self) -> Option<std::ops::Range<usize>> {
        self.imp().selection.borrow().clone()
    }

    #[doc(hidden)]
    pub fn selection_rects(&self) -> Vec<postio_render::Rect> {
        match (self.selection(), self.document()) {
            (Some(range), Some(document)) => document.text.rects(range),
            _ => Vec::new(),
        }
    }

    #[doc(hidden)]
    pub fn set_launcher(&self, launch: impl Fn(&str) + 'static) {
        self.imp().launcher.replace(Some(Box::new(launch)));
    }

    #[doc(hidden)]
    pub fn hover(&self, at: gtk::graphene::Point) {
        let point = self.document_point(at);
        let target = self
            .document()
            .and_then(|doc| doc.link_at(point).map(|link| link.target.describe()));
        self.set_tooltip_text(target.as_deref());
        self.set_cursor_from_name(Some(if target.is_some() { "pointer" } else { "text" }));
    }

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

    #[doc(hidden)]
    pub fn focused_link_target(&self) -> Option<String> {
        let document = self.document()?;
        let link = document.links.get(self.imp().focused_link.get()?)?;
        Some(link.target.describe())
    }

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
            LinkTarget::Verb { .. } => {}
            LinkTarget::Fragment { scope, id } => {
                let Some(document) = self.document() else {
                    return;
                };
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
                    self.scroll_document_to(top);
                }
            }
        }
    }

    pub(super) fn scroll_into_view(&self, rect: postio_render::Rect) {
        let (top, page) = self.window();
        if rect.y0 < top {
            self.scroll_document_to(rect.y0);
        } else if rect.y1 > top + page {
            self.scroll_document_to(rect.y1 - page);
        }
    }

    pub fn connect_rendered(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("rendered", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            f(&view);
            None
        })
    }

    pub fn page(&self, forward: bool) {
        let Some(adjustment) = self.scrolling() else {
            return;
        };
        let step = if forward {
            adjustment.page_size()
        } else {
            -adjustment.page_size()
        };
        adjustment.set_value(adjustment.value() + step);
    }

    /// Negative scrolls up; the adjustment clamps.
    pub fn scroll_lines(&self, lines: i32) {
        let Some(adjustment) = self.scrolling() else {
            return;
        };
        adjustment.set_value(adjustment.value() + f64::from(lines) * adjustment.step_increment());
    }

    pub fn scroll_to_edge(&self, bottom: bool) {
        let Some(adjustment) = self.scrolling() else {
            return;
        };
        adjustment.set_value(if bottom {
            adjustment.upper() - adjustment.page_size()
        } else {
            adjustment.lower()
        });
    }

    /// Lays the view out inside `scroller`: as tall as its document and never scrolling itself, so
    /// only the tiles under the scroller's window are rasterised. Keys and finds scroll the
    /// scroller.
    pub fn flow_in(&self, scroller: &gtk::ScrolledWindow) {
        let imp = self.imp();
        imp.flow.replace(Some(scroller.downgrade()));
        self.set_vadjustment(Some(&gtk::Adjustment::new(0.0, 0.0, 0.0, 1.0, 1.0, 0.0)));
        self.set_hadjustment(Some(&gtk::Adjustment::new(0.0, 0.0, 0.0, 1.0, 1.0, 0.0)));
        let view = self.downgrade();
        scroller.vadjustment().connect_value_changed(move |_| {
            if let Some(view) = view.upgrade() {
                view.queue_draw();
            }
        });
        self.scroll_sideways();
        self.queue_resize();
    }

    /// Takes a sideways scroll for a paper page wider than the column (drawn at `PAPER_FIT_FLOOR`);
    /// the column only scrolls down, so the view moves its own sideways adjustment.
    fn scroll_sideways(&self) {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.connect_scroll({
            let view = self.downgrade();
            move |controller, dx, dy| {
                let Some(view) = view.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let state = controller.current_event_state();
                if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                    return glib::Propagation::Proceed;
                }
                let shifted = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                let across = if dx != 0.0 {
                    dx
                } else if shifted {
                    dy
                } else {
                    return glib::Propagation::Proceed;
                };
                let Some(adjustment) = view.imp().hadjustment.borrow().clone() else {
                    return glib::Propagation::Proceed;
                };
                if adjustment.upper() - adjustment.page_size() <= 0.5 {
                    return glib::Propagation::Proceed;
                }
                let step = match controller.unit() {
                    gtk::gdk::ScrollUnit::Wheel => adjustment.step_increment(),
                    _ => 1.0,
                };
                let most = adjustment.upper() - adjustment.page_size();
                adjustment.set_value((adjustment.value() + across * step).clamp(0.0, most));
                view.queue_draw();
                if dy != 0.0 && !shifted {
                    glib::Propagation::Proceed
                } else {
                    glib::Propagation::Stop
                }
            }
        });
        self.add_controller(scroll);
    }

    /// Reads the document's ground from `probe`, a widget whose CSS `color` is the column's ground
    /// token. The renderer has no style context, so it is resolved at each render and passed as
    /// `--flow-ground`.
    pub fn set_ground(&self, probe: &impl IsA<gtk::Widget>) {
        self.imp().ground.replace(Some(probe.as_ref().downgrade()));
    }

    /// Reads reader palette `variable` (such as `--r-ink`) from `probe`, as `set_ground` does, so
    /// the body is drawn in the column's own colours.
    pub fn add_palette_probe(&self, variable: &'static str, probe: &impl IsA<gtk::Widget>) {
        self.imp()
            .palette
            .borrow_mut()
            .push((variable, probe.as_ref().downgrade()));
    }

    fn ground_css(&self) -> Option<String> {
        let imp = self.imp();
        let mut declarations = Vec::new();
        if let Some(probe) = imp.ground.borrow().as_ref().and_then(|weak| weak.upgrade()) {
            declarations.push(format!("--flow-ground:{}", css_colour(&probe.color())));
        }
        for (variable, probe) in imp.palette.borrow().iter() {
            if let Some(probe) = probe.upgrade() {
                declarations.push(format!("{variable}:{}", css_colour(&probe.color())));
            }
        }
        (!declarations.is_empty()).then(|| format!(":root{{{};}}", declarations.join(";")))
    }

    pub fn flows(&self) -> bool {
        self.imp().flow.borrow().is_some()
    }

    fn flow_scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.imp()
            .flow
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }

    fn scrolling(&self) -> Option<gtk::Adjustment> {
        match self.flow_scroller() {
            Some(scroller) => Some(scroller.vadjustment()),
            None => self.imp().vadjustment.borrow().clone(),
        }
    }

    pub(super) fn window(&self) -> (f64, f64) {
        if self.flows() {
            let Some(scroller) = self.flow_scroller() else {
                return (0.0, 0.0);
            };
            let Some(bounds) = self.compute_bounds(&scroller) else {
                return (0.0, 0.0);
            };
            let above = f64::from(bounds.y()).min(0.0).abs();
            let below = (f64::from(scroller.height()) - f64::from(bounds.y()))
                .min(f64::from(self.height()));
            return (above, (below - above).max(0.0));
        }
        let top = self
            .imp()
            .vadjustment
            .borrow()
            .as_ref()
            .map_or(0.0, |a| a.value());
        (top, f64::from(self.height()))
    }

    pub(super) fn scroll_document_to(&self, y: f64) {
        self.scroll_document_reaching(y);
    }

    /// False when the adjustment stopped short, as a column does before it has grown to a document
    /// just drawn.
    pub(super) fn scroll_document_reaching(&self, y: f64) -> bool {
        let (adjustment, want) = match self.flow_scroller() {
            Some(scroller) => {
                let Some(bounds) = self.compute_bounds(&scroller) else {
                    return false;
                };
                let adjustment = scroller.vadjustment();
                let start = adjustment.value() + f64::from(bounds.y());
                (adjustment, start + y)
            }
            None => match self.imp().vadjustment.borrow().clone() {
                Some(adjustment) => (adjustment, y),
                None => return false,
            },
        };
        adjustment.set_value(want);
        (adjustment.value() - want).abs() < 0.5
    }

    pub fn scrolled(&self) -> f64 {
        self.scrolling().map_or(0.0, |a| a.value())
    }

    /// `None` until something scrolls it.
    pub fn scroll_extent(&self) -> Option<(f64, f64)> {
        self.scrolling()
            .map(|a| (a.value(), (a.upper() - a.page_size()).max(0.0)))
    }

    pub fn document(&self) -> Option<Arc<RenderedDocument>> {
        self.imp().document.borrow().clone()
    }

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

    /// Fractional (1.0, 1.25, 2.0); painting at 1.0 and letting the compositor stretch blurs on
    /// HiDPI.
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
        let ground = self.ground_css();
        imp.ground_drawn.replace(ground.clone());
        let grounded = |document: String| match &ground {
            Some(ground) if document.contains("</style>") => {
                document.replacen("</style>", &format!("{ground}</style>"), 1)
            }
            _ => document,
        };
        let document = grounded(content.document);
        let composed = content.fallback.map(|fallback| postio_render::Fallback {
            document: grounded(fallback.document),
            ..fallback
        });
        let request = RenderRequest {
            generation,
            document,
            plain_text: content.plain_text,
            fallback: composed,
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
        let fallback = RenderRequest {
            generation,
            document: String::new(),
            plain_text: request.plain_text.clone(),
            fallback: request.fallback.clone(),
            over_cap: None,
            resources: request.resources.clone(),
            viewport: request.viewport,
            theme: request.theme,
            darkened: Vec::new(),
            toggled_folds: Vec::new(),
            reader_view: Vec::new(),
        };
        #[cfg(feature = "test-hooks")]
        if let Some(held) = imp.held.borrow_mut().as_mut() {
            held.push(request.resources.hold_lookup());
        }
        let result = std::rc::Rc::new(self.renderer().request(request));
        imp.pending.set(Some(generation));
        let view = self.downgrade();
        let answered = std::rc::Rc::clone(&result);
        glib::timeout_add_local_once(imp.deadline.get(), move || {
            let Some(view) = view.upgrade() else { return };
            let imp = view.imp();
            if imp.pending.get() != Some(generation) {
                return;
            }
            // The poll below is also a main-loop timer: a render that finished in time wins when
            // both are due at once.
            if let Ok(document) = answered.try_recv() {
                view.show(document);
                return;
            }
            view.renderer().abandon(generation);
            let next = generation + 1;
            imp.generation.set(next);
            imp.pending.set(None);
            let document = view.renderer().fallback(
                &RenderRequest {
                    generation: next,
                    ..fallback.clone()
                },
                postio_render::FallbackReason::Deadline,
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

    fn theme_changed(&self) {
        self.keep_place();
        self.request_render();
        // The style manager signals before the stylesheet is in place, so the ground read now can
        // be the old scheme's: read it again once the main loop has turned.
        if self.imp().ground.borrow().is_some() || !self.imp().palette.borrow().is_empty() {
            let view = self.downgrade();
            glib::idle_add_local_once(move || {
                let Some(view) = view.upgrade() else { return };
                if view.ground_css() != *view.imp().ground_drawn.borrow() {
                    view.keep_place();
                    view.request_render();
                }
            });
        }
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
            match self.flow_scroller() {
                Some(scroller) => scroller.vadjustment().set_value(0.0),
                None => self.scroll_document_to(0.0),
            }
        } else if let Some(y) = anchor {
            self.scroll_document_to(y);
        }
        if self.flows() {
            self.queue_resize();
        }
        self.refresh_find();
        self.announce_contents(old_len);
        self.emit_by_name::<()>("rendered", &[]);
        self.queue_draw();
    }

    fn configure_adjustments(&self) {
        let imp = self.imp();
        if !self.flows() {
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
        let (top, height) = self.window();
        let left = imp.hadjustment.borrow().as_ref().map_or(0.0, |a| a.value());
        let width = f64::from(self.width());
        if self.flows() {
            snapshot.push_clip(&gtk::graphene::Rect::new(
                0.0,
                top as f32,
                width as f32,
                height as f32,
            ));
            snapshot.translate(&gtk::graphene::Point::new(0.0, top as f32));
        } else {
            snapshot.push_clip(&gtk::graphene::Rect::new(
                0.0,
                0.0,
                width as f32,
                height as f32,
            ));
        }
        let pinch = imp.pinch.get();
        if let Some(scale) = pinch {
            snapshot.save();
            snapshot.scale(scale as f32, scale as f32);
        }
        let scale = self.surface_scale();
        let origin = self
            .native()
            .and_then(|native| self.compute_point(&native, &gtk::graphene::Point::zero()))
            .map_or(0.0, |point| f64::from(point.y()))
            + if self.flows() { top } else { 0.0 };
        let view = self.downgrade();
        imp.tiles.borrow_mut().draw(
            snapshot,
            &document,
            tiles::Frame {
                left,
                top,
                height,
                origin,
                scale,
            },
            move || {
                if let Some(view) = view.upgrade() {
                    view.queue_draw();
                }
            },
        );
        self.draw_highlight(snapshot, left, top);
        self.draw_find(snapshot, left, top);
        self.draw_selection(snapshot, left, top);
        if pinch.is_some() {
            snapshot.restore();
        }
        snapshot.pop();
    }
}

pub(crate) fn css_colour(colour: &gtk::gdk::RGBA) -> String {
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (
        channel(colour.red()),
        channel(colour.green()),
        channel(colour.blue()),
    );
    if colour.alpha() >= 0.999 {
        format!("rgb({r},{g},{b})")
    } else {
        format!("rgba({r},{g},{b},{:.3})", colour.alpha().clamp(0.0, 1.0))
    }
}
