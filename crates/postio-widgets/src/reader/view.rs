//! The reading pane, drawn by the reading renderer. A body is hostile input: no script engine
//! exists to run it, nothing is
//! fetched unless the user allowed it, `cid:` images resolve through a [`BlobSource`] and never the
//! network, and a click
//! never navigates the pane.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_model::message::MessageBody;

use super::BlobSource;
use super::banner::{DecodeNotice, RemoteImageBanner, UnsubscribeBanner};
use super::message_header::MessageHeader;
use super::notices::{Notice, NoticeSlot};
use postio_body::sanitize::RemoteImages;
pub use postio_body::treatment::Treatment;
use postio_ui::allowlist::RemoteImageAllowList;
pub use postio_ui::reader::document::{
    Absent, HeldBack, Rendering, Sheet, Treated, absent_html, body_html, content_security_policy,
    document_for, reader_ground, sheet_for, wrap_document,
};

use super::render_mode::RenderModeLine;

struct Open {
    body: MessageBody,
    sender: Option<String>,
    /// Per message and reset by every `render`: `View original` answers about this newsletter and
    /// must not carry to the next.
    rendering: Rendering,
    /// Recorded at `render` because the answer cannot change while one message is on screen. It
    /// tells `View original` apart
    /// from ordinary correspondence, which is also `Rendering::Original` and must keep following
    /// the theme.
    bulk: bool,
    /// Used only while it serves exactly this body, policy and rendering.
    prepared: Option<postio_ui::reader::document::Prepared>,
    chosen: Option<Treatment>,
    remembered: Option<Treatment>,
}

type RenderedHandler = Box<dyn Fn(HeldBack)>;

type UnsubscribeHandler = Box<dyn Fn(&str)>;

/// `Clone` is cheap: every field is a GObject reference or an `Rc`.
#[derive(Clone)]
pub struct Reader {
    container: gtk::Box,
    under_header: gtk::Box,
    view: crate::body_view::BodyView,
    scroller: gtk::ScrolledWindow,
    body: gtk::Overlay,
    find: Rc<crate::body_view::find::FindBar>,
    header: Rc<MessageHeader>,
    banner: Rc<RemoteImageBanner>,
    reader_notice: Rc<crate::widgets::NoticeBar>,
    unsubscribe_banner: Rc<UnsubscribeBanner>,
    notices: Rc<NoticeSlot>,
    /// A click on the banner's button carries no data, so this is what it reports.
    unsubscribe_list: Rc<RefCell<Option<String>>>,
    on_unsubscribe: Rc<RefCell<Vec<UnsubscribeHandler>>>,
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    treatments: Rc<dyn super::SenderTreatments>,
    open: Rc<RefCell<Option<Open>>>,
    absent: Rc<std::cell::Cell<Option<Absent>>>,
    highlight: Rc<RefCell<Vec<String>>>,
    chips: super::chips::Chips,
    rendered: Rc<RefCell<Vec<RenderedHandler>>>,
    paints: Rc<std::cell::Cell<u32>>,
    loads: Rc<std::cell::Cell<u32>>,
    document: Rc<RefCell<String>>,
    place: Rc<Place>,
}

impl Drop for Reader {
    /// Balances [`postio_ui::reader::cost::note_surface_created`] so `surfaces_held` means what it
    /// says: a conversation that
    /// keeps every surface it opened is the ADR 0032 defect.
    fn drop(&mut self) {
        postio_ui::reader::cost::note_surface_released();
    }
}

/// A different message starts at the top; a redraw of the same content (late body, shown images)
/// keeps the place. Never holds
/// the view: its signal handlers hold a `Place`, and the reverse would be a cycle that keeps every
/// renderer alive.
struct Place {
    source: Rc<dyn BlobSource>,
    plain: RefCell<String>,
    keep: std::cell::Cell<bool>,
    shown: RefCell<String>,
    fetch: RefCell<Option<Rc<RemoteFetch>>>,
    remote: RefCell<std::collections::HashMap<String, Vec<u8>>>,
    asked: RefCell<std::collections::HashSet<String>>,
    /// Consent loads images at once rather than after the dwell.
    consented: std::cell::Cell<bool>,
    flow: std::cell::Cell<bool>,
    treatments: std::cell::Cell<bool>,
    treated: std::cell::Cell<Option<Treated>>,
    line: RefCell<Option<Rc<RenderModeLine>>>,
    on_treatment: RefCell<Vec<TreatmentHandler>>,
}

type TreatmentHandler = Rc<dyn Fn(Treatment)>;

pub type RemoteFetch = dyn Fn(Vec<String>, RemoteArrived);

pub type RemoteArrived = Box<dyn FnOnce(Vec<(String, Vec<u8>)>)>;

impl Place {
    fn new(source: Rc<dyn BlobSource>) -> Self {
        Place {
            source,
            plain: RefCell::new(String::new()),
            keep: std::cell::Cell::new(false),
            shown: RefCell::new(String::new()),
            fetch: RefCell::new(None),
            remote: RefCell::default(),
            asked: RefCell::default(),
            consented: std::cell::Cell::new(false),
            flow: std::cell::Cell::new(false),
            treatments: std::cell::Cell::new(false),
            treated: std::cell::Cell::new(None),
            line: RefCell::new(None),
            on_treatment: RefCell::new(Vec::new()),
        }
    }
}

impl Reader {
    /// The allow list is the one every reader of the app shares ([`super::shared_allowlist`]):
    /// loaded once, so no reader can
    /// disagree about who is allowed.
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::sharing(source, &RemoteImageAllowList::path())
    }

    pub fn sharing(source: Rc<dyn BlobSource>, allowlist_path: &std::path::Path) -> Self {
        Self::build(
            source,
            super::shared_allowlist(allowlist_path),
            allowlist_path.to_owned(),
        )
    }

    pub fn with_allowlist(
        source: Rc<dyn BlobSource>,
        allowlist: RemoteImageAllowList,
        allowlist_path: std::path::PathBuf,
    ) -> Self {
        Self::build(source, Rc::new(RefCell::new(allowlist)), allowlist_path)
    }

    fn build(
        source: Rc<dyn BlobSource>,
        allowlist: Rc<RefCell<RemoteImageAllowList>>,
        allowlist_path: std::path::PathBuf,
    ) -> Self {
        // One `Reader` is one rendering surface, counted from construction; `Drop` balances it (ADR
        // 0032).
        postio_ui::reader::cost::note_surface_created();

        let view = crate::body_view::BodyView::new(crate::body_view::DEFAULT_RENDER_DEADLINE);
        view.add_css_class("postio-reader-view");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .build();
        let place = Rc::new(Place::new(Rc::clone(&source)));
        let find = Rc::new(crate::body_view::find::FindBar::new(&view));
        let zoom_indicator = crate::body_view::zoom::ZoomIndicator::new(&view);
        zoom_indicator.widget().set_halign(gtk::Align::End);
        zoom_indicator.widget().set_valign(gtk::Align::Start);
        zoom_indicator
            .widget()
            .set_margin_top(crate::widgets::space::S2);
        zoom_indicator
            .widget()
            .set_margin_end(crate::widgets::space::S4);
        let body = gtk::Overlay::builder().child(&scroller).build();
        body.add_overlay(zoom_indicator.widget());

        let header = Rc::new(MessageHeader::new());
        let banner = Rc::new(RemoteImageBanner::new());
        let decode_notice = Rc::new(DecodeNotice::new());
        let reader_notice =
            crate::widgets::NoticeBar::new("view-reveal-symbolic", "postio-reader-view-notice");
        reader_notice.set_text("Reader view — the sender's layout, fonts and footer are hidden");
        reader_notice.set_action(Some("View original"));
        let unsubscribe_banner = Rc::new(UnsubscribeBanner::new());
        notice_keys(
            &postio_core::Keymap::resolve(&Default::default()),
            &banner,
            &reader_notice,
            &unsubscribe_banner,
        );
        let chips = super::chips::Chips::new();

        // The header does not scroll away with the body (#319): a sibling in this native box, never
        // markup inside the document.
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&header.widget());
        let under_header = gtk::Box::new(gtk::Orientation::Vertical, 0);
        under_header.add_css_class("postio-reader-under-header");
        under_header.set_visible(false);
        container.append(&under_header);
        let notices = Rc::new(NoticeSlot::new([
            decode_notice.widget(),
            reader_notice.widget(),
            banner.widget(),
            unsubscribe_banner.widget(),
        ]));
        container.append(&notices.widget());
        container.append(find.widget());
        container.append(&body);
        container.append(&chips.widget());
        let reader = Reader {
            container,
            under_header,
            view,
            scroller,
            body,
            find,
            header,
            banner,
            reader_notice,
            unsubscribe_banner,
            notices,
            unsubscribe_list: Rc::new(RefCell::new(None)),
            on_unsubscribe: Rc::new(RefCell::new(Vec::new())),
            treatments: super::treatments_for(&allowlist, &allowlist_path),
            allowlist,
            open: Rc::new(RefCell::new(None)),
            absent: Rc::new(std::cell::Cell::new(None)),
            highlight: Rc::new(RefCell::new(Vec::new())),
            chips,
            rendered: Rc::new(RefCell::new(Vec::new())),
            paints: Rc::new(std::cell::Cell::new(0)),
            loads: Rc::new(std::cell::Cell::new(0)),
            document: Rc::new(RefCell::new(String::new())),
            place,
        };

        // Weakly: the banner's buttons live in its own widget tree, so a closure their `clicked`
        // signal owns must not hold a
        // strong `Rc` back to the struct that owns them, a cycle nothing would free.
        {
            let weak = Rc::downgrade(&reader.reader_notice);
            let view = reader.view.clone();
            {
                let open = Rc::clone(&reader.open);
                let document = Rc::clone(&reader.document);
                let loads = Rc::clone(&reader.loads);
                let place = Rc::clone(&reader.place);
                let notices = Rc::clone(&reader.notices);
                view.connect_view_source(move |view| {
                    let source = source_document(open.borrow().as_ref());
                    load_document(
                        &Canvas {
                            view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
                        },
                        &source,
                    );
                });
            }

            let open = Rc::clone(&reader.open);
            let allowlist = Rc::clone(&reader.allowlist);
            // Weakly: the banner's own closures hold the notice, so a strong reference back would
            // be a cycle that leaks a render
            // thread and a snapshot per message (`gtk_reader_teardown`).
            let banner_from_notice = Rc::downgrade(&reader.banner);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
            let document = Rc::clone(&reader.document);
            reader.reader_notice.connect_action(move || {
                let Some(notice) = weak.upgrade() else { return };
                let Some(banner) = banner_from_notice.upgrade() else {
                    return;
                };
                {
                    let mut guard = open.borrow_mut();
                    let Some(current) = guard.as_mut() else {
                        return;
                    };
                    if current.rendering == Rendering::Original {
                        return;
                    }
                    current.rendering = Rendering::Original;
                }
                let allowed = open
                    .borrow()
                    .as_ref()
                    .and_then(|current| current.sender.clone())
                    .is_some_and(|sender| allowlist.borrow().is_allowed(&sender));
                place.keep.set(true);
                render_open(
                    &Canvas {
                        view: &view,
                        document: &document,
                        loads: &loads,
                        place: &place,
                        notices: &notices,
                    },
                    &banner,
                    &notice,
                    &open,
                    &highlight,
                    if allowed {
                        RemoteImages::Allowed
                    } else {
                        RemoteImages::Blocked
                    },
                    &rendered,
                );
            });
        }

        let banner_weak = Rc::downgrade(&reader.banner);
        {
            let view = reader.view.clone();
            let open = Rc::clone(&reader.open);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let notice_weak = Rc::downgrade(&reader.reader_notice);
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
            let document = Rc::clone(&reader.document);
            let banner_weak = banner_weak.clone();
            reader.banner.connect_show_once(move || {
                let Some(reader_notice) = notice_weak.upgrade() else {
                    return;
                };
                if let Some(banner) = banner_weak.upgrade() {
                    place.keep.set(true);
                    place.consented.set(true);
                    render_open(
                        &Canvas {
                            view: &view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
                        },
                        &banner,
                        &reader_notice,
                        &open,
                        &highlight,
                        RemoteImages::Allowed,
                        &rendered,
                    );
                }
            });
        }
        {
            let view = reader.view.clone();

            let open = Rc::clone(&reader.open);
            let allowlist = Rc::clone(&reader.allowlist);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let notice_weak = Rc::downgrade(&reader.reader_notice);
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
            let document = Rc::clone(&reader.document);
            reader.banner.connect_always_allow(move || {
                let Some(reader_notice) = notice_weak.upgrade() else {
                    return;
                };
                let sender = open.borrow().as_ref().and_then(|o| o.sender.clone());
                if let Some(sender) = sender {
                    let mut allowlist = allowlist.borrow_mut();
                    allowlist.allow(&sender);
                    if let Err(error) = allowlist.save_to(&allowlist_path) {
                        glib::g_warning!(
                            "postio",
                            "could not save the remote-image allow list: {error}"
                        );
                    }
                }
                if let Some(banner) = banner_weak.upgrade() {
                    place.keep.set(true);
                    place.consented.set(true);
                    render_open(
                        &Canvas {
                            view: &view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
                        },
                        &banner,
                        &reader_notice,
                        &open,
                        &highlight,
                        RemoteImages::Allowed,
                        &rendered,
                    );
                }
            });
        }

        {
            let unsubscribe_list = Rc::clone(&reader.unsubscribe_list);
            let on_unsubscribe = Rc::clone(&reader.on_unsubscribe);
            reader.unsubscribe_banner.connect_unsubscribe(move || {
                if let Some(list) = unsubscribe_list.borrow().clone() {
                    for handler in on_unsubscribe.borrow().iter() {
                        handler(&list);
                    }
                }
            });
        }

        reader.reset();
        reader
    }

    /// Starts font discovery (once per process) off the main thread, so the first message is
    /// rendered in full rather than
    /// falling back at the deadline.
    pub fn warm(&self) {
        crate::body_view::prewarm_fonts();
    }

    pub fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    pub fn view(&self) -> &crate::body_view::BodyView {
        &self.view
    }

    pub fn notice_slot(&self) -> gtk::Widget {
        self.notices.widget()
    }

    /// A body prepared under the wrong policy is simply drawn again when shown.
    pub fn remote_images_for(&self, sender: Option<&str>) -> RemoteImages {
        if sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(sender)) {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        }
    }

    pub fn banner_visible(&self) -> bool {
        self.notices.shows(Notice::RemoteImages)
    }

    pub fn header(&self) -> Rc<MessageHeader> {
        Rc::clone(&self.header)
    }

    pub fn banner_always_allow_label(&self) -> String {
        self.banner.always_allow_label()
    }

    /// Only when the message has what the banner offers (images held back, a list to leave),
    /// whichever notice the slot shows.
    /// The registry entries these answer were buttons only, so a person without a pointer could not
    /// reach them.
    pub fn run_banner_command(&self, command: postio_core::CommandId) -> bool {
        use postio_core::CommandId;
        match command {
            CommandId::ShowImages if self.notices.wanted(Notice::RemoteImages) => {
                self.banner.emit_show_once()
            }
            CommandId::AlwaysShowImages if self.notices.wanted(Notice::RemoteImages) => {
                self.banner.emit_always_allow()
            }
            CommandId::Unsubscribe if self.notices.wanted(Notice::Unsubscribe) => {
                self.unsubscribe_banner.emit_unsubscribe()
            }
            _ => return false,
        }
        true
    }

    pub fn click_always_allow(&self) {
        self.banner.emit_always_allow();
    }

    pub fn click_show_once(&self) {
        self.banner.emit_show_once();
    }

    pub fn unsubscribe_banner_visible(&self) -> bool {
        self.notices.shows(Notice::Unsubscribe)
    }

    pub fn unsubscribe_banner_label(&self) -> String {
        self.unsubscribe_banner.label()
    }

    pub fn click_unsubscribe(&self) {
        self.unsubscribe_banner.emit_unsubscribe();
    }

    /// Independent of [`render`](Self::render): the envelope is known as soon as headers have
    /// synced, well before a body, so
    /// a header-only message gets the same header.
    pub fn set_message_header(
        &self,
        from: &[postio_model::address::EmailAddress],
        to: &[postio_model::address::EmailAddress],
        cc: &[postio_model::address::EmailAddress],
        subject: Option<&str>,
        date: chrono::DateTime<chrono::Utc>,
    ) {
        self.header.set_message(from, to, cc, subject, date);
    }

    pub fn set_sender_warning(&self, line: Option<&str>) {
        self.header.set_warning(line);
    }

    #[doc(hidden)]
    pub fn sender_warning_label(&self) -> Option<String> {
        self.header.warning_label()
    }

    pub fn set_account(&self, name: Option<&str>, hue: usize) {
        self.header.set_account(name, hue);
    }

    #[doc(hidden)]
    pub fn account_label(&self) -> Option<String> {
        self.header.account_label()
    }

    /// `sender` is the allow-list key: a sender already on the standing list loads remote images
    /// with no banner. Otherwise
    /// they stay blocked until "show once" or "always allow", which re-render through the same
    /// [`Open`] state.
    pub fn render(&self, body: &MessageBody, sender: Option<&str>) {
        self.render_prepared(body, sender, None);
    }

    /// `prepared` is `postio_ui::reader::document::prepare_treated` run on a worker under the
    /// policy
    /// [`remote_images_for`](Self::remote_images_for) gives this sender, so the two html5ever
    /// parses are not paid on the main
    /// thread. Without it, or prepared for something else, it draws as `render` does.
    pub fn render_prepared(
        &self,
        body: &MessageBody,
        sender: Option<&str>,
        prepared: Option<postio_ui::reader::document::Prepared>,
    ) {
        self.paints.set(self.paints.get() + 1);
        self.absent.set(None);
        // Cleared here: a caveat outliving its message would put "these may not be the sender's
        // words" over mail that decoded
        // perfectly, and a sometimes-wrong warning is learned to be ignored. Callers set it again
        // through `set_encoding_problems`.
        self.notices.want(Notice::Decode, false);
        self.set_unsubscribe(None);
        // Whether it reads as bulk is still asked: it picks the sheet the original is drawn on. The
        // same body from the same
        // sender keeps its place; another starts at the top, since this pane has no message
        // identity.
        let same = self
            .open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.body == *body && open.sender.as_deref() == sender);
        self.place.keep.set(same);
        // What a render past its deadline shows: the message's own text part. Deriving text from
        // the HTML would put an html5ever
        // parse back on this thread, which `render_prepared` exists to keep off.
        self.place
            .plain
            .replace(body.text.clone().unwrap_or_default());
        let bulk = prepared
            .as_ref()
            .and_then(|prepared| prepared.verdict_for(body))
            .unwrap_or_else(|| postio_ui::reader::document::suits_reader_view(body));
        let rendering = postio_ui::reader::document::opening_rendering();
        let remembered = sender.and_then(|sender| self.treatments.treatment_for(sender));
        *self.open.borrow_mut() = Some(Open {
            body: body.clone(),
            sender: sender.map(str::to_owned),
            rendering,
            bulk,
            prepared,
            chosen: remembered,
            remembered,
        });
        let allowed = sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(sender));
        let remote = if allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        render_open(
            &self.canvas(),
            &self.banner,
            &self.reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
    }

    /// A no-op for a message with no markup of its own.
    pub fn toggle_reader_view(&self) {
        {
            let mut guard = self.open.borrow_mut();
            let Some(open) = guard.as_mut() else { return };
            if open
                .body
                .html
                .as_deref()
                .is_none_or(|html| html.trim().is_empty())
            {
                return;
            }
            open.rendering = if open.rendering == Rendering::Reader {
                Rendering::Original
            } else {
                Rendering::Reader
            };
        }
        self.rerender();
    }

    /// Draws the message in its owner's column: header, notices, body and parts scroll together in
    /// `scroller`, the body flat on
    /// the column's ground. What the view rasterises is still only what `scroller` shows.
    /// The find bar leaves the column: a bar at the column's top is only on screen at the top, so
    /// opening it would scroll the
    /// message there. The owner places [`find_bar`](Self::find_bar) above `scroller`. Called once,
    /// before anything is shown.
    pub fn flow_in(&self, scroller: &gtk::ScrolledWindow) {
        self.scroller.set_child(None::<&gtk::Widget>);
        self.body.set_child(Some(&self.view));
        self.view.flow_in(scroller);
        self.place.flow.set(true);
        self.chips.set_cards(true);
        self.notices.set_collapsing(true);
        if let Some(container) = self.find.widget().parent().and_downcast::<gtk::Box>() {
            container.remove(self.find.widget());
            let ground = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            ground.add_css_class("postio-flow-ground");
            ground.set_accessible_role(gtk::AccessibleRole::Presentation);
            container.append(&ground);
            self.view.set_ground(&ground);
            for (variable, class) in FLOW_PALETTE {
                let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                probe.add_css_class(class);
                probe.set_accessible_role(gtk::AccessibleRole::Presentation);
                container.append(&probe);
                self.view.add_palette_probe(variable, &probe);
            }
        }
    }

    /// Focus's open message calls this once, before anything is shown; a reader that does not draws
    /// as it always has.
    pub fn use_treatments(&self) {
        if self.place.treatments.replace(true) {
            return;
        }
        let line = RenderModeLine::new();
        line.set_key(
            postio_core::Keymap::resolve(&Default::default())
                .binding(postio_core::CommandId::SwitchTreatment),
        );
        let switcher = self.switcher();
        line.connect_switch({
            let switcher = switcher.clone();
            move || {
                switcher.switch();
            }
        });
        line.connect_always(move || {
            switcher.remember();
        });
        self.container
            .insert_child_after(&line.widget(), Some(&self.notices.widget()));
        // A body that fell back to plain text is in neither treatment; the next snapshot of the
        // sender's markup puts it back.
        let place = Rc::downgrade(&self.place);
        let open = Rc::downgrade(&self.open);
        self.view.connect_rendered(move |view| {
            let (Some(place), Some(open)) = (place.upgrade(), open.upgrade()) else {
                return;
            };
            let Some(line) = place.line.borrow().clone() else {
                return;
            };
            match view.document().map(|document| document.outcome) {
                Some(postio_render::Outcome::FellBack(reason)) if place.treated.get().is_some() => {
                    line.show_fallback(reason.why());
                }
                Some(postio_render::Outcome::Rendered) if line.is_fallback() => {
                    let remembered = open.borrow().as_ref().and_then(|open| open.remembered);
                    show_treatment(&place, remembered);
                }
                _ => {}
            }
        });
        self.place.line.replace(Some(line));
    }

    /// App colours when nothing is drawn under a treatment.
    pub fn treatment(&self) -> Treatment {
        self.place
            .treated
            .get()
            .map(|treated| treated.shown)
            .unwrap_or_default()
    }

    pub fn treated(&self) -> Option<Treated> {
        self.place.treated.get()
    }

    /// The same treatment can arrive twice in a row; a caller sizing a column to it is idempotent.
    pub fn connect_treatment_changed(&self, handler: impl Fn(Treatment) + 'static) {
        self.place.on_treatment.borrow_mut().push(Rc::new(handler));
    }

    /// For this message only: the next opens as the rule or its sender's remembered choice says.
    /// False when there is no HTML
    /// body under a treatment to switch.
    pub fn switch_treatment(&self) -> bool {
        self.switcher().switch().is_some()
    }

    #[doc(hidden)]
    pub fn render_mode_line(&self) -> Option<Rc<RenderModeLine>> {
        self.place.line.borrow().clone()
    }

    /// 1.0 when it fitted or is not on paper, never under `postio_render::render::PAPER_FIT_FLOOR`,
    /// below which the sheet
    /// scrolls sideways. Read off the snapshot on screen, so a column that changes width is fitted
    /// again.
    pub fn paper_fit(&self) -> f64 {
        self.view.document().map_or(1.0, |document| document.fit)
    }

    fn switcher(&self) -> Switcher {
        Switcher {
            view: self.view.downgrade(),
            document: Rc::clone(&self.document),
            loads: Rc::clone(&self.loads),
            place: Rc::downgrade(&self.place),
            notices: Rc::clone(&self.notices),
            banner: Rc::downgrade(&self.banner),
            reader_notice: Rc::downgrade(&self.reader_notice),
            open: Rc::clone(&self.open),
            highlight: Rc::clone(&self.highlight),
            rendered: Rc::clone(&self.rendered),
            allowlist: Rc::clone(&self.allowlist),
            treatments: Rc::clone(&self.treatments),
        }
    }

    /// Test-facing: without an observable the scrolling methods returned having done nothing and
    /// every caller reported success (#1431).
    #[doc(hidden)]
    pub fn scrolled_for_test(&self) -> f64 {
        self.view.scrolled()
    }

    /// Per message, not sticky: wanting one newsletter's layout says nothing about the next. A
    /// no-op when the pane is empty or
    /// already original.
    pub fn view_original(&self) {
        {
            let mut guard = self.open.borrow_mut();
            let Some(open) = guard.as_mut() else { return };
            if open.rendering == Rendering::Original {
                return;
            }
            open.rendering = Rendering::Original;
        }
        self.rerender();
    }

    /// The drawn state, not an intention.
    pub fn is_reader_view(&self) -> bool {
        self.open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.rendering == Rendering::Reader)
    }

    pub fn reader_notice_visible(&self) -> bool {
        self.notices.shows(Notice::ReaderView)
    }

    pub fn click_view_original(&self) {
        self.reader_notice.press_action();
    }

    /// The message has not changed, only the way it is drawn, so re-entering `render` would reset
    /// the choice just made.
    fn rerender(&self) {
        let allowed = self
            .open
            .borrow()
            .as_ref()
            .and_then(|open| open.sender.clone())
            .is_some_and(|sender| self.allowlist.borrow().is_allowed(&sender));
        let remote = if allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        render_open(
            &self.canvas(),
            &self.banner,
            &self.reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
    }

    fn canvas(&self) -> Canvas<'_> {
        Canvas {
            view: &self.view,
            document: &self.document,
            loads: &self.loads,
            place: &self.place,
            notices: &self.notices,
        }
    }

    pub fn connect_rendered(&self, handler: impl Fn(HeldBack) + 'static) {
        self.rendered.borrow_mut().push(Box::new(handler));
    }

    pub fn set_under_header(&self, widget: Option<&gtk::Widget>) {
        while let Some(child) = self.under_header.first_child() {
            self.under_header.remove(&child);
        }
        if let Some(widget) = widget {
            self.under_header.append(widget);
        }
        self.under_header.set_visible(widget.is_some());
    }

    /// Metadata only: a chip is drawn from what `BODYSTRUCTURE` said, so a message nothing was
    /// fetched for still shows what came with it.
    pub fn set_attachments(&self, root: &str, parts: &[postio_model::Attachment]) {
        self.chips.set_parts(root, parts);
    }

    pub fn connect_attachment(&self, handler: impl Fn(&postio_ui::reader::parts::Node) + 'static) {
        self.chips.connect_activated(handler);
    }

    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        // From the same keymap: written down here they would go on saying `C-o` after a rebind
        // (#1002).
        notice_keys(
            keymap,
            &self.banner,
            &self.reader_notice,
            &self.unsubscribe_banner,
        );
        if let Some(line) = self.place.line.borrow().as_ref() {
            line.set_key(keymap.binding(postio_core::CommandId::SwitchTreatment));
        }
    }

    /// The owner's, because this crate speaks no protocol. Asked only for a document that names
    /// remote images, which only an
    /// allowed sender or "show once" produces.
    pub fn set_remote_fetch(&self, fetch: impl Fn(Vec<String>, RemoteArrived) + 'static) {
        self.place.fetch.replace(Some(Rc::new(fetch)));
    }

    /// The reading position stays: in a column the bar is above it, and the first match is the
    /// first from where the person is reading.
    pub fn find_in_message(&self) {
        self.find.open();
    }

    pub fn close_find(&self) {
        self.find.close();
    }

    pub fn find_step(&self, forward: bool) {
        self.view.find_step(forward);
    }

    pub fn find_bar(&self) -> &crate::body_view::find::FindBar {
        &self.find
    }

    pub fn finding(&self) -> bool {
        self.find.widget().is_search_mode()
    }

    pub fn zoom_in(&self) {
        self.view.zoom_in();
    }

    pub fn zoom_out(&self) {
        self.view.zoom_out();
    }

    pub fn zoom_reset(&self) {
        self.view.zoom_reset();
    }

    pub fn zoom(&self) -> u16 {
        self.view.zoom()
    }

    pub fn set_zoom(&self, percent: u16) {
        self.view.set_zoom_percent(percent);
    }

    pub fn connect_zoom_changed(&self, f: impl Fn(u16) + 'static) {
        self.view
            .connect_local("zoom-changed", false, move |values| {
                let view = values[0]
                    .get::<crate::body_view::BodyView>()
                    .expect("the signal's own view");
                f(view.zoom());
                None
            });
    }

    /// Marking happens after sanitizing (see [`postio_ui::search::mark_html`]), so nothing loosens
    /// what the reader renders.
    /// Takes effect on the next [`Reader::render`].
    pub fn set_highlight(&self, terms: Vec<String>) {
        *self.highlight.borrow_mut() = terms;
    }

    /// Stays inside the same body view, built by [`wrap_document`] with remote images blocked, so a
    /// state plate can no more
    /// reach the network than a message can.
    pub fn show_absent(&self, state: Absent) {
        self.paints.set(self.paints.get() + 1);
        *self.open.borrow_mut() = None;
        self.absent.set(Some(state));
        // A plate has no text of its own and must not fall back to the previous message's.
        self.place.plain.replace(String::new());
        self.place.treated.set(None);
        show_treatment(&self.place, None);
        // Every notice, not only the images banner: reader view, a decode caveat and the list were
        // about the message before.
        self.set_unsubscribe(None);
        self.notices.clear();
        load_document(
            &self.canvas(),
            &wrap_document(&absent_html(state), RemoteImages::Blocked, Sheet::Theme),
        );
        // No body drawn, so nothing is held back: `connect_rendered` watchers must not keep the
        // previous count.
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
        }
    }

    /// Test-facing: tells a coalesced repaint from an idempotent one (#396).
    #[doc(hidden)]
    pub fn paints(&self) -> u32 {
        self.paints.get()
    }

    /// Counts the times that cost a render: an arrival that recomposes the document byte-for-byte
    /// identically is a paint that
    /// must not be a load (#749).
    #[doc(hidden)]
    pub fn loads(&self) -> u32 {
        self.loads.get()
    }

    #[doc(hidden)]
    pub fn test_document(&self) -> String {
        self.document.borrow().clone()
    }

    pub fn absent(&self) -> Option<Absent> {
        self.absent.get()
    }

    /// Says whether the body on screen is a guess rather than what was sent: base64 outside its
    /// alphabet, an unknown
    /// `Content-Transfer-Encoding`, or a charset that lost octets to U+FFFD (#901). Call it after
    /// [`render`](Self::render), which clears it.
    pub fn set_encoding_problems(&self, problems: bool) {
        self.notices.want(Notice::Decode, problems);
    }

    pub fn shows_encoding_problems(&self) -> bool {
        self.notices.shows(Notice::Decode)
    }

    /// Call it after [`render`](Self::render), which clears it. `list_identifier` is the `List-Id`
    /// or the sender's domain,
    /// whichever the caller found.
    pub fn set_unsubscribe(&self, list_identifier: Option<&str>) {
        *self.unsubscribe_list.borrow_mut() = list_identifier.map(str::to_owned);
        self.unsubscribe_banner.set_list(list_identifier);
        self.notices
            .want(Notice::Unsubscribe, list_identifier.is_some());
    }

    /// The reader only asks; a caller decides what leaving a list means (this crate has no SQL to
    /// log it).
    pub fn connect_unsubscribe_activated(&self, handler: impl Fn(&str) + 'static) {
        self.on_unsubscribe.borrow_mut().push(Box::new(handler));
    }

    pub fn clear(&self) {
        self.reset();
        load_document(
            &self.canvas(),
            &wrap_document("", RemoteImages::Blocked, Sheet::Theme),
        );
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
        }
    }

    /// A new reader starts from this (#1603): the window's reader is built before the first frame,
    /// so the constructor must not render.
    fn reset(&self) {
        *self.open.borrow_mut() = None;
        self.absent.set(None);
        self.place.treated.set(None);
        show_treatment(&self.place, None);
        self.place.plain.replace(String::new());
        self.header.clear();
        self.set_unsubscribe(None);
        self.notices.clear();
    }

    fn showing(&self) -> bool {
        self.open.borrow().is_some()
    }

    /// Does not move the keyboard (#438). A no-op with nothing open and at the end of the document.
    pub fn page_down(&self) {
        if !self.showing() {
            return;
        }
        self.view.page(true);
    }

    pub fn scroll_lines(&self, lines: i32) {
        if self.showing() {
            self.view.scroll_lines(lines);
        }
    }

    pub fn scroll_to_edge(&self, bottom: bool) {
        if self.showing() {
            self.view.scroll_to_edge(bottom);
        }
    }

    pub fn page_up(&self) {
        if !self.showing() {
            return;
        }
        self.view.page(false);
    }
}

struct Canvas<'a> {
    view: &'a crate::body_view::BodyView,
    document: &'a RefCell<String>,
    loads: &'a Rc<std::cell::Cell<u32>>,
    place: &'a Rc<Place>,
    notices: &'a NoticeSlot,
}

/// The view and the place are held weakly: the line's own buttons keep a `Switcher`, the line lives
/// in the reader's widgets
/// and the view's handlers hold the place, so a strong hold would be a cycle keeping every renderer
/// alive.
#[derive(Clone)]
struct Switcher {
    view: glib::WeakRef<crate::body_view::BodyView>,
    document: Rc<RefCell<String>>,
    loads: Rc<std::cell::Cell<u32>>,
    place: std::rc::Weak<Place>,
    notices: Rc<NoticeSlot>,
    banner: std::rc::Weak<RemoteImageBanner>,
    reader_notice: std::rc::Weak<crate::widgets::NoticeBar>,
    open: Rc<RefCell<Option<Open>>>,
    highlight: Rc<RefCell<Vec<String>>>,
    rendered: Rc<RefCell<Vec<RenderedHandler>>>,
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    treatments: Rc<dyn super::SenderTreatments>,
}

impl Switcher {
    fn switch(&self) -> Option<Treatment> {
        let place = self.place.upgrade()?;
        let view = self.view.upgrade()?;
        let banner = self.banner.upgrade()?;
        let reader_notice = self.reader_notice.upgrade()?;
        let next = place
            .treated
            .get()
            .filter(|treated| treated.html)?
            .shown
            .other();
        let sender = {
            let mut open = self.open.borrow_mut();
            let current = open.as_mut()?;
            current.chosen = Some(next);
            current.sender.clone()
        };
        let remote = if sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(&sender)) {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        place.keep.set(true);
        render_open(
            &Canvas {
                view: &view,
                document: &self.document,
                loads: &self.loads,
                place: &place,
                notices: &self.notices,
            },
            &banner,
            &reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
        // An outcome, never content: which treatment, not whose mail.
        tracing::debug!(
            treatment = next.attribute_value(),
            "switched the open message's treatment"
        );
        Some(next)
    }

    fn remember(&self) -> Option<Treatment> {
        let place = self.place.upgrade()?;
        let shown = place.treated.get().filter(|treated| treated.html)?.shown;
        let sender = {
            let mut open = self.open.borrow_mut();
            let current = open.as_mut()?;
            current.chosen = Some(shown);
            current.remembered = Some(shown);
            current.sender.clone()?
        };
        self.treatments.set_treatment(&sender, Some(shown));
        show_treatment(&place, Some(shown));
        tracing::info!(
            treatment = shown.attribute_value(),
            "remembered a treatment for a sender"
        );
        Some(shown)
    }
}

/// Every call is a whole render on the render thread; the tally is what a test holds it to. Whether
/// a load is needed is not
/// decided here: this pane has a body and a sender, not a message identity, so it cannot tell a
/// second identical message
/// from one arriving twice. That belongs in the app that owns the store.
fn load_document(canvas: &Canvas<'_>, document: &str) {
    canvas.loads.set(canvas.loads.get() + 1);
    // The single choke point every render passes through, so the place to count from.
    postio_ui::reader::cost::note_render();
    canvas.document.replace(document.to_owned());
    let place = canvas.place;
    place.shown.replace(document.to_owned());
    let content = content_for(document, place);
    if place.keep.replace(false) {
        canvas.view.set_content(content);
    } else {
        canvas.view.set_content_from_top(content);
    }
    fetch_remote(canvas.view, place, document);
}

fn content_for(document: &str, place: &Place) -> crate::body_view::Content {
    let resources = resources_for(document, &*place.source);
    let remote = place.remote.borrow();
    for url in remote_image_urls(document) {
        if let Some(bytes) = remote.get(&url) {
            resources.insert_remote(&url, bytes.clone());
        }
    }
    let flowed = |document: String| {
        if place.flow.get() {
            flow_document(&document)
        } else {
            document
        }
    };
    crate::body_view::Content {
        document: flowed(document.to_owned()),
        resources: std::sync::Arc::new(resources),
        plain_text: place.plain.borrow().clone(),
        fallback: Some(fallback_for(place, flowed)),
        over_cap: None,
    }
}

/// The same column, face and rhythm as every other body, not the renderer's own page. Under a
/// treatment the render-mode line
/// says the body fell back; elsewhere the document does.
fn fallback_for(place: &Place, flowed: impl Fn(String) -> String) -> postio_render::Fallback {
    use postio_ui::reader::document;
    let plain = document::plain_text_body_html(&place.plain.borrow());
    let treated = place.treated.get().is_some();
    let html = if treated {
        document::document_for_treated(&plain, "", RemoteImages::Blocked, Treatment::AppColours)
    } else {
        document::document_for(&plain, "", RemoteImages::Blocked, document::Sheet::Theme)
    };
    postio_render::Fallback {
        document: flowed(html),
        notice: !treated,
    }
}

/// The ground is the column's own, so the body needs no page; its frame and padding go and the
/// column decides the measure.
/// Mail that paints its own page keeps a quiet hairline frame, its only edge against the column.
/// The ground is
/// `--flow-ground`, which the view defines at each render from the column's ground token
/// (`BodyView::set_ground`): no
/// colour is written here, so body and dialog cannot disagree in light or dark.
const FLOW_CSS: &str = "\nbody { background: var(--flow-ground); padding: 0; }\n";

/// Correspondence has no page of its own: the body loses its frame and padding so its lines share
/// the column's edges. The
/// rhythm inside the body is the app colours treatment's, in `postio-ui`'s `treatment.css`.
const FLOW_FLAT_CSS: &str =
    ".postio-body { padding: 0; border: 0; border-radius: 0; min-height: 0; }\n";

/// A probe nobody styles reads as the toolkit's foreground, the ink: a safe answer for all but the
/// ground.
pub const FLOW_PALETTE: [(&str, &str); 6] = [
    ("--r-ink", "postio-flow-ink"),
    ("--r-ink-secondary", "postio-flow-ink-secondary"),
    ("--r-dim", "postio-flow-dim"),
    ("--r-accent", "postio-flow-accent"),
    ("--r-hairline", "postio-flow-hairline"),
    ("--r-hairline-strong", "postio-flow-hairline-strong"),
];

fn flow_document(document: &str) -> String {
    let own_page = document.contains("class=\"postio-canvas\"")
        || document.contains(&format!(
            "class=\"{}\"",
            postio_ui::reader::document::SENDERS_SHEET_CLASS
        ))
        || document.contains(
            &postio_ui::reader::document::contain_body_treated("", Treatment::Paper)
                .replace("</div>", ""),
        );
    let mut css = String::from(FLOW_CSS);
    if !own_page {
        css.push_str(FLOW_FLAT_CSS);
    }
    document.replacen("</style>", &format!("{css}</style>"), 1)
}

/// A document names a remote image only when its sender is allowed or the user chose "Show once". A
/// message counts as opened
/// once it has stayed on screen for [`DWELL_TO_READ`], so a cursor sweeping past fetches nothing,
/// or at once when the load
/// is the user's own consent.
///
/// [`DWELL_TO_READ`]: postio_ui::dwell::DWELL_TO_READ
fn fetch_remote(view: &crate::body_view::BodyView, place: &Rc<Place>, document: &str) {
    if remote_image_urls(document).is_empty() {
        return;
    }
    if place.consented.replace(false) {
        ask_remote(view, place, document);
        return;
    }
    let view = view.downgrade();
    let weak = Rc::downgrade(place);
    let document = document.to_owned();
    glib::timeout_add_local_once(postio_ui::dwell::DWELL_TO_READ, move || {
        let (Some(view), Some(place)) = (view.upgrade(), weak.upgrade()) else {
            return;
        };
        if *place.shown.borrow() == document {
            ask_remote(&view, &place, &document);
        }
    });
}

fn ask_remote(view: &crate::body_view::BodyView, place: &Rc<Place>, document: &str) {
    let Some(fetch) = place.fetch.borrow().clone() else {
        return;
    };
    let wanted: Vec<String> = {
        let mut asked = place.asked.borrow_mut();
        remote_image_urls(document)
            .into_iter()
            .filter(|url| asked.insert(url.clone()))
            .collect()
    };
    if wanted.is_empty() {
        return;
    }
    let view = view.downgrade();
    let weak = Rc::downgrade(place);
    let document = document.to_owned();
    fetch(
        wanted,
        Box::new(move |arrived| {
            let (Some(view), Some(place)) = (view.upgrade(), weak.upgrade()) else {
                return;
            };
            if arrived.is_empty() {
                return;
            }
            place.remote.borrow_mut().extend(arrived);
            // One redraw for the whole batch, and only of the document it was asked for.
            if *place.shown.borrow() == document {
                view.set_content(content_for(&document, &place));
            }
        }),
    );
}

fn source_document(open: Option<&Open>) -> String {
    let mut out = String::new();
    if let Some(body) = open.map(|open| &open.body) {
        let source = body.html.as_deref().or(body.text.as_deref()).unwrap_or("");
        out.push_str("<pre class=\"postio-source\" style=\"white-space: pre-wrap\">");
        for c in source.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
        out.push_str("</pre>");
    }
    wrap_document(&out, RemoteImages::Blocked, Sheet::Theme)
}

/// Never a link's `href`, a cursor or anything else CSS can name: the contract allows images and
/// backgrounds only.
fn remote_image_urls(document: &str) -> Vec<String> {
    let decode = |url: &str| {
        url.replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    };
    let mut urls: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if (url.starts_with("http://") || url.starts_with("https://")) && !urls.contains(&url) {
            urls.push(url);
        }
    };
    for attribute in ["src=\"", "background=\""] {
        for piece in document.split(attribute).skip(1) {
            if let Some(end) = piece.find('"') {
                push(decode(&piece[..end]));
            }
        }
    }
    let mut pieces = document.split("url(");
    let mut before = pieces.next().unwrap_or("");
    for piece in pieces {
        let declaration = before
            .rfind([';', '{', '"', '\''])
            .map_or(before, |at| &before[at + 1..]);
        let is_background = declaration.trim_start().starts_with("background");
        before = piece;
        if !is_background {
            continue;
        }
        let piece = decode(piece);
        let piece = piece.trim_start();
        let (quote, rest) = match piece.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &piece[1..]),
            _ => (None, piece),
        };
        let end = match quote {
            Some(q) => rest.find(q),
            None => rest.find(')'),
        };
        if let Some(end) = end {
            push(rest[..end].trim().to_owned());
        }
    }
    urls
}

/// Resolved now, on this thread: the render thread cannot call `source` and asks for nothing it was
/// not given.
fn resources_for(document: &str, source: &dyn BlobSource) -> postio_render::Resources {
    let resources = postio_render::Resources::new();
    for face in postio_ui::reader::document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let prefix = format!("{}:", postio_body::CID_SCHEME);
    let mut rest = document;
    while let Some(at) = rest.find(&prefix) {
        rest = &rest[at + prefix.len()..];
        let end = rest
            .find(['"', '\'', ')', ' ', '>', '<'])
            .unwrap_or(rest.len());
        let reference = &rest[..end];
        let (scope, id) = match reference.split_once('/') {
            Some((scope, id)) => (Some(scope), id),
            None => (None, reference),
        };
        let id = postio_body::sanitize::percent_decode(id);
        if let Some((bytes, _)) = source.resolve_in(scope, &id) {
            resources.insert_part(scope, &id, bytes);
        }
        rest = &rest[end..];
    }
    resources
}

fn notice_keys(
    keymap: &postio_core::Keymap,
    banner: &RemoteImageBanner,
    reader_notice: &crate::widgets::NoticeBar,
    unsubscribe_banner: &UnsubscribeBanner,
) {
    use postio_core::CommandId;
    banner.set_action_key(keymap.binding(CommandId::ShowImages));
    reader_notice.set_action_key(keymap.binding(CommandId::ViewOriginal));
    unsubscribe_banner.set_action_key(keymap.binding(CommandId::Unsubscribe));
}

/// A free function so the two banner-signal closures can call it through weak captures without
/// holding a whole `Reader`,
/// which would be a reference cycle.
fn render_open(
    canvas: &Canvas<'_>,
    banner: &RemoteImageBanner,
    reader_notice: &crate::widgets::NoticeBar,
    open: &Rc<RefCell<Option<Open>>>,
    highlight: &Rc<RefCell<Vec<String>>>,
    remote: RemoteImages,
    rendered: &Rc<RefCell<Vec<RenderedHandler>>>,
) {
    let (drawn, sender, bulk) = {
        let guard = open.borrow();
        let Some(current) = guard.as_ref() else {
            return;
        };
        let drawn =
            if canvas.place.treatments.get() {
                match current.prepared.as_ref().and_then(|prepared| {
                    prepared.treated_for(&current.body, remote, current.chosen)
                }) {
                    Some(rendered) => rendered.clone(),
                    None => postio_ui::reader::document::body_html_treated(
                        &current.body,
                        remote,
                        current.chosen,
                        None,
                    ),
                }
            } else {
                match current.prepared.as_ref() {
                    Some(prepared) if prepared.serves(&current.body, remote, current.rendering) => {
                        prepared.rendered().clone()
                    }
                    _ => body_html(&current.body, remote, current.rendering),
                }
            };
        (drawn, current.sender.clone(), current.bulk)
    };
    let remembered = open
        .borrow()
        .as_ref()
        .and_then(|current| current.remembered);
    let held_back = drawn.held_back;
    let content = drawn.html.clone();
    // After sanitizing and quote-folding: ammonia would strip the `<mark>` as an unknown tag.
    let content = postio_ui::search::mark_html(&content, &highlight.borrow());

    banner.set_sender(sender.as_deref());
    // The count before the visibility: a notice that appeared and then changed what it said would
    // flicker (#1008).
    banner.set_held_back(held_back);
    canvas.notices.want(
        Notice::RemoteImages,
        remote == RemoteImages::Blocked && held_back.total() > 0,
    );

    // Only while drawn reduced: a notice offering an original already on screen is a control that
    // does nothing.
    canvas
        .notices
        .want(Notice::ReaderView, drawn.rendering == Rendering::Reader);
    if drawn.rendering == Rendering::Reader && drawn.links_dropped > 0 {
        reader_notice.set_text(&format!(
            "Reader view — {} link{} kept of {}",
            drawn.links_kept,
            if drawn.links_kept == 1 { "" } else { "s" },
            drawn.links_total()
        ));
    } else {
        reader_notice.set_text("Reader view — the sender's layout, fonts and footer are hidden");
    }

    // `Rendering::Original` alone is not enough: correspondence is Original too and must keep
    // following the theme.
    // `sheet_for` holds the rule so the frontends cannot differ.
    let sheet = sheet_for(drawn.rendering, bulk);
    let document = match drawn.treated {
        Some(treated) => postio_ui::reader::document::document_for_treated(
            &content,
            &drawn.styles,
            remote,
            treated.shown,
        ),
        None => document_for(&content, &drawn.styles, remote, sheet),
    };
    canvas.place.treated.set(drawn.treated);
    show_treatment(canvas.place, remembered);
    load_document(canvas, &document);

    for handler in rendered.borrow().iter() {
        handler(held_back);
    }
    if let Some(treated) = drawn.treated {
        let handlers = canvas.place.on_treatment.borrow().clone();
        for handler in handlers {
            handler(treated.shown);
        }
    }
}

fn show_treatment(place: &Place, remembered: Option<Treatment>) {
    if let Some(line) = place.line.borrow().as_ref() {
        line.show(place.treated.get().and_then(|treated| {
            postio_ui::reader::document::render_mode_words(treated, remembered)
        }));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_column_flattens_app_colours_and_keeps_papers_sheet() {
        use postio_ui::reader::document::document_for_treated;
        let flat = |treatment| {
            super::flow_document(&document_for_treated(
                "<p>x</p>",
                "",
                super::RemoteImages::Blocked,
                treatment,
            ))
            .contains(super::FLOW_FLAT_CSS)
        };
        assert!(
            flat(super::Treatment::AppColours),
            "app colours kept a frame"
        );
        assert!(!flat(super::Treatment::Paper), "paper lost its sheet");
    }

    #[test]
    fn the_flow_sheet_names_its_ground_and_writes_no_colour() {
        let sheet = format!("{}{}", super::FLOW_CSS, super::FLOW_FLAT_CSS);
        assert!(sheet.contains("var(--flow-ground)"), "{sheet}");
        assert!(
            !sheet.contains('#') && !sheet.contains("rgb") && !sheet.contains("--flow-ground:"),
            "the flow sheet writes a colour of its own: {sheet}"
        );
    }

    #[test]
    fn remote_image_urls_are_images_and_styles_never_links() {
        let document = r#"<a href="https://example.net/page">x</a>
            <img src="https://images.example.net/a.gif?x=1&amp;y=2">
            <img src="postio-cid:part@example.com">
            <div style="background:url(&quot;http://images.example.net/b.png&quot;)"></div>
            <style>.hero { background: url(https://images.example.net/c.png) }
            .pointer { cursor: url(https://images.example.net/cursor.png), auto }
            li { list-style-image: url(https://images.example.net/bullet.png) }</style>
            <table background="https://images.example.net/d.png"></table>
            <img src="https://images.example.net/a.gif?x=1&amp;y=2">"#;
        assert_eq!(
            super::remote_image_urls(document),
            [
                "https://images.example.net/a.gif?x=1&y=2",
                "https://images.example.net/d.png",
                "http://images.example.net/b.png",
                "https://images.example.net/c.png",
            ]
        );
    }

    #[test]
    fn the_two_schemes_do_not_share_a_ground() {
        // A palette that collapsed to one value would paint a white flash into dark mode and pass a
        // parseability-only test.
        assert_ne!(
            postio_ui::reader::document::reader_ground(false),
            postio_ui::reader::document::reader_ground(true),
            "both schemes report the same ground, so one of them is painting \
             the wrong colour between messages"
        );
    }

    use super::*;

    #[test]
    fn every_absent_body_says_which_kind_of_absent_it_is() {
        let said: Vec<String> = [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
        ]
        .iter()
        .map(|state| absent_html(*state))
        .collect();

        for (state, html) in [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
        ]
        .iter()
        .zip(&said)
        {
            assert!(
                !html.trim().is_empty(),
                "{state:?} rendered nothing, which is the bug"
            );
        }

        for (a, first) in said.iter().enumerate() {
            for (b, second) in said.iter().enumerate() {
                assert!(
                    a == b || first != second,
                    "two different reasons produced the same words, so the pane                      cannot be telling the user which one they are looking at"
                );
            }
        }
    }

    #[test]
    fn the_states_worth_retrying_name_the_retry_key() {
        let key = postio_core::registry::get(postio_core::CommandId::Refresh).default_binding;
        let named = format!("<kbd>{key}</kbd>");
        assert!(absent_html(Absent::Offline).contains(&named));
        assert!(absent_html(Absent::Missing).contains(&named));
    }

    #[test]
    fn a_message_with_no_body_is_not_offered_a_retry() {
        let html = absent_html(Absent::Empty);
        assert!(!html.contains("check for new mail"), "{html}");
    }

    #[test]
    fn a_foreign_draft_says_it_cannot_be_edited_here_and_offers_no_retry() {
        let html = absent_html(Absent::ForeignDraft);
        assert!(!html.contains("check for new mail"), "{html}");
        assert!(
            html.contains("another") || html.contains("device") || html.contains("client"),
            "should say this draft came from somewhere else: {html}"
        );
    }

    #[test]
    fn the_csp_only_allows_remote_images_when_asked() {
        assert!(!content_security_policy(RemoteImages::Blocked).contains("https:"));
        assert!(content_security_policy(RemoteImages::Allowed).contains("https:"));
    }

    #[test]
    fn the_document_carries_the_base_uri_and_the_stylesheet() {
        let doc = wrap_document("<p>hi</p>", RemoteImages::Blocked, Sheet::Theme);
        assert!(doc.contains("<style>"));
        assert!(doc.contains("<p>hi</p>"));
        assert!(doc.contains("Content-Security-Policy"));
    }
}
