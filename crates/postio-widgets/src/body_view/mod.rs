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
        /// The range highlighted in the text on screen, if one is.
        pub(super) highlight: RefCell<Option<std::ops::Range<usize>>>,
        /// A highlight still to be brought into view, waiting for the
        /// column to grow to the document it is in: the adjustment watched,
        /// and the handler to let go of.
        pub(super) reveal: RefCell<Option<(gtk::Adjustment, glib::SignalHandlerId)>>,
        /// Where a drag began, in the view's coordinates.
        pub(super) drag_start: Cell<Option<gtk::graphene::Point>>,
        /// The scroller this view is laid out inside, when it takes its
        /// place from that instead of scrolling itself: see
        /// [`super::BodyView::flow_in`].
        pub(super) flow: RefCell<Option<glib::WeakRef<gtk::ScrolledWindow>>>,
        /// What the column's ground is read from, when the view flows: a
        /// widget whose CSS `color` is the column's ground token
        /// ([`BodyView::set_ground`]).
        pub(super) ground: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        /// The rest of the column's palette, read the same way: each a
        /// reader variable (`--r-ink`) and the widget whose `color` is the
        /// token it stands for ([`BodyView::add_palette_probe`]).
        pub(super) palette: RefCell<Vec<(&'static str, glib::WeakRef<gtk::Widget>)>>,
        /// The ground the last render was asked for.
        pub(super) ground_drawn: RefCell<Option<String>>,
        /// The messages the user darkened (FR-013a): for this session only,
        /// never stored.
        pub(super) darkened: RefCell<Vec<String>>,
        /// The style manager's handlers, disconnected on dispose.
        pub(super) theme_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        /// Renders held on the render thread, and the gates holding them:
        /// see [`super::BodyView::hold_renders`].
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
                current: RefCell::default(),
                to_top: Cell::new(false),
                darkened: RefCell::default(),
                flow: RefCell::default(),
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
        /// A view that flows inside a scroller is as tall as its document,
        /// so the scroller's column is what scrolls; on its own it asks for
        /// nothing and takes what it is given.
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
    ///
    /// Nothing the user did to the last message is drawn over this one: it
    /// is shown as it was sent. A darkening applies to the one message it
    /// was asked of (spec 006 FR-013a), and a single message's scope is the
    /// empty string, so a darkened scope kept here would darken every
    /// message after it. A selection, a focused link and a fold are places
    /// in one message's text. Focus's dialog shows every message in one
    /// view, which is where this was found (specs/007-postio-focus research
    /// R1).
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
        let top = self.window().0;
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

    /// Hold every render this view asks for from now on, on the render
    /// thread, until [`release_renders`](Self::release_renders): how a test
    /// makes a render outlast its deadline (spec 006 FR-023) through a
    /// caller that builds the view's content itself, such as a reader.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn hold_renders(&self) {
        self.imp().held.replace(Some(Vec::new()));
    }

    /// Let every held render go on, and hold no more.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn release_renders(&self) {
        for gate in self.imp().held.take().unwrap_or_default() {
            gate.release();
        }
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
        self.imp().tiles.borrow().settled(self.window().0)
    }

    /// Call `f` when the fallback notice's "View source" is chosen.
    pub fn connect_view_source(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("view-source", false, move |values| {
            let view = values[0].get::<BodyView>().expect("the signal's own view");
            f(&view);
            None
        })
    }

    /// Every fold in the message on screen: its id, what its line says,
    /// and whether it is open -- for a surface outside the body that names
    /// them (Focus's fold line, screen 04).
    pub fn folds(&self) -> Vec<(String, String, bool)> {
        let Some(document) = self.document() else {
            return Vec::new();
        };
        document
            .folds
            .iter()
            .map(|fold| {
                // The line's words are the text drawn inside its summary.
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
                    // Less the disclosure triangle the summary draws.
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

    /// Open the fold `id` if it is closed; an open one stays open.
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
                    self.scroll_document_to(top);
                }
            }
        }
    }

    /// Scroll so `rect`, in document coordinates, is in view.
    pub(super) fn scroll_into_view(&self, rect: postio_render::Rect) {
        let (top, page) = self.window();
        if rect.y0 < top {
            self.scroll_document_to(rect.y0);
        } else if rect.y1 > top + page {
            self.scroll_document_to(rect.y1 - page);
        }
    }

    /// The message the rail marks as current: the one with the most of it
    /// on screen (001 FR-035).
    pub fn current_message(&self) -> Option<String> {
        let document = self.document()?;
        let (top, page) = self.window();
        let extents: Vec<postio_ui::reader::rail::Extent> = document
            .messages
            .iter()
            .map(|m| postio_ui::reader::rail::Extent {
                top: m.rect.y0,
                height: m.rect.height(),
            })
            .collect();
        let at = postio_ui::reader::rail::current(&extents, top, page)?;
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
        let Some(document) = self.document() else {
            return;
        };
        if let Some(message) = document.messages.iter().find(|m| m.scope == scope) {
            self.scroll_document_to(message.rect.y0);
        }
    }

    /// Scroll one page down (`forward`) or up: the view's own height, or a
    /// flowing view's scroller's.
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

    /// Scroll by `lines` steps of the view's own line increment: down for
    /// a positive count, up for a negative one. The adjustment clamps.
    pub fn scroll_lines(&self, lines: i32) {
        let Some(adjustment) = self.scrolling() else {
            return;
        };
        adjustment.set_value(adjustment.value() + f64::from(lines) * adjustment.step_increment());
    }

    /// Scroll to the top of the document, or to the end of it; for a view
    /// that flows, the top or end of the whole column.
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

    /// Lay this view out inside `scroller`, one column with whatever sits
    /// beside it, instead of scrolling itself.
    ///
    /// The view is then as tall as its document and never scrolls; what is
    /// in view is read from `scroller`'s adjustment each time it draws, so
    /// only the tiles under the scroller's window are rasterised and a long
    /// message costs what a screenful does (research R8 still holds). Keys
    /// and finds scroll the scroller.
    pub fn flow_in(&self, scroller: &gtk::ScrolledWindow) {
        let imp = self.imp();
        imp.flow.replace(Some(scroller.downgrade()));
        // Its own adjustments stay at rest, so a point in the view is a
        // point in the document.
        self.set_vadjustment(Some(&gtk::Adjustment::new(0.0, 0.0, 0.0, 1.0, 1.0, 0.0)));
        self.set_hadjustment(Some(&gtk::Adjustment::new(0.0, 0.0, 0.0, 1.0, 1.0, 0.0)));
        let view = self.downgrade();
        scroller.vadjustment().connect_value_changed(move |_| {
            if let Some(view) = view.upgrade() {
                view.queue_draw();
                view.report_current_message();
            }
        });
        self.scroll_sideways();
        self.queue_resize();
    }

    /// Take a sideways scroll -- a touchpad's, or Shift with the wheel --
    /// for a page wider than the column: one on paper that fitting would
    /// have zoomed under `postio_render::render::PAPER_FIT_FLOOR`, which is
    /// drawn at the floor and read across instead (T207). The column only
    /// scrolls down, so the view moves its own sideways adjustment; a scroll
    /// with nothing sideways in it is the column's.
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
                // A diagonal touchpad scroll still moves the column down.
                if dy != 0.0 && !shifted {
                    glib::Propagation::Proceed
                } else {
                    glib::Propagation::Stop
                }
            }
        });
        self.add_controller(scroll);
    }

    /// Read the document's ground from `probe`: a widget styled so its CSS
    /// `color` is the ground token of the column the view flows in
    /// (`.postio-flow-ground`). The renderer has no GTK style context, so
    /// the colour is resolved here, at each render -- a switch to dark
    /// renders again and reads the dark value -- and handed to the document
    /// as `--flow-ground`, never written into it as a literal (T203).
    pub fn set_ground(&self, probe: &impl IsA<gtk::Widget>) {
        self.imp().ground.replace(Some(probe.as_ref().downgrade()));
    }

    /// Read the document's `variable` (a reader palette variable such as
    /// `--r-ink`) from `probe`, as [`set_ground`](Self::set_ground) reads
    /// the ground: a widget whose CSS `color` is the column's token for it.
    ///
    /// What lets a body drawn in the app's colours (specs/007-postio-focus
    /// T211) be drawn in the column's own ink, accent and hairlines rather
    /// than the generated reader palette's -- the same tokens the chrome
    /// around it is drawn in, in light and dark, with no colour written in
    /// Rust.
    pub fn add_palette_probe(&self, variable: &'static str, probe: &impl IsA<gtk::Widget>) {
        self.imp()
            .palette
            .borrow_mut()
            .push((variable, probe.as_ref().downgrade()));
    }

    /// The ground and palette the document is drawn with, as CSS, when the
    /// view reads any.
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

    /// Whether the view flows inside a scroller.
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

    /// What scrolls the document: the view's own adjustment, or the column's.
    fn scrolling(&self) -> Option<gtk::Adjustment> {
        match self.flow_scroller() {
            Some(scroller) => Some(scroller.vadjustment()),
            None => self.imp().vadjustment.borrow().clone(),
        }
    }

    /// The part of the document in view: its top, and its height, in
    /// document coordinates.
    pub(super) fn window(&self) -> (f64, f64) {
        if self.flows() {
            let Some(scroller) = self.flow_scroller() else {
                return (0.0, 0.0);
            };
            let Some(bounds) = self.compute_bounds(&scroller) else {
                return (0.0, 0.0);
            };
            // `bounds` is where the view is in the scroller's window, which
            // already has the scrolling in it.
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

    /// Scroll so document height `y` is at the top of what is in view.
    pub(super) fn scroll_document_to(&self, y: f64) {
        self.scroll_document_reaching(y);
    }

    /// [`scroll_document_to`](Self::scroll_document_to), saying whether the
    /// scroll got there: false when the adjustment stopped short, which a
    /// column does when it has not yet grown to a document just drawn.
    pub(super) fn scroll_document_reaching(&self, y: f64) -> bool {
        let (adjustment, want) = match self.flow_scroller() {
            Some(scroller) => {
                let Some(bounds) = self.compute_bounds(&scroller) else {
                    return false;
                };
                let adjustment = scroller.vadjustment();
                // Where the view starts in the column, whatever the column
                // is scrolled to now.
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

    /// How far the message is scrolled: the column's for a flowing view.
    pub fn scrolled(&self) -> f64 {
        self.scrolling().map_or(0.0, |a| a.value())
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
        let ground = self.ground_css();
        imp.ground_drawn.replace(ground.clone());
        let document = match ground {
            Some(ground) if content.document.contains("</style>") => {
                content
                    .document
                    .replacen("</style>", &format!("{ground}</style>"), 1)
            }
            _ => content.document,
        };
        let request = RenderRequest {
            generation,
            document,
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
        #[cfg(feature = "test-hooks")]
        if let Some(held) = imp.held.borrow_mut().as_mut() {
            held.push(request.resources.hold_lookup());
        }
        let result = std::rc::Rc::new(self.renderer().request(request));
        imp.pending.set(Some(generation));
        // The deadline (FR-023): if the render is still out when it passes,
        // give up on it and show the plain text instead.
        let view = self.downgrade();
        let answered = std::rc::Rc::clone(&result);
        glib::timeout_add_local_once(imp.deadline.get(), move || {
            let Some(view) = view.upgrade() else { return };
            let imp = view.imp();
            if imp.pending.get() != Some(generation) {
                return;
            }
            // Out by the deadline, or only not yet collected? The poll
            // below is a main-loop timer too, and a main thread busy past
            // the deadline finds both due at once; a render that finished
            // in time is the one shown (T218).
            if let Ok(document) = answered.try_recv() {
                view.show(document);
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
        // The style manager says the scheme changed before the stylesheet
        // that paints it is in place, so the column's ground read just now
        // can be the old scheme's: read it again once the main loop has
        // turned, and draw again if it moved.
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
                // The top of the column, not of the body inside it.
                Some(scroller) => scroller.vadjustment().set_value(0.0),
                None => self.scroll_document_to(0.0),
            }
        } else if let Some(y) = anchor {
            self.scroll_document_to(y);
        }
        if self.flows() {
            self.queue_resize();
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
        // A flowing view's height is its document's and the column scrolls
        // it; across, a page wider than the column is still the view's own
        // to scroll (`scroll_sideways`).
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
            // Only the window in the scroller is drawn, placed where it is.
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
        // A pinch scales what is already drawn; the render comes at its end.
        let pinch = imp.pinch.get();
        if let Some(scale) = pinch {
            snapshot.save();
            snapshot.scale(scale as f32, scale as f32);
        }
        // Where the space tiles are placed in sits in the surface, so their
        // edges can be put on its device pixels.
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

/// A toolkit colour as CSS: `rgb()`, or `rgba()` when it is see-through --
/// a hairline is a translucent ink.
fn css_colour(colour: &gtk::gdk::RGBA) -> String {
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
