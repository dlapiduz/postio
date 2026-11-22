//! The open-email dialog: `Enter` opens the conversation under the cursor over the list. Its
//! message view is one view moved between two hosts,
//! the dialog and the reading pane ([`OpenMessage::place`]), so they cannot drift. One dialog and
//! one [`Reader`] for the window's life: the header
//! is drawn from the row already in hand, and the body is drawn only if it is still the message on
//! screen when it lands.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::Body;
use postio_core::{CommandId, Keymap};
use postio_model::{Attachment, MessageBody, MessageId};
use postio_ui::focus_dialog::{self, rhythm};
use postio_ui::hints;
use postio_widgets::reader::{Reader, RemoteImageAllowList};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::S3;
use postio_widgets::widgets::{Action, ActionBar, Kind, Size};

use crate::list::FocusRow;
use crate::open_header::HeaderCard;

const WINDOW: (i32, i32) = (1440, 900);

fn toolbar() -> &'static [Action] {
    crate::verbs::actions("focus-open-", focus_dialog::OPEN_TOOLBAR)
}

fn send_toolbar() -> &'static [Action] {
    crate::verbs::actions("focus-open-", focus_dialog::SEND_TOOLBAR)
}

const FOLDED: [CommandId; 3] = focus_dialog::FOLDED;

type Handler = Rc<dyn Fn(CommandId)>;

type ReadHandler = Rc<dyn Fn(MessageId)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub index: u32,
    pub total: u32,
}

pub struct OpenMessage {
    client: Client,
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    title: gtk::Label,
    subtitle: gtk::Label,
    up_key: gtk::Box,
    down_key: gtk::Box,
    toolbar: Rc<ActionBar>,
    send_bar: Rc<ActionBar>,
    send_state: Cell<Option<postio_model::DraftState>>,
    /// A draft another client wrote: the received row stands without the verbs that answer a
    /// correspondent (#1212).
    foreign_draft: Cell<bool>,
    thread_chip: gtk::Box,
    kicker: gtk::Label,
    kicker_said: RefCell<String>,
    thread_bar: Rc<crate::thread_bar::ThreadBar>,
    banner: Rc<crate::thread_bar::OlderBanner>,
    new_reply: Rc<crate::thread_bar::OlderBanner>,
    match_banner: Rc<crate::thread_bar::MatchBanner>,
    subject: gtk::Label,
    labels: gtk::Box,
    clamp: adw::Clamp,
    more: gtk::Popover,
    more_items: gtk::Box,
    content: gtk::Box,
    slot: RefCell<Option<gtk::Box>>,
    /// Raised while the view moves between hosts: the dialog closing on the way out is a move, not
    /// the message closing.
    placing: Cell<bool>,
    parent: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    changed: RefCell<Option<Rc<dyn Fn()>>>,
    chip: RefCell<Option<ChipHandler>>,
    row: Cell<Option<MessageId>>,
    window: Cell<(i32, i32)>,
    following: Cell<bool>,
    row_width: Cell<Option<i32>>,
    this: RefCell<std::rc::Weak<OpenMessage>>,
    header_card: Rc<HeaderCard>,
    reader: Reader,
    fold_line: gtk::Button,
    fold_label: gtk::Label,
    keymap: RefCell<Keymap>,
    /// A count that moves with every open, so a body that lands for an earlier one is dropped.
    shown: Rc<Cell<Option<MessageId>>>,
    /// Every later draw of it (`O`, images allowed, a fold) marks the sentence where it stands and
    /// leaves the place the person's.
    revealed: Cell<Option<MessageId>>,
    inline: Inline,
    generation: Rc<Cell<u64>>,
    /// Raised when the message being prepared off the interface thread is no longer the one on
    /// screen: a worker still queued does nothing,
    /// so `j` held down prepares the message it lands on and not every one it passed.
    preparing: RefCell<Arc<AtomicBool>>,
    /// One reading at a time (#1609): `j` held over the pane reads the message it lands on; the
    /// latest asked for waits here.
    reading: Cell<bool>,
    waiting: Cell<Option<(MessageId, u64)>>,
    open: Rc<Cell<bool>>,
    handler: RefCell<Option<Handler>>,
    thread_total: Cell<usize>,
    at: Rc<Cell<usize>>,
    position: Cell<Position>,
    hit: Cell<Option<Position>>,
    accounts: RefCell<Vec<(postio_model::AccountId, String)>>,
    account_said: Rc<RefCell<Option<String>>>,
    line_said: Rc<RefCell<String>>,
    messages: Cell<u32>,
    parts: Rc<RefCell<Vec<Attachment>>>,
    marker: RefCell<Option<(MessageId, postio_model::listing::MarkerSummary)>>,
    dots: RefCell<Vec<(String, String)>>,
    card: RefCell<Option<gtk::Box>>,
    body: Rc<RefCell<Option<MessageBody>>>,
    capture: Cell<bool>,
    dwell: RefCell<Option<glib::SourceId>>,
    /// Refocusing the window does not start its clock again.
    settled: Cell<Option<MessageId>>,
    away: Cell<bool>,
    covered: Cell<bool>,
    read: RefCell<Option<ReadHandler>>,
}

impl OpenMessage {
    pub fn controls() -> Vec<CommandId> {
        toolbar()
            .iter()
            .map(|action| action.command)
            .chain([
                CommandId::CancelSend,
                CommandId::RetrySend,
                CommandId::MarkSent,
                CommandId::Back,
                CommandId::PrevMessage,
                CommandId::NextMessage,
                CommandId::DismissMarker,
                CommandId::ThreadMenu,
                CommandId::SwitchTreatment,
                CommandId::ShowImages,
                CommandId::AlwaysShowImages,
            ])
            .collect()
    }

    /// `runtime` is where a remote image is fetched; with none, nothing is ever fetched.
    pub fn new(
        client: Client,
        keymap: &Keymap,
        allowlist: &std::path::Path,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Rc<Self> {
        // The reader asks for inline (`cid:`) images synchronously while it lays the document out,
        // so the message's inline parts are
        // read before render and the reader is answered from them: never a blocking call on the
        // main thread (#1608).
        let shown: Rc<Cell<Option<MessageId>>> = Rc::default();
        let inline: Inline = Rc::default();
        let source = {
            let inline = Rc::clone(&inline);
            Rc::new(move |content_id: &str| inline.borrow().get(&cid_key(content_id)).cloned())
        };
        let reader = Reader::sharing(source, allowlist);
        reader.header().widget().set_visible(false);
        let header_card = Rc::new(HeaderCard::new());
        if let Some(runtime) = runtime {
            reader.set_remote_fetch(remote_fetch(runtime));
        }

        let close = postio_widgets::widgets::close_button();
        close.add_css_class("focus-open-close");

        let title = gtk::Label::new(None);
        title.add_css_class("focus-open-title");
        title.set_ellipsize(pango::EllipsizeMode::End);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("focus-open-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.set_valign(gtk::Align::Center);
        titles.append(&title);
        titles.append(&subtitle);

        // A cap inside a button is the button's shortcut (`a11y::teach_shortcuts`); a cap beside it
        // made four things of two and taught a screen reader nothing.
        let (up, up_key) = step("go-up-symbolic", "Previous message");
        let (down, down_key) = step("go-down-symbolic", "Next message");
        let steps = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        steps.add_css_class("linked");
        steps.add_css_class("focus-open-steps");
        steps.set_valign(gtk::Align::Center);
        steps.append(&up);
        steps.append(&down);

        let header = gtk::CenterBox::new();
        header.add_css_class("focus-open-header");
        header.set_start_widget(Some(&steps));
        header.set_center_widget(Some(&titles));
        header.set_end_widget(Some(&close));

        let toolbar = ActionBar::new(toolbar(), "focus-open-toolbar");
        if let Some(row) = toolbar.widget().downcast_ref::<gtk::Box>() {
            row.set_spacing(0);
        }
        tighten_keycaps(&toolbar.widget());
        let send_bar = ActionBar::new(send_toolbar(), "focus-open-send-toolbar");
        if let Some(row) = send_bar.widget().downcast_ref::<gtk::Box>() {
            row.set_spacing(0);
        }
        tighten_keycaps(&send_bar.widget());
        send_bar.set_visible(false);
        let more_items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        more_items.add_css_class("focus-row-menu");
        let more = gtk::Popover::builder()
            .child(&more_items)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        more.add_css_class("focus-row-menu-popover");
        more.add_css_class("focus-open-more-menu");
        // Opened from the keyboard, the menu draws one highlight, where the keyboard is: a pointer
        // resting under it highlights nothing until it moves.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let items = more_items.downgrade();
            move |_, _, _| {
                if let Some(items) = items.upgrade() {
                    items.remove_css_class("focus-keyboard-led");
                }
            }
        });
        more_items.add_controller(motion);
        if let Some(button) = toolbar.button(CommandId::MoreActions) {
            let button = button.widget();
            more.set_parent(&button);
            // A popover is its parent's to let go of, or GTK warns as the
            // button is finalized.
            let held = more.downgrade();
            button.connect_destroy(move |_| {
                if let Some(more) = held.upgrade() {
                    more.unparent();
                }
            });
        }

        // Each block carries the gap the handoff gives it (`rhythm`), so a block that is absent
        // takes its gap with it.
        let thread_chip = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::THREAD_GAP);
        thread_chip.add_css_class("focus-open-thread");
        thread_chip.set_margin_bottom(rhythm::MARKER_TO_SUBJECT);
        let kicker = gtk::Label::new(None);
        kicker.set_xalign(0.0);
        thread_chip.append(&kicker);
        thread_chip.set_visible(false);
        let thread_bar = crate::thread_bar::ThreadBar::new();
        let banner = crate::thread_bar::OlderBanner::new();
        let match_banner = crate::thread_bar::MatchBanner::new();
        match_banner
            .widget()
            .set_margin_bottom(rhythm::MARKER_TO_SUBJECT);
        let new_reply = crate::thread_bar::OlderBanner::with_class("focus-new-reply-banner");
        new_reply
            .widget()
            .set_margin_bottom(rhythm::MARKER_TO_SUBJECT);
        banner.widget().set_margin_bottom(rhythm::MARKER_TO_SUBJECT);
        let subject = gtk::Label::new(None);
        subject.add_css_class("focus-open-subject");
        subject.set_xalign(0.0);
        subject.set_wrap(true);
        subject.set_wrap_mode(pango::WrapMode::WordChar);
        let labels = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::LABEL_GAP);
        labels.add_css_class("focus-open-labels");
        labels.set_margin_top(rhythm::SUBJECT_TO_LABELS);
        let fold_label = gtk::Label::new(None);
        fold_label.set_xalign(0.0);
        let fold_line = gtk::Button::new();
        postio_widgets::widgets::button::style(&fold_line, Kind::Ghost, Size::Regular);
        fold_line.add_css_class("focus-open-fold-line");
        fold_line.set_child(Some(&fold_label));
        fold_line.set_halign(gtk::Align::Start);
        fold_line.set_visible(false);

        let header_widget = header_card.widget();
        header_widget.set_margin_top(rhythm::LABELS_TO_SENDER);
        let reader_widget = reader.widget();
        reader.view().set_margin_top(rhythm::CARD_TO_BODY);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("focus-open-column");
        column.set_margin_top(rhythm::TOP);
        column.set_margin_bottom(rhythm::BOTTOM);
        // Its own height, not the page's: the bottom padding sits under the last block, not at the
        // foot of a stretched column.
        column.set_valign(gtk::Align::Start);
        column.append(match_banner.widget());
        column.append(banner.widget());
        column.append(&thread_chip);
        column.append(&subject);
        column.append(&labels);
        column.append(&header_widget);
        column.append(&reader_widget);
        column.append(&fold_line);
        // One column for everything inside the message: width from the dialog's and the body's
        // treatment, centred, every block filling it.
        // Everything scrolls together, the body drawn in it rather than in a scroller of its own.
        let clamp = adw::Clamp::builder()
            .child(&column)
            .maximum_size(focus_dialog::column_width(
                focus_dialog::DIALOG_MAX,
                postio_body::treatment::Treatment::AppColours,
            ))
            .tightening_threshold(focus_dialog::column_width(
                focus_dialog::DIALOG_MAX,
                postio_body::treatment::Treatment::AppColours,
            ))
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        scroller.add_css_class("focus-open-scroller");
        reader.flow_in(&scroller);
        // The marker's sentence is revealed as the message opens, but never by scrolling the
        // subject and sender off (#2125).
        reader.view().keep_in_view_on_reveal(&subject);
        // The slot is hidden while it has nothing to say, so with no notice its gap goes too.
        reader.notice_slot().set_margin_top(rhythm::CARD_TO_NOTICE);
        reader.use_treatments();
        if let Some(line) = reader.render_mode_line() {
            let line = line.widget();
            line.set_margin_top(rhythm::CARD_TO_BODY);
            let view = reader.view().clone();
            line.connect_visible_notify(move |line| {
                view.set_margin_top(if line.is_visible() {
                    rhythm::MODE_LINE_TO_BODY
                } else {
                    rhythm::CARD_TO_BODY
                });
            });
        }

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-open");
        content.append(&header);
        content.append(&toolbar.widget());
        content.append(&send_bar.widget());
        content.append(thread_bar.widget());
        // Find sits above the column, not at its top, so opening it keeps the reading position.
        content.append(reader.find_bar().widget());
        // A reply that arrives while reading is said over the foot of the message: anything that
        // gave the text a different place or size would move what is being read.
        let overlay = gtk::Overlay::new();
        overlay.set_vexpand(true);
        overlay.set_child(Some(&scroller));
        let said = new_reply.widget();
        said.set_valign(gtk::Align::End);
        said.set_halign(gtk::Align::Fill);
        said.set_margin_top(0);
        said.set_margin_bottom(postio_widgets::widgets::space::S4);
        said.set_margin_start(postio_widgets::widgets::space::S4);
        said.set_margin_end(postio_widgets::widgets::space::S4);
        overlay.add_overlay(said);
        content.append(&overlay);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));
        let dialog = adw::Dialog::builder()
            .content_width(focus_dialog::dialog_width(WINDOW.0))
            .content_height(focus_dialog::dialog_height(WINDOW.1))
            .child(&toasts)
            .build();
        dialog.set_widget_name(DIALOG_NAME);
        // Space and Page Down read the message, and Return does not press a step button in the
        // header.
        dialog.set_focus(Some(&scroller));
        let open = Rc::new(Cell::new(false));

        let page = Rc::new(OpenMessage {
            client,
            dialog,
            toasts,
            title,
            subtitle,
            up_key,
            down_key,
            toolbar,
            send_bar,
            send_state: Cell::new(None),
            foreign_draft: Cell::new(false),
            thread_chip,
            kicker,
            kicker_said: RefCell::default(),
            thread_bar,
            banner,
            new_reply,
            match_banner,
            subject,
            labels,
            clamp,
            more,
            more_items,
            content,
            slot: RefCell::default(),
            placing: Cell::new(false),
            parent: RefCell::default(),
            changed: RefCell::default(),
            row: Cell::new(None),
            window: Cell::new(WINDOW),
            following: Cell::new(false),
            row_width: Cell::new(None),
            this: RefCell::default(),
            header_card,
            reader,
            fold_line,
            fold_label,
            keymap: RefCell::new(keymap.clone()),
            shown,
            revealed: Cell::new(None),
            inline,
            generation: Rc::default(),
            preparing: RefCell::default(),
            reading: Cell::new(false),
            waiting: Cell::new(None),
            open,
            handler: RefCell::default(),
            chip: RefCell::default(),
            thread_total: Cell::new(0),
            at: Rc::default(),
            accounts: RefCell::default(),
            account_said: Rc::default(),
            line_said: Rc::default(),
            position: Cell::new(Position { index: 0, total: 0 }),
            hit: Cell::new(None),
            messages: Cell::new(1),
            parts: Rc::default(),
            marker: RefCell::default(),
            dots: RefCell::default(),
            card: RefCell::default(),
            body: Rc::default(),
            capture: Cell::new(false),
            dwell: RefCell::default(),
            settled: Cell::new(None),
            away: Cell::new(false),
            covered: Cell::new(false),
            read: RefCell::default(),
        });

        let weak = Rc::downgrade(&page);
        page.this.replace(weak.clone());
        page.match_banner.connect(
            {
                let weak = weak.clone();
                move || {
                    if let Some(page) = weak.upgrade() {
                        page.run(CommandId::FindNext);
                    }
                }
            },
            {
                let weak = weak.clone();
                move || {
                    if let Some(page) = weak.upgrade() {
                        page.run(CommandId::Back);
                    }
                }
            },
        );
        page.new_reply.connect_press({
            let weak = weak.clone();
            move || {
                if let Some(page) = weak.upgrade() {
                    page.run(CommandId::LatestInConversation);
                }
            }
        });
        page.banner.connect_press({
            let weak = weak.clone();
            move || {
                if let Some(page) = weak.upgrade() {
                    page.run(CommandId::LatestInConversation);
                }
            }
        });
        page.reader.connect_attachment({
            let weak = weak.clone();
            move |node| {
                let Some(page) = weak.upgrade() else {
                    return;
                };
                let chip = page.chip.borrow().clone();
                if let (Some(chip), Some(part)) = (chip, node.attachment) {
                    chip(part);
                }
            }
        });
        // A dialog closed by its own ways out closes the message; one closed
        // on the view's way to the pane does not.
        page.dialog.connect_closed({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade()
                    && !page.placing.get()
                    && page.slot.borrow().is_none()
                    && page.open.replace(false)
                {
                    page.cancel_dwell();
                    page.tell_changed();
                }
            }
        });
        close.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.close();
                }
            }
        });
        for (button, command) in [(up, CommandId::PrevMessage), (down, CommandId::NextMessage)] {
            let weak = weak.clone();
            button.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.run(command);
                }
            });
        }
        for bar in [&page.toolbar, &page.send_bar] {
            let weak = weak.clone();
            bar.connect_command(move |command| {
                if let Some(page) = weak.upgrade() {
                    page.run(command.id());
                }
            });
        }
        page.fold_line.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open_first_fold();
                }
            }
        });
        // When the body view has the new snapshot, not when the render was asked for: the fold line
        // and highlight are read off it.
        page.reader.view().connect_rendered({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.show_fold_line();
                    page.highlight_marker();
                }
            }
        });
        page.reader.connect_treatment_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.fit_column();
                }
            }
        });
        page.set_keymap(keymap);
        page.fold_into_more(focus_dialog::folds_into_more(page.dialog.content_width()));
        page
    }

    pub fn place(&self, slot: Option<&gtk::Box>) {
        let here = self.slot.borrow().clone();
        match (slot, here) {
            (Some(slot), Some(here)) if *slot == here => return,
            (None, None) => return,
            (Some(slot), here) => {
                match &here {
                    None => {
                        if self.open.get() {
                            self.placing.set(true);
                            self.dialog.force_close();
                            self.placing.set(false);
                        }
                        self.dialog.set_child(None::<&gtk::Widget>);
                        self.toasts.set_child(None::<&gtk::Widget>);
                    }
                    Some(here) => here.remove(&self.content),
                }
                slot.append(&self.content);
                self.slot.replace(Some(slot.clone()));
            }
            (None, Some(here)) => {
                here.remove(&self.content);
                self.slot.replace(None);
                self.toasts.set_child(Some(&self.content));
                self.dialog.set_child(Some(&self.toasts));
                let parent = self
                    .parent
                    .borrow()
                    .as_ref()
                    .and_then(glib::WeakRef::upgrade);
                if self.open.get()
                    && let Some(parent) = parent
                {
                    self.dialog.present(Some(&parent));
                }
            }
        }
        let (width, height) = self.window.get();
        self.fit(width, height);
    }

    pub fn in_pane(&self) -> bool {
        self.slot.borrow().is_some()
    }

    pub fn view(&self) -> gtk::Widget {
        self.content.clone().upcast()
    }

    pub fn connect_changed(&self, changed: impl Fn() + 'static) {
        self.changed.replace(Some(Rc::new(changed)));
    }

    pub fn connect_chip(&self, chip: impl Fn(postio_model::ids::AttachmentId) + 'static) {
        self.chip.replace(Some(Rc::new(chip)));
    }

    pub fn part_node(
        &self,
        id: postio_model::ids::AttachmentId,
    ) -> Option<postio_ui::reader::parts::Node> {
        self.part_nodes()
            .into_iter()
            .find(|node| node.attachment == Some(id))
    }

    pub fn part_nodes(&self) -> Vec<postio_ui::reader::parts::Node> {
        postio_ui::reader::parts::tree("multipart/mixed", &self.parts.borrow())
            .into_iter()
            .filter(|node| node.is_leaf() && node.attachment.is_some() && node.filename.is_some())
            .collect()
    }

    pub fn attachment_chips(&self) -> Vec<gtk::Widget> {
        let mut found = Vec::new();
        let mut stack = vec![self.reader.widget()];
        while let Some(widget) = stack.pop() {
            if widget.has_css_class("postio-attachment") && widget.is::<gtk::Button>() {
                found.push(widget.clone());
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        found
    }

    fn tell_changed(&self) {
        let changed = self.changed.borrow().clone();
        if let Some(changed) = changed {
            changed();
        }
    }

    pub fn showing_row(&self) -> Option<MessageId> {
        self.open.get().then(|| self.row.get()).flatten()
    }

    fn present(&self, parent: &gtk::Widget) {
        if self.open.get() {
            return;
        }
        self.parent.replace(Some(parent.downgrade()));
        self.follow(parent);
        if !self.in_pane() {
            self.dialog.present(Some(parent));
        }
        self.open.set(true);
        self.tell_changed();
    }

    fn host_width(&self) -> i32 {
        let window = self.window.get().0;
        let dialog = focus_dialog::dialog_width(window);
        if self.in_pane() {
            focus_dialog::pane_width(window).unwrap_or(dialog)
        } else {
            dialog
        }
    }

    /// Its size comes from the window and nothing else, so stepping through the list never resizes
    /// it.
    fn fit(&self, width: i32, height: i32) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.window.set((width, height));
        let dialog = focus_dialog::dialog_width(width);
        if self.dialog.content_width() != dialog {
            self.dialog.set_content_width(dialog);
        }
        let tall = focus_dialog::dialog_height(height);
        if self.dialog.content_height() != tall {
            self.dialog.set_content_height(tall);
        }
        self.fit_column();
        let host = self.host_width();
        self.fold_into_more(focus_dialog::folds_into_more(host) || self.full_row_width() > host);
    }

    /// The handoff's 760px rule assumes its narrower face, so a row that would not fit folds too,
    /// rather than widening the dialog
    /// past its window's rule. Measured once on screen and kept: only its words and keys change it.
    fn full_row_width(&self) -> i32 {
        if let Some(width) = self.row_width.get() {
            return width;
        }
        let folded: Vec<gtk::Widget> = FOLDED
            .iter()
            .filter_map(|command| self.toolbar.button(*command))
            .map(|button| button.widget())
            .filter(|widget| !widget.is_visible())
            .collect();
        let more = self
            .toolbar
            .button(CommandId::MoreActions)
            .map(|button| button.widget())
            .filter(gtk::Widget::is_visible);
        for widget in &folded {
            widget.set_visible(true);
        }
        if let Some(more) = &more {
            more.set_visible(false);
        }
        let (_, natural, _, _) = self
            .toolbar
            .widget()
            .measure(gtk::Orientation::Horizontal, -1);
        for widget in &folded {
            widget.set_visible(false);
        }
        if let Some(more) = &more {
            more.set_visible(true);
        }
        // Kept only once the row is on screen, dressed by its stylesheet.
        if self.toolbar.widget().is_mapped() {
            self.row_width.set(Some(natural));
        }
        natural
    }

    fn fold_into_more(&self, fold: bool) {
        for command in FOLDED {
            if let Some(button) = self.toolbar.button(command) {
                button.widget().set_visible(!fold);
            }
        }
        if let Some(button) = self.toolbar.button(CommandId::MoreActions) {
            button.widget().set_visible(fold);
        }
        if !fold {
            self.more.popdown();
        }
    }

    pub fn folded(&self) -> bool {
        self.toolbar
            .button(CommandId::MoreActions)
            .is_some_and(|button| button.widget().is_visible())
    }

    pub fn show_more(&self) {
        if !self.folded() {
            return;
        }
        while let Some(child) = self.more_items.first_child() {
            self.more_items.remove(&child);
        }
        let keymap = self.keymap.borrow().clone();
        for command in FOLDED {
            let Some(action) = toolbar().iter().find(|action| action.command == command) else {
                continue;
            };
            let item = gtk::Button::new();
            postio_widgets::widgets::button::style(&item, Kind::Ghost, Size::Regular);
            item.add_css_class("focus-row-menu-item");
            item.add_css_class("focus-open-more-item");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::KEYCAP_GAP);
            let words = gtk::Label::new(Some(action.label));
            words.set_xalign(0.0);
            words.set_hexpand(true);
            row.append(&words);
            if let Some(key) = hints::key(&keymap, command) {
                row.append(&keyhint::cap(&hints::short(&key)));
            }
            item.set_child(Some(&row));
            item.update_property(&[gtk::accessible::Property::Label(action.label)]);
            let weak = self.this.borrow().clone();
            item.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.more.popdown();
                    page.run(command);
                }
            });
            self.more_items.append(&item);
        }
        self.more_items.add_css_class("focus-keyboard-led");
        self.more.popup();
        if let Some(first) = self.more_items.first_child() {
            first.grab_focus();
        }
    }

    pub fn more_open(&self) -> bool {
        self.more.is_visible()
    }

    pub fn toast_overlay(&self) -> adw::ToastOverlay {
        self.toasts.clone()
    }

    pub fn close_more(&self) {
        self.more.popdown();
        if let Some(button) = self.toolbar.button(CommandId::MoreActions) {
            button.widget().grab_focus();
        }
    }

    /// `None` while the message is in the reading pane, whose list is no obstacle.
    pub fn picker_anchor(&self) -> Option<(gtk::Widget, gtk::gdk::Rectangle)> {
        if self.in_pane() || !self.dialog.is_mapped() {
            return None;
        }
        let child = self.dialog.child()?;
        let bar = self.toolbar.widget();
        let below = bar
            .compute_bounds(&child)
            .map_or(100, |bounds| (bounds.y() + bounds.height()).ceil() as i32);
        Some((child, gtk::gdk::Rectangle::new(56, below, 1, 1)))
    }

    pub fn message_has(&self, widget: &gtk::Widget) -> bool {
        self.reader
            .view()
            .ancestor(gtk::ScrolledWindow::static_type())
            .is_some_and(|scroller| *widget == scroller || widget.is_ancestor(&scroller))
    }

    /// Also where the keyboard returns after a composer closes over the message.
    pub fn focus_message(&self) {
        if self.in_pane() {
            return;
        }
        if let Some(scroller) = self
            .reader
            .view()
            .ancestor(gtk::ScrolledWindow::static_type())
        {
            self.dialog.set_focus(Some(&scroller));
        }
    }

    pub fn more_item_with_focus(&self) -> Option<String> {
        if !self.more.is_visible() {
            return None;
        }
        let mut child = self.more_items.first_child();
        while let Some(item) = child {
            if item.has_focus() {
                return item
                    .first_child()
                    .and_then(|row| row.first_child())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.text().to_string());
            }
            child = item.next_sibling();
        }
        None
    }

    pub fn more_menu(&self) -> gtk::Popover {
        self.more.clone()
    }

    fn fit_column(&self) {
        let column = focus_dialog::column_width(self.host_width(), self.reader.treatment());
        if self.clamp.maximum_size() != column {
            self.clamp.set_maximum_size(column);
            self.clamp.set_tightening_threshold(column);
        }
    }

    /// The window's own size is read on each layout: the surface's includes the shadow a restored
    /// window draws around itself.
    fn follow(&self, parent: &gtk::Widget) {
        let Some(window) = parent.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let (width, height) = (window.width(), window.height());
        if width > 0 && height > 0 {
            self.fit(width, height);
        } else {
            let (width, height) = window.default_size();
            self.fit(width, height);
        }
        if self.following.get() {
            return;
        }
        let Some(surface) = window.surface() else {
            return;
        };
        self.following.set(true);
        let page = self.this.borrow().clone();
        let window = window.downgrade();
        surface.connect_layout(move |_, _, _| {
            if let (Some(page), Some(window)) = (page.upgrade(), window.upgrade()) {
                page.fit(window.width(), window.height());
            }
        });
    }

    pub fn set_capture(&self, capture: bool) {
        self.capture.set(capture);
    }

    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    fn run(&self, command: CommandId) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(command);
        }
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.toolbar.set_keymap(keymap);
        for action in toolbar() {
            if let Some(button) = self.toolbar.button(action.command) {
                let key = hints::key(keymap, action.command).map(|key| hints::short(&key));
                button.set_key(key.as_deref());
            }
        }
        self.send_bar.set_keymap(keymap);
        for action in send_toolbar() {
            if let Some(button) = self.send_bar.button(action.command) {
                let key = hints::key(keymap, action.command).map(|key| hints::short(&key));
                button.set_key(key.as_deref());
            }
        }
        self.reader.set_keymap(keymap);
        // Its keys are part of its width.
        self.row_width.set(None);
        for (holder, command) in [
            (&self.up_key, CommandId::PrevMessage),
            (&self.down_key, CommandId::NextMessage),
        ] {
            while let Some(child) = holder.first_child() {
                holder.remove(&child);
            }
            if let Some(key) = hints::key(keymap, command) {
                holder.append(&keyhint::cap(&key));
            }
        }
        self.show_fold_line();
    }

    pub fn show(&self, parent: &impl IsA<gtk::Widget>, row: &FocusRow, position: Position) {
        let Some(conversation) = row.as_conversation() else {
            return;
        };
        let summary = &conversation.summary;
        let message = summary.representative.id;
        let subject = summary
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_owned());
        self.title.set_text(&subject);
        self.subject.set_text(&subject);
        self.position.set(position);
        self.hit.set(None);
        self.messages.set(summary.message_count.max(1));
        self.clear_thread();
        self.set_send_state(summary.representative.send_state);
        self.show_position(true);
        self.show_labels(&conversation.labels);
        self.marker
            .replace(summary.marker.clone().map(|marker| (message, marker)));
        self.row.set(Some(row.id()));

        self.present(parent.upcast_ref());
        self.show_message(message);
    }

    /// `j` and `k` are the controller's: a hit's step is to the next hit.
    pub fn show_found(
        &self,
        parent: &impl IsA<gtk::Widget>,
        message: MessageId,
        subject: &str,
        hit: Option<Position>,
    ) {
        self.hit.set(hit);
        self.begin_hit(message, subject);
        self.present(parent.upcast_ref());
        self.show_message(message);
        self.read_hit_thread(message);
    }

    pub fn show_in_place(&self, message: MessageId) {
        self.show_message(message);
    }

    fn begin_hit(&self, message: MessageId, subject: &str) {
        let _ = message;
        self.title.set_text(subject);
        self.subject.set_text(subject);
        self.messages.set(1);
        self.clear_thread();
        self.show_position(true);
        self.show_labels(&[]);
        self.marker.replace(None);
        self.row.set(None);
        self.set_send_state(None);
    }

    fn read_hit_thread(&self, message: MessageId) {
        use postio_model::listing::MailStore as _;
        let client = self.client.clone();
        let generation = Rc::clone(&self.generation);
        let asked = generation.get();
        let weak = self.this.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let Ok(rows) = client.message_rows(vec![message]).await else {
                return;
            };
            let Some(row) = rows.into_iter().next() else {
                return;
            };
            if generation.get() != asked {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            // The results view's rows are no list's, so a result opened from there is titled by the
            // message itself.
            if page.subject.text().is_empty()
                && let Some(subject) = row.subject.as_deref()
            {
                page.title.set_text(subject);
                page.subject.set_text(subject);
            }
            page.messages.set(row.thread_count.max(1));
            page.show_position(true);
        });
    }

    fn show_message(&self, message: MessageId) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let preparing = std::mem::take(&mut *self.preparing.borrow_mut());
        preparing.store(true, Ordering::Relaxed);
        self.shown.set(Some(message));
        self.set_foreign_draft(false);
        self.revealed.set(None);
        self.body.replace(None);
        self.show_marker_card(message);
        self.header_card.clear();
        self.account_said.replace(None);
        self.draw_place_line();
        self.fold_line.set_visible(false);
        self.reader
            .show_absent(postio_ui::reader::document::Absent::Partial);
        self.reader.set_unsubscribe(None);
        // The view is put at the top now, and the new document's first snapshot starts there too.
        self.reader.view().scroll_to_edge(false);
        self.settled.set(None);
        self.arm_dwell(message);
        self.load(message, generation);
    }

    /// Only the open message is read, never the list. Same generation, nothing cleared first and
    /// the view left where it is,
    /// so "Downloading this message" goes straight to the body (#1788).
    pub fn body_landed(&self, message: MessageId) {
        if !self.is_open() || self.shown.get() != Some(message) || self.body.borrow().is_some() {
            return;
        }
        self.load(message, self.generation.get());
    }

    fn read_landed(&self) {
        self.reading.set(false);
        if let Some((message, generation)) = self.waiting.take()
            && generation == self.generation.get()
        {
            self.load(message, generation);
        }
    }

    fn set_send_state(&self, state: Option<postio_model::DraftState>) {
        self.send_state.set(state);
        let verbs = focus_dialog::send_verbs(state);
        self.toolbar.set_visible(verbs.is_none());
        self.send_bar.set_visible(verbs.is_some());
        for action in send_toolbar() {
            if let Some(button) = self.send_bar.button(action.command) {
                button
                    .widget()
                    .set_visible(verbs.is_some_and(|verbs| verbs.contains(&action.command)));
            }
        }
    }

    /// Reply, Reply all and Forward answer a correspondent a draft does not have.
    fn set_foreign_draft(&self, foreign: bool) {
        self.foreign_draft.set(foreign);
        for action in toolbar() {
            if let Some(button) = self.toolbar.button(action.command) {
                button
                    .widget()
                    .set_visible(focus_dialog::received_offers(action.command, foreign));
            }
        }
    }

    pub fn foreign_draft(&self) -> bool {
        self.foreign_draft.get()
    }

    pub fn send_state(&self) -> Option<postio_model::DraftState> {
        self.send_state.get()
    }

    pub fn connect_read(&self, read: impl Fn(MessageId) + 'static) {
        self.read.replace(Some(Rc::new(read)));
    }

    fn arm_dwell(&self, message: MessageId) {
        self.stop_dwell();
        if !self.open.get() || self.away.get() || self.covered.get() {
            tracing::debug!(
                message = message.get(),
                open = self.open.get(),
                away = self.away.get(),
                covered = self.covered.get(),
                "the read clock did not start"
            );
            return;
        }
        let postio_ui::dwell::Arm::Start { after, .. } =
            postio_ui::dwell::on_cursor(Some(message.get()))
        else {
            return;
        };
        let page = self.this.borrow().clone();
        let source = glib::timeout_add_local_once(after, move || {
            let Some(page) = page.upgrade() else {
                return;
            };
            // Fired: the id is spent, and removing it again would warn.
            let _ = page.dwell.borrow_mut().take();
            page.settled.set(Some(message));
            if !page.open.get() || page.shown.get() != Some(message) {
                tracing::debug!(
                    message = message.get(),
                    "the read clock ran out on a message no longer shown"
                );
                return;
            }
            tracing::debug!(
                message = message.get(),
                "the read clock ran out: marking read"
            );
            let read = page.read.borrow().clone();
            if let Some(read) = read {
                read(message);
            }
        });
        self.dwell.replace(Some(source));
    }

    /// The clock must not overrule the person's own read state.
    pub fn cancel_dwell(&self) {
        self.settled.set(self.shown.get());
        self.stop_dwell();
    }

    /// A message left open in a window that is not the active one is not being read: the clock
    /// stops, and restarts from the top
    /// when the window is back, unless it has already finished with that message.
    pub fn set_window_active(&self, active: bool) {
        tracing::debug!(active, "the window's focus changed under an open message");
        self.away.set(!active);
        if !active {
            self.stop_dwell();
        } else if self.open.get()
            && let Some(message) = self.shown.get()
            && self.settled.get() != Some(message)
        {
            self.arm_dwell(message);
        }
    }

    /// A message not on screen is not being read: the clock stops, and restarts from the top when
    /// it is back, if not already read.
    pub fn set_covered(&self, covered: bool) {
        self.covered.set(covered);
        if covered {
            self.stop_dwell();
        } else if self.open.get()
            && let Some(message) = self.shown.get()
            && self.settled.get() != Some(message)
        {
            self.arm_dwell(message);
        }
    }

    fn stop_dwell(&self) {
        if let Some(source) = self.dwell.borrow_mut().take() {
            source.remove();
        }
    }

    fn clear_thread(&self) {
        self.thread_total.set(0);
        self.at.set(0);
        self.thread_bar.clear();
        self.thread_chip.set_visible(false);
        self.kicker.set_text("");
        self.kicker_said.replace(String::new());
        self.banner.set(None);
        self.new_reply.set(None);
        self.match_banner.set(None);
        self.header_card.set_reply_to(None);
    }

    pub fn set_thread_bar(&self, view: &postio_ui::thread_bar::ThreadBarView) {
        self.thread_bar.set(view);
        self.thread_total.set(
            view.count_line
                .split_whitespace()
                .next()
                .and_then(|count| count.parse().ok())
                .unwrap_or(0),
        );
        if let Some(at) = self.thread_bar.current() {
            self.at.set(at as usize);
        }
        self.show_position(false);
    }

    pub fn set_kicker(&self, kicker: &str) {
        self.kicker.set_text(kicker);
        self.kicker_said.replace(kicker.to_owned());
        self.thread_chip.set_visible(!kicker.is_empty());
        self.show_position(false);
    }

    pub fn set_older_banner(&self, banner: Option<&postio_ui::thread_bar::BannerView>) {
        self.banner.set(banner);
    }

    pub fn set_new_reply(&self, banner: Option<&postio_ui::thread_bar::BannerView>) {
        self.new_reply.set(banner);
    }

    pub fn set_match_banner(&self, banner: Option<&postio_ui::thread_bar::MatchBannerView>) {
        self.match_banner.set(banner);
        self.show_position(false);
    }

    pub fn match_banner_said(&self) -> Option<String> {
        self.match_banner.said()
    }

    pub fn matched_chips(&self) -> Vec<u32> {
        self.thread_bar.matched()
    }

    pub fn new_reply_up(&self) -> bool {
        self.new_reply.said().is_some()
    }

    pub fn thread_button(&self) -> gtk::Button {
        self.thread_bar.menu_button().clone()
    }

    pub fn connect_thread_menu(&self, handler: impl Fn() + 'static) {
        self.thread_bar.connect_menu(handler);
    }

    pub fn connect_thread_chip(&self, handler: impl Fn(crate::thread_bar::ChipClick) + 'static) {
        self.thread_bar.connect_chip(handler);
    }

    pub fn set_reply_to(&self, reply_to: Option<&str>) {
        self.header_card.set_reply_to(reply_to);
    }

    pub fn connect_bar_room(&self, handler: impl Fn(u32) + 'static) {
        self.thread_bar.connect_room(handler);
    }

    pub fn step_thread(&self, by: isize) {
        self.run(if by < 0 {
            CommandId::PrevInConversation
        } else {
            CommandId::NextInConversation
        });
    }

    pub fn row(&self) -> Option<MessageId> {
        self.row.get()
    }

    pub fn thread_bar_state(&self) -> (Option<u32>, usize) {
        (self.thread_bar.current(), self.thread_bar.chip_count())
    }

    pub fn banner_said(&self) -> Option<String> {
        self.banner.said()
    }

    pub fn reply_to_said(&self) -> Option<String> {
        self.header_card.reply_to()
    }

    fn show_position(&self, latest: bool) {
        let position = self.position.get();
        let messages = self.messages.get();
        let thread_len = self.thread_total.get();
        let latest = latest || thread_len == 0 || self.at.get() + 1 == thread_len;
        let kicker = self.kicker_said.borrow().clone();
        let from_results = self.match_banner.said().is_some();
        let said = if !kicker.is_empty() && (self.hit.get().is_none() || from_results) {
            // The header says where the message is in the thread, as the kicker does, never where
            // its row is in the inbox.
            postio_ui::thread_bar::place(self.at.get(), thread_len)
        } else if let Some(hit) = self.hit.get() {
            focus_dialog::result_line(
                hit.index as usize,
                hit.total as usize,
                messages,
                self.at.get(),
                latest,
            )
        } else if position.total == 0 {
            "Found by search".to_owned()
        } else {
            focus_dialog::position_line(
                position.index as usize,
                position.total as usize,
                messages,
                self.at.get(),
                latest,
                self.send_state.get(),
            )
        };
        self.line_said.replace(said);
        self.draw_place_line();
    }

    pub fn set_accounts(&self, accounts: Vec<(postio_model::AccountId, String)>) {
        let several = accounts.len() > 1;
        self.accounts
            .replace(if several { accounts } else { Vec::new() });
        self.account_said.replace(None);
        self.draw_place_line();
    }

    fn draw_place_line(&self) {
        let line = self.line_said.borrow();
        let account = self.account_said.borrow();
        self.subtitle
            .set_text(&focus_dialog::with_account(&line, account.as_deref()));
    }

    /// While a reading is on its way, the ask waits, and only the latest.
    fn load(&self, message: MessageId, generation: u64) {
        if self.reading.replace(true) {
            self.waiting.set(Some((message, generation)));
            return;
        }
        let client = self.client.clone();
        let reader = self.reader.clone();
        let current = Rc::clone(&self.generation);
        let shown_parts = Rc::clone(&self.parts);
        let shown_body = Rc::clone(&self.body);
        let inline = Rc::clone(&self.inline);
        let header_card = Rc::clone(&self.header_card);
        let accounts = self.accounts.borrow().clone();
        let account_said = Rc::clone(&self.account_said);
        let line_said = Rc::clone(&self.line_said);
        let subtitle = self.subtitle.clone();
        let preparing = Arc::clone(&self.preparing.borrow());
        let page = self.this.borrow().clone();
        shown_parts.borrow_mut().clear();
        inline.borrow_mut().clear();
        glib::spawn_future_local(async move {
            if current.get() != generation {
                if let Some(page) = page.upgrade() {
                    page.read_landed();
                }
                return;
            }
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.readings(vec![message], false).await;
            if let Some(page) = page.upgrade() {
                page.read_landed();
            }
            if current.get() != generation {
                return;
            }
            let Some(reading) = read.ok().and_then(|mut readings| readings.pop()) else {
                reader.show_absent(postio_ui::reader::document::Absent::Missing);
                return;
            };
            if let Some(row) = reading.row.as_deref() {
                let account = accounts
                    .iter()
                    .find(|(id, _)| *id == row.account_id)
                    .map(|(_, address)| address.clone());
                subtitle.set_text(&focus_dialog::with_account(
                    &line_said.borrow(),
                    account.as_deref(),
                ));
                account_said.replace(account);
                header_card.set(
                    &row.from,
                    &row.to,
                    &row.cc,
                    row.date.unwrap_or(row.received_at),
                );
                reader.set_message_header(
                    &row.from,
                    &row.to,
                    &row.cc,
                    row.subject.as_deref(),
                    row.date.unwrap_or(row.received_at),
                );
                // A message is read once, and its warning rides the same reading as the header it
                // belongs under (#1928).
                header_card.set_warning(reading.sender_warning.as_deref());
                reader.set_sender_warning(reading.sender_warning.as_deref());
            }
            let sender = reading
                .row
                .as_deref()
                .and_then(|row| row.from.first())
                .map(|from| from.address.clone());
            let parts: Vec<Attachment> = reading
                .row
                .as_deref()
                .map(|row| row.attachments.clone())
                .unwrap_or_default();
            shown_parts.replace(parts.clone());
            match reading.body {
                Body::Ready {
                    body,
                    encoding_problems,
                } => {
                    let content_type = reading
                        .row
                        .as_deref()
                        .and_then(|row| row.content_type.clone());
                    reader.set_attachments(
                        &root_type(content_type.as_deref(), &body, &parts),
                        &parts,
                    );
                    for content_id in parts.iter().filter_map(|part| part.content_id.clone()) {
                        let asked = content_id
                            .trim()
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .to_owned();
                        // POSTIO-GLIB-SAFE: as the reading's.
                        let read = client.inline_part(message, asked).await;
                        if current.get() != generation {
                            return;
                        }
                        if let Ok(Some(found)) = read {
                            inline.borrow_mut().insert(cid_key(&content_id), found);
                        }
                    }
                    // Sanitised, classified and treated on a worker, under the policy this sender
                    // is drawn with. A message nobody waits for is not prepared.
                    // Preparation is before the render deadline, not inside it: a slow sanitise
                    // delays the draw without tipping it into the plain-text fallback.
                    let remote = reader.remote_images_for(sender.as_deref());
                    let prepared = {
                        let body = body.clone();
                        let preparing = Arc::clone(&preparing);
                        gtk::gio::spawn_blocking(move || {
                            (!preparing.load(Ordering::Relaxed)).then(|| {
                                postio_ui::reader::document::prepare_treated(&body, remote)
                            })
                        })
                        // POSTIO-GLIB-SAFE: gio's own thread pool answers.
                        .await
                        .ok()
                        .flatten()
                    };
                    if current.get() != generation {
                        return;
                    }
                    shown_body.replace(Some(body.clone()));
                    reader.render_prepared(&body, sender.as_deref(), prepared);
                    reader.set_encoding_problems(encoding_problems);
                    // After `render_prepared`, which clears it. The band is for list mail only
                    // (`List-Id` or `List-Unsubscribe`), while `M` is not (`FocusWindow::act`).
                    let offer = reading.row.as_deref().and_then(|row| {
                        postio_ui::unsubscribe::banner(
                            reading.send_state,
                            row.list_id.as_deref(),
                            row.promoted.map(|said| said.unsubscribe_offered),
                            &row.from,
                        )
                    });
                    reader.set_unsubscribe(offer.as_ref().map(|o| o.list_identifier.as_str()));
                }
                Body::Partial => reader.show_absent(postio_ui::reader::document::Absent::Partial),
                Body::Offline => reader.show_absent(postio_ui::reader::document::Absent::Offline),
                Body::Missing => reader.show_absent(postio_ui::reader::document::Absent::Missing),
                Body::Empty => reader.show_absent(postio_ui::reader::document::Absent::Empty),
                Body::Failed => reader.show_absent(postio_ui::reader::document::Absent::Failed),
                Body::ForeignDraft => {
                    reader.show_absent(postio_ui::reader::document::Absent::ForeignDraft);
                    if let Some(page) = page.upgrade() {
                        page.set_foreign_draft(true);
                    }
                }
            }
        });
    }

    pub fn thread_chip_said(&self) -> String {
        self.kicker.text().to_string()
    }

    fn show_labels(&self, labels: &[postio_model::Label]) {
        while let Some(child) = self.labels.first_child() {
            self.labels.remove(&child);
        }
        let mut dots = Vec::new();
        for label in labels {
            let colour = crate::places::label_rgb(label);
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::LABEL_GAP);
            pill.add_css_class("focus-open-label-pill");
            let dot = crate::places::colour_dot(colour);
            dot.add_css_class("focus-open-label-dot");
            dot.set_content_width(focus_dialog::LABEL_DOT);
            pill.append(&dot);
            pill.append(&gtk::Label::new(Some(&label.name)));
            self.labels.append(&pill);
            dots.push((label.name.clone(), colour.to_hex()));
        }
        self.dots.replace(dots);
        let add = gtk::Button::new();
        postio_widgets::widgets::button::style(&add, Kind::Ghost, Size::Regular);
        add.add_css_class("focus-open-add-label");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::KEYCAP_GAP);
        row.append(&gtk::Label::new(Some("+ Label")));
        if let Some(key) = hints::key(&self.keymap.borrow(), CommandId::AddLabel) {
            row.append(&keyhint::cap(&key));
        }
        add.set_child(Some(&row));
        let handler = self.handler.borrow().clone();
        add.connect_clicked(move |_| {
            if let Some(handler) = &handler {
                handler(CommandId::AddLabel);
            }
        });
        self.labels.append(&add);
    }

    fn show_fold_line(&self) {
        let folds = self.reader.view().folds();
        let Some((_, words, _)) = folds.iter().find(|(_, _, open)| !open) else {
            self.fold_line.set_visible(false);
            return;
        };
        let mut said = format!("\u{203a} {words} folded");
        if let Some(key) = hints::key(&self.keymap.borrow(), CommandId::ViewSource) {
            said.push_str(&format!(" \u{b7} {key} shows the raw source"));
        }
        self.fold_label.set_text(&said);
        self.fold_line.set_visible(true);
    }

    fn open_first_fold(&self) {
        let folds = self.reader.view().folds();
        if let Some((id, _, _)) = folds.iter().find(|(_, _, open)| !open) {
            self.reader.view().open_fold(id);
        }
    }

    pub fn close(&self) {
        self.cancel_dwell();
        if !self.in_pane() {
            self.dialog.close();
        }
        if self.open.replace(false) {
            self.tell_changed();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    pub fn dialog(&self) -> adw::Dialog {
        self.dialog.clone()
    }

    pub fn title(&self) -> String {
        self.title.text().to_string()
    }

    pub fn subtitle(&self) -> String {
        self.subtitle.text().to_string()
    }

    pub fn choices(&self) -> Vec<crate::chooser::Choice> {
        use crate::chooser::Choice;
        let mut choices: Vec<Choice> = Vec::new();
        if let Some(document) = self.reader.view().document() {
            for link in &document.links {
                let postio_render::LinkTarget::External(url) = &link.target else {
                    continue;
                };
                let target = url.to_string();
                if choices
                    .iter()
                    .any(|choice| matches!(choice, Choice::Link { target: t, .. } if *t == target))
                {
                    continue;
                }
                let words = words_in(&document, link.rect);
                choices.push(Choice::Link {
                    words: if words.is_empty() {
                        target.clone()
                    } else {
                        words
                    },
                    target,
                });
            }
        }
        for part in self.parts.borrow().iter() {
            let Some(name) = part.filename.clone() else {
                continue;
            };
            choices.push(Choice::Part {
                name,
                size: postio_ui::format::human_size(part.size),
                id: part.id,
            });
        }
        choices
    }

    pub fn dismiss_marker(&self) -> bool {
        let Some(card) = self.card.borrow().clone() else {
            return false;
        };
        let mut stack = vec![card.upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if widget.has_css_class("focus-marker-dismiss")
                && let Some(button) = widget.downcast_ref::<gtk::Button>()
            {
                button.emit_clicked();
                return true;
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
        }
        false
    }

    pub fn marker_card_said(&self) -> Vec<String> {
        let Some(card) = self.card.borrow().clone() else {
            return Vec::new();
        };
        let mut said = Vec::new();
        let mut stack = vec![card.upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        said
    }

    fn show_marker_card(&self, message: MessageId) {
        let card = self
            .marker
            .borrow()
            .as_ref()
            .filter(|(marked, _)| *marked == message)
            .map(|(_, marker)| self.marker_card(marker));
        self.reader
            .set_under_header(card.as_ref().map(|card| card.upcast_ref::<gtk::Widget>()));
        self.card.replace(card);
    }

    fn marker_card(&self, marker: &postio_model::listing::MarkerSummary) -> gtk::Box {
        let line = postio_ui::focus_row::marker_line(
            marker,
            postio_ui::clock::now().to_utc(),
            &chrono::Local,
        )
        .capturing(self.capture.get());
        let card = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        card.add_css_class("focus-marker-card");
        card.set_margin_top(rhythm::SENDER_TO_CARD);
        let chip = gtk::Label::new(Some(line.chip));
        chip.add_css_class("focus-marker-chip");
        chip.set_valign(gtk::Align::Center);
        card.append(&chip);
        if let Some(date) = &line.date {
            let date = gtk::Label::new(Some(date));
            date.add_css_class("focus-marker-date");
            card.append(&date);
        }
        if let Some(quote) = &line.quote {
            let quote = gtk::Label::new(Some(&format!("\u{201c}{quote}\u{201d}")));
            quote.add_css_class("focus-marker-quote");
            // The whole sentence wraps onto a second line: cut short, it no longer says what was
            // asked.
            quote.set_wrap(true);
            quote.set_wrap_mode(pango::WrapMode::WordChar);
            quote.set_width_chars(1);
            quote.set_xalign(0.0);
            quote.set_hexpand(true);
            card.append(&quote);
        } else {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            card.append(&spacer);
        }
        if let Some(status) = line.status {
            let status = gtk::Label::new(Some(status));
            status.add_css_class("dim-label");
            card.append(&status);
        }
        let keymap = self.keymap.borrow().clone();
        for (command, words) in line.actions {
            let button = gtk::Button::new();
            postio_widgets::widgets::button::style(&button, Kind::Secondary, Size::Regular);
            button.add_css_class("focus-marker-action");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::KEYCAP_GAP);
            row.append(&gtk::Label::new(Some(words)));
            if let Some(key) = hints::key(&keymap, command) {
                row.append(&keyhint::cap(&key));
            }
            button.set_child(Some(&row));
            let handler = self.handler.borrow().clone();
            button.connect_clicked(move |_| {
                if let Some(handler) = &handler {
                    handler(command);
                }
            });
            card.append(&button);
        }
        let dismiss = gtk::Button::new();
        postio_widgets::widgets::button::style(&dismiss, Kind::Ghost, Size::Regular);
        dismiss.add_css_class("focus-marker-dismiss");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::KEYCAP_GAP);
        row.append(&gtk::Label::new(Some("Dismiss")));
        if let Some(key) = hints::key(&keymap, CommandId::DismissMarker) {
            row.append(&keyhint::cap(&key));
        }
        dismiss.set_child(Some(&row));
        let handler = self.handler.borrow().clone();
        dismiss.connect_clicked(move |_| {
            if let Some(handler) = &handler {
                handler(CommandId::DismissMarker);
            }
        });
        card.append(&dismiss);
        card
    }

    pub fn clear_marker(&self) {
        self.marker.replace(None);
        if let Some(shown) = self.shown.get() {
            self.show_marker_card(shown);
        }
        self.reader.view().set_highlight(None);
    }

    /// Found by its words through `TextIndex::locate`, whose offset is chars into the body's own
    /// text, the text the detector read.
    fn highlight_marker(&self) {
        let Some(shown) = self.shown.get() else {
            return;
        };
        let excerpt = self
            .marker
            .borrow()
            .as_ref()
            .filter(|(marked, _)| *marked == shown)
            .and_then(|(_, marker)| marker.excerpt.clone());
        let (Some(excerpt), Some(body), Some(document)) = (
            excerpt,
            self.body.borrow().clone(),
            self.reader.view().document(),
        ) else {
            return;
        };
        let own = postio_body::own_text(&body);
        // The first place its words stand: the marker keeps its span, the listing does not carry
        // it.
        let offset = own.find(&excerpt).map_or(0, |at| own[..at].chars().count());
        let range = document.text.locate(postio_render::Excerpt {
            text: &excerpt,
            offset,
            source_len: own.chars().count(),
        });
        if range.is_some() && self.revealed.get() != Some(shown) {
            self.revealed.set(Some(shown));
            self.reader.view().set_highlight(range);
        } else {
            self.reader.view().mark(range);
        }
    }

    pub fn thread_known(&self) -> usize {
        self.thread_total.get()
    }

    pub fn shown(&self) -> Option<MessageId> {
        self.shown.get()
    }

    pub fn body_text(&self) -> String {
        self.reader
            .view()
            .document()
            .map(|document| document.text.text.clone())
            .unwrap_or_default()
    }

    pub fn label_dots(&self) -> Vec<(String, String)> {
        self.dots.borrow().clone()
    }

    pub fn reader(&self) -> &Reader {
        &self.reader
    }
}

pub const DIALOG_NAME: &str = "focus-open-message";

fn root_type(stored: Option<&str>, body: &MessageBody, parts: &[Attachment]) -> String {
    if let Some(content_type) = stored {
        return content_type.to_owned();
    }
    match (parts.is_empty(), body.text.is_some(), body.html.is_some()) {
        (false, _, _) => "multipart/mixed".to_owned(),
        (true, true, true) => "multipart/alternative".to_owned(),
        (true, false, true) => "text/html".to_owned(),
        _ => "text/plain".to_owned(),
    }
}

type ChipHandler = Rc<dyn Fn(postio_model::ids::AttachmentId)>;

pub fn allowlist_path() -> std::path::PathBuf {
    RemoteImageAllowList::path()
}

fn words_in(document: &postio_render::RenderedDocument, rect: postio_render::Rect) -> String {
    let inside: Vec<&postio_render::Cluster> = document
        .text
        .clusters
        .iter()
        .filter(|cluster| {
            rect.contains(postio_render::Point::new(
                (cluster.rect.x0 + cluster.rect.x1) / 2.0,
                (cluster.rect.y0 + cluster.rect.y1) / 2.0,
            ))
        })
        .collect();
    match (
        inside.iter().map(|c| c.range.start).min(),
        inside.iter().map(|c| c.range.end).max(),
    ) {
        (Some(start), Some(end)) => document.text.slice(start..end).trim().to_owned(),
        _ => String::new(),
    }
}

fn remote_fetch(
    runtime: tokio::runtime::Handle,
) -> impl Fn(Vec<String>, postio_widgets::reader::view::RemoteArrived) + 'static {
    let fetcher = std::sync::Arc::new(postio_runtime::remote_images::RemoteImageFetcher::new());
    postio_widgets::present::reading::remote_fetch(runtime, move |parsed: Vec<url::Url>| {
        let fetcher = std::sync::Arc::clone(&fetcher);
        async move {
            fetcher
                .fetch_all(&parsed)
                .await
                .into_iter()
                .map(|(url, fetched)| match fetched {
                    postio_runtime::remote_images::Fetched::Image(bytes) => {
                        (url, Some(bytes.to_vec()))
                    }
                    postio_runtime::remote_images::Fetched::Failed(_) => (url, None),
                })
                .collect()
        }
    })
}

type Inline = Rc<RefCell<std::collections::HashMap<String, (Vec<u8>, String)>>>;

/// Without the angle brackets a `Content-ID` header wears, in any case.
fn cid_key(content_id: &str) -> String {
    content_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_ascii_lowercase()
}

fn tighten_keycaps(root: &gtk::Widget) {
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("postio-keycap-button")
            && let Some(content) = widget.first_child().and_downcast::<gtk::Box>()
        {
            content.set_spacing(focus_dialog::KEYCAP_GAP);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
}

fn step(icon: &str, name: &str) -> (gtk::Button, gtk::Box) {
    let image = gtk::Image::from_icon_name(icon);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    let key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    key.set_valign(gtk::Align::Center);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::STEP_KEYCAP_GAP);
    content.append(&image);
    content.append(&key);
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    postio_widgets::widgets::button::dress_icon(&button, name);
    button.add_css_class("focus-open-step");
    (button, key)
}
