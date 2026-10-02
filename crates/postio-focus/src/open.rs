//! The open-email dialog (screen 04; contracts/focus-surface.md, "The
//! open-email dialog"): `Enter` opens the conversation under the cursor over
//! the list, and the list stays in place behind it.
//!
//! One dialog and one [`Reader`] for the window's life (scenario 7): the
//! hundredth open reuses the message view the first one built. The header --
//! the subject, the position, the thread chip, the labels -- is drawn from
//! the row already in hand, so the dialog is up before anything is read; the
//! body arrives from the store and is drawn only if it is still the message
//! on screen when it lands.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::Body;
use postio_core::{CommandId, Keymap};
use postio_model::{Attachment, MessageBody, MessageId};
use postio_ui::focus_dialog::{self, rhythm};
use postio_ui::hints;
use postio_widgets::reader::{Reader, RemoteImageAllowList, Verbs};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::S3;
use postio_widgets::widgets::{Action, ActionBar, Kind, Size};

use crate::list::FocusRow;
use crate::open_header::HeaderCard;

/// The window size the dialog is fitted to before it knows its window's:
/// the window's own default.
const WINDOW: (i32, i32) = (1440, 900);

/// The toolbar, in the order screen 04 draws it. Task and Note join in
/// milestone 3 (spec C9).
const TOOLBAR: &[Action] = &[
    Action::new(CommandId::Reply, "Reply", "focus-open-reply"),
    Action::new(CommandId::ReplyAll, "Reply all", "focus-open-reply-all"),
    Action::new(CommandId::Forward, "Forward", "focus-open-forward"),
    Action::new(CommandId::Archive, "Archive", "focus-open-archive"),
    Action::new(CommandId::Snooze, "Snooze", "focus-open-snooze"),
    Action::new(CommandId::RemindIfNoReply, "Remind", "focus-open-remind"),
    Action::new(CommandId::AddLabel, "Label", "focus-open-label"),
    Action::new(CommandId::Move, "Move", "focus-open-move"),
    Action::new(CommandId::Delete, "Delete", "focus-open-delete"),
    Action::new(CommandId::MoreActions, "More", "focus-open-more"),
];

/// What a narrow dialog folds into More (T206), in the action row's order.
const FOLDED: [CommandId; 3] = [CommandId::AddLabel, CommandId::Move, CommandId::Delete];

/// What a control in the dialog asks the window to do.
type Handler = Rc<dyn Fn(CommandId)>;

/// Where the dialog is in the list: the row's place and how many there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The row's index in the list, from 0.
    pub index: u32,
    /// How many rows the list has.
    pub total: u32,
}

/// The dialog, and the one message view in it.
pub struct OpenMessage {
    client: Client,
    dialog: adw::Dialog,
    title: gtk::Label,
    subtitle: gtk::Label,
    up_key: gtk::Box,
    down_key: gtk::Box,
    toolbar: Rc<ActionBar>,
    thread_chip: gtk::Box,
    subject: gtk::Label,
    labels: gtk::Box,
    /// What holds the column to its width, centred in the dialog (T207).
    clamp: adw::Clamp,
    /// More's menu: the folded verbs, with their keys.
    more: gtk::Popover,
    more_items: gtk::Box,
    /// The window's size the dialog was last fitted to, and whether it
    /// follows the window's resizes yet.
    window: Cell<(i32, i32)>,
    following: Cell<bool>,
    /// How wide the action row is with every verb laid out, once measured.
    row_width: Cell<Option<i32>>,
    /// The dialog itself, for a handler that must not keep it alive.
    this: RefCell<std::rc::Weak<OpenMessage>>,
    /// Who it is from, to and copied, and when (screen 04): drawn here, in
    /// place of the reader's own header, which Focus does not show.
    header_card: Rc<HeaderCard>,
    reader: Reader,
    fold_line: gtk::Button,
    fold_label: gtk::Label,
    keymap: RefCell<Keymap>,
    /// The message on screen, and a count that moves with every open, so a
    /// body that lands for an earlier one is dropped.
    shown: Rc<Cell<Option<MessageId>>>,
    /// The message whose marked sentence the column was taken to: once, as
    /// it opened. Every later draw of it -- `O`, images allowed, a fold --
    /// marks the sentence where it stands and leaves the place the person's.
    revealed: Cell<Option<MessageId>>,
    /// The inline parts of the message on screen, by content id: what the
    /// reader's `cid:` images resolve to.
    inline: Inline,
    generation: Rc<Cell<u64>>,
    open: Rc<Cell<bool>>,
    handler: RefCell<Option<Handler>>,
    /// The conversation's messages, oldest first, once read, and which of
    /// them is on screen; the row's place in the list.
    thread: Rc<RefCell<Vec<MessageId>>>,
    at: Rc<Cell<usize>>,
    position: Cell<Position>,
    messages: Cell<u32>,
    /// The parts of the message on screen, as its row lists them.
    parts: Rc<RefCell<Vec<Attachment>>>,
    /// The row's marker, and the message it belongs to.
    marker: RefCell<Option<(MessageId, postio_model::listing::MarkerSummary)>>,
    /// The dot each label's pill carries: its name and colour, `#rrggbb`.
    dots: RefCell<Vec<(String, String)>>,
    /// The card drawn for it, while it is shown.
    card: RefCell<Option<gtk::Box>>,
    /// The body on screen, once it has landed: where the marker's sentence
    /// is looked for.
    body: Rc<RefCell<Option<MessageBody>>>,
    /// Whether a to-do's card offers Task: once a vault is configured.
    capture: Cell<bool>,
}

impl OpenMessage {
    /// The commands the dialog has a control for: its toolbar, Close and
    /// the two steps.
    pub fn controls() -> Vec<CommandId> {
        TOOLBAR
            .iter()
            .map(|action| action.command)
            .chain([
                CommandId::Back,
                CommandId::PrevMessage,
                CommandId::NextMessage,
                CommandId::DismissMarker,
                // The render-mode line's switch, over an HTML body (T213).
                CommandId::SwitchTreatment,
            ])
            .collect()
    }

    /// The dialog, reading through `client`, with the remote-image allow
    /// list at `allowlist` and its keys from `keymap`.
    ///
    /// `runtime` is where a remote image is fetched, for a sender the person
    /// allowed or a message they chose to show once; with none, nothing is
    /// ever fetched.
    pub fn new(
        client: Client,
        keymap: &Keymap,
        allowlist: &std::path::Path,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Rc<Self> {
        // Inline (`cid:`) images resolve against the message on screen. The
        // reader asks synchronously, while it lays the document out, so the
        // message's inline parts are read from the store's owner before it
        // is rendered (`load`) and the reader is answered from them -- never
        // a blocking call on the main thread (#1608).
        let shown: Rc<Cell<Option<MessageId>>> = Rc::default();
        let inline: Inline = Rc::default();
        let source = {
            let inline = Rc::clone(&inline);
            Rc::new(move |content_id: &str| inline.borrow().get(&cid_key(content_id)).cloned())
        };
        let reader = Reader::sharing(source, allowlist, Verbs::NONE);
        // The subject is the column's heading, over the header card.
        reader.header().widget().set_visible(false);
        let header_card = Rc::new(HeaderCard::new());
        if let Some(runtime) = runtime {
            reader.set_remote_fetch(remote_fetch(runtime));
        }

        // The header: Close, the title and position, and the steps.
        // An X icon at the right (T189), apart from the verbs; Escape
        // still closes, so it carries no keycap.
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

        // The steps (T219): one pair, each a single control carrying its
        // key inside it -- the chevron, then its cap -- as every verb in the
        // dialog carries its own ("Reply e"). A cap standing beside its
        // button made four things of two, and taught a screen reader
        // nothing: a cap inside a button is the button's shortcut
        // (`a11y::teach_shortcuts`).
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

        let toolbar = ActionBar::new(TOOLBAR, "focus-open-toolbar");
        // The handoff's verbs sit edge to edge, each padded 8px a side.
        if let Some(row) = toolbar.widget().downcast_ref::<gtk::Box>() {
            row.set_spacing(0);
        }
        tighten_keycaps(&toolbar.widget());
        // More, and the menu it opens: built once, filled from the keymap.
        let more_items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // The row menu's dress (T199): one menu pattern (FR-092).
        more_items.add_css_class("focus-row-menu");
        let more = gtk::Popover::builder()
            .child(&more_items)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        more.add_css_class("focus-row-menu-popover");
        more.add_css_class("focus-open-more-menu");
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

        // The column: the thread marker, the subject, the labels, the
        // sender block, the reader (the action card, the body, the
        // attachments), and the fold line. Each block carries the gap the
        // handoff gives it (`rhythm`), so a block that is absent takes its
        // gap with it (T208).
        let thread_chip = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::THREAD_GAP);
        thread_chip.add_css_class("focus-open-thread");
        thread_chip.set_margin_bottom(rhythm::MARKER_TO_SUBJECT);
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
        // The body sits 24px under the action card, or under the sender
        // block when there is no card.
        reader.view().set_margin_top(rhythm::CARD_TO_BODY);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("focus-open-column");
        column.set_margin_top(rhythm::TOP);
        column.set_margin_bottom(rhythm::BOTTOM);
        // Its own height, not the page's: the bottom padding sits under the
        // last block, not at the foot of a stretched column.
        column.set_valign(gtk::Align::Start);
        column.append(&thread_chip);
        column.append(&subject);
        column.append(&labels);
        column.append(&header_widget);
        column.append(&reader_widget);
        column.append(&fold_line);
        // One column for everything inside the message (T207): its width
        // comes from the dialog's and the body's treatment
        // (`focus_dialog::column_width`), centred, and every block fills
        // it, so they share both edges. Everything from the thread marker
        // to the fold line scrolls together (screen 04), the body drawn in
        // it rather than in a scroller of its own.
        let clamp = adw::Clamp::builder()
            .child(&column)
            .maximum_size(focus_dialog::COLUMN_APP_COLOURS)
            .tightening_threshold(focus_dialog::COLUMN_APP_COLOURS)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        scroller.add_css_class("focus-open-scroller");
        reader.flow_in(&scroller);
        // Every body in app colours or as sent on paper, named by a line
        // above it (T210-T213); `reader.treatment()` is what the column's
        // width follows.
        reader.use_treatments();
        // Over an HTML body the line takes the card's 24 above it and keeps
        // 12 of its own to the body (T208); with no line, the body has the 24.
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
        // Find sits above the column, not at its top, so opening it keeps
        // the reading position (T203).
        content.append(reader.find_bar().widget());
        content.append(&scroller);

        let dialog = adw::Dialog::builder()
            .content_width(focus_dialog::dialog_width(WINDOW.0))
            .content_height(focus_dialog::dialog_height(WINDOW.1))
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);
        let open = Rc::new(Cell::new(false));
        dialog.connect_closed({
            let open = Rc::clone(&open);
            move |_| open.set(false)
        });

        let page = Rc::new(OpenMessage {
            client,
            dialog,
            title,
            subtitle,
            up_key,
            down_key,
            toolbar,
            thread_chip,
            subject,
            labels,
            clamp,
            more,
            more_items,
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
            open,
            handler: RefCell::default(),
            thread: Rc::default(),
            at: Rc::default(),
            position: Cell::new(Position { index: 0, total: 0 }),
            messages: Cell::new(1),
            parts: Rc::default(),
            marker: RefCell::default(),
            dots: RefCell::default(),
            card: RefCell::default(),
            body: Rc::default(),
            capture: Cell::new(false),
        });

        let weak = Rc::downgrade(&page);
        page.this.replace(weak.clone());
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
        page.toolbar.connect_command({
            let weak = weak.clone();
            move |command| {
                if let Some(page) = weak.upgrade() {
                    page.run(command.id());
                }
            }
        });
        page.fold_line.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open_first_fold();
                }
            }
        });
        // When the body view has the new snapshot, not when the render was
        // asked for: the fold line and the highlight are read off it.
        page.reader.view().connect_rendered({
            let weak = weak.clone();
            move |_| {
                if let Some(page) = weak.upgrade() {
                    page.show_fold_line();
                    page.highlight_marker();
                }
            }
        });
        // A body shown on paper takes the wider column; one shown in the
        // app's colours the narrower. The dialog itself does not move.
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

    /// Size the dialog for a window `width` by `height` (T205): its width
    /// and height come from the window and nothing else, so stepping
    /// through the list never resizes it.
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
        self.fold_into_more(
            focus_dialog::folds_into_more(dialog) || self.full_row_width() > dialog,
        );
    }

    /// How wide the action row is with every verb laid out: the handoff's
    /// 760px rule assumes its narrower face, so a row that would not fit
    /// folds too, rather than widening the dialog past its window's rule.
    /// Measured once on screen and kept: only its words and its keys
    /// change it.
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

    /// Fold Label, Move and Delete into More, or lay them out again.
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

    /// Whether the action row has folded its last verbs into More.
    pub fn folded(&self) -> bool {
        self.toolbar
            .button(CommandId::MoreActions)
            .is_some_and(|button| button.widget().is_visible())
    }

    /// More (`.`): the verbs the action row folded away, in a menu under
    /// its button, each running its one command. Nothing when nothing is
    /// folded.
    pub fn show_more(&self) {
        if !self.folded() {
            return;
        }
        while let Some(child) = self.more_items.first_child() {
            self.more_items.remove(&child);
        }
        let keymap = self.keymap.borrow().clone();
        for command in FOLDED {
            let Some(action) = TOOLBAR.iter().find(|action| action.command == command) else {
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
        self.more.popup();
        if let Some(first) = self.more_items.first_child() {
            first.grab_focus();
        }
    }

    /// Whether More's menu is open.
    pub fn more_open(&self) -> bool {
        self.more.is_visible()
    }

    /// More's menu, for a test to read.
    pub fn more_menu(&self) -> gtk::Popover {
        self.more.clone()
    }

    /// The column's width, for the dialog's and the body's treatment.
    fn fit_column(&self) {
        let dialog = focus_dialog::dialog_width(self.window.get().0);
        let column = focus_dialog::column_width(dialog, self.reader.treatment());
        if self.clamp.maximum_size() != column {
            self.clamp.set_maximum_size(column);
            self.clamp.set_tightening_threshold(column);
        }
    }

    /// Fit the dialog to `parent`'s window now, and again whenever that
    /// window is resized: its surface's `layout` says when, for every new
    /// size, maximised and tiled ones included, and the window's own size
    /// is read then -- the surface's includes the shadow a restored window
    /// draws around itself.
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

    /// Whether a to-do's card offers Task `t` beside Snooze: once a vault is
    /// configured (spec C9). Read when a card is next drawn.
    pub fn set_capture(&self, capture: bool) {
        self.capture.set(capture);
    }

    /// Run `handler` with the command a control stands for.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    fn run(&self, command: CommandId) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(command);
        }
    }

    /// Read every key the dialog shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.toolbar.set_keymap(keymap);
        // The action row's caps are tight: `Del`, not `Delete` (SPEC 2).
        for action in TOOLBAR {
            if let Some(button) = self.toolbar.button(action.command) {
                let key = hints::key(keymap, action.command).map(|key| hints::short(&key));
                button.set_key(key.as_deref());
            }
        }
        // The render-mode line's cap among them (T213).
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

    /// Show `row`, at `position` in the list, over `parent`: at once, from
    /// what the row says, then the body from the store.
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
        self.messages.set(summary.message_count.max(1));
        self.thread.borrow_mut().clear();
        self.at.set(0);
        self.show_position(true);
        self.show_labels(&conversation.labels);
        self.marker
            .replace(summary.marker.clone().map(|marker| (message, marker)));

        if !self.open.get() {
            self.follow(parent.upcast_ref());
            self.dialog.present(Some(parent));
            self.open.set(true);
        }
        self.show_message(message);
        if let (Some(thread), true) = (summary.id, summary.message_count > 1) {
            self.read_thread(thread, message);
        }
    }

    /// Show `message`, found by a search rather than a row of the list:
    /// its subject, no position in the list, and no marker or labels until
    /// the reading has them.
    pub fn show_found(&self, parent: &impl IsA<gtk::Widget>, message: MessageId, subject: &str) {
        self.title.set_text(subject);
        self.subject.set_text(subject);
        self.subtitle.set_text("Found by search");
        self.messages.set(1);
        self.thread.borrow_mut().clear();
        self.at.set(0);
        self.show_thread_chip(1);
        self.show_labels(&[]);
        self.marker.replace(None);
        if !self.open.get() {
            self.follow(parent.upcast_ref());
            self.dialog.present(Some(parent));
            self.open.set(true);
        }
        self.show_message(message);
    }

    /// Show `message` of the conversation on screen: clear what the last
    /// one left, and read this one.
    fn show_message(&self, message: MessageId) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        // The inline-image source reads this same cell.
        self.shown.set(Some(message));
        self.revealed.set(None);
        self.body.replace(None);
        self.show_marker_card(message);
        self.header_card.clear();
        self.fold_line.set_visible(false);
        self.reader
            .show_absent(postio_ui::reader::document::Absent::Partial);
        // Whatever the last message was read down to, this one is read from
        // its top: the view is put there now, and the new document's first
        // snapshot starts there too.
        self.reader.view().scroll_to_edge(false);
        self.load(message, generation);
    }

    /// Read the conversation's messages, so `[` and `]` can step through
    /// them, and find `showing` among them.
    fn read_thread(&self, thread: postio_model::ThreadId, showing: MessageId) {
        let client = self.client.clone();
        let (messages, at) = (Rc::clone(&self.thread), Rc::clone(&self.at));
        let generation = Rc::clone(&self.generation);
        let asked = generation.get();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.conversation(thread).await;
            let Ok(rows) = read else { return };
            if generation.get() != asked {
                return;
            }
            let ids: Vec<MessageId> = rows.iter().map(|row| row.id).collect();
            at.set(
                ids.iter()
                    .position(|id| *id == showing)
                    .unwrap_or(ids.len().saturating_sub(1)),
            );
            messages.replace(ids);
        });
    }

    /// Step `by` messages through the conversation: `[` is -1, `]` is 1.
    /// Nothing happens past either end, or before the conversation is read.
    pub fn step_thread(&self, by: isize) {
        let (next, message) = {
            let thread = self.thread.borrow();
            let Some(next) = self.at.get().checked_add_signed(by) else {
                return;
            };
            match thread.get(next) {
                Some(message) => (next, *message),
                None => return,
            }
        };
        self.at.set(next);
        self.show_position(false);
        self.show_message(message);
    }

    /// The position line and the thread chip, for where the dialog is: the
    /// row's place in the list, and the message's in the conversation.
    fn show_position(&self, latest: bool) {
        let position = self.position.get();
        let messages = self.messages.get();
        let thread_len = self.thread.borrow().len();
        let latest = latest || thread_len == 0 || self.at.get() + 1 == thread_len;
        let mut said = format!("Message {} of {}", position.index + 1, position.total);
        if messages > 1 {
            if latest {
                said.push_str(&format!(" \u{b7} thread of {messages}"));
            } else {
                said.push_str(&format!(
                    " \u{b7} {} of {messages} in the thread",
                    self.at.get() + 1
                ));
            }
        }
        self.subtitle.set_text(&said);
        self.show_thread_chip(messages);
    }

    /// Read `message` from the store, and draw it if it is still the one on
    /// screen when it lands.
    fn load(&self, message: MessageId, generation: u64) {
        let client = self.client.clone();
        let reader = self.reader.clone();
        let current = Rc::clone(&self.generation);
        let shown_parts = Rc::clone(&self.parts);
        let shown_body = Rc::clone(&self.body);
        let inline = Rc::clone(&self.inline);
        let header_card = Rc::clone(&self.header_card);
        shown_parts.borrow_mut().clear();
        inline.borrow_mut().clear();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.readings(vec![message], false).await;
            if current.get() != generation {
                return;
            }
            let Some(reading) = read.ok().and_then(|mut readings| readings.pop()) else {
                reader.show_absent(postio_ui::reader::document::Absent::Missing);
                return;
            };
            if let Some(row) = reading.row.as_deref() {
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
                    // The inline images, before the reader asks for them.
                    for content_id in parts.iter().filter_map(|part| part.content_id.clone()) {
                        // Asked for as a `cid:` URI names it, as the reader
                        // would have asked.
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
                    shown_body.replace(Some(body.clone()));
                    reader.render(&body, sender.as_deref());
                    reader.set_encoding_problems(encoding_problems);
                }
                Body::Partial => reader.show_absent(postio_ui::reader::document::Absent::Partial),
                Body::Offline => reader.show_absent(postio_ui::reader::document::Absent::Offline),
                Body::Missing => reader.show_absent(postio_ui::reader::document::Absent::Missing),
                Body::Empty => reader.show_absent(postio_ui::reader::document::Absent::Empty),
                Body::ForeignDraft => {
                    reader.show_absent(postio_ui::reader::document::Absent::ForeignDraft);
                }
            }
        });
    }

    /// "Latest of 6 in this thread · [ earlier message", for a thread.
    fn show_thread_chip(&self, messages: u32) {
        while let Some(child) = self.thread_chip.first_child() {
            self.thread_chip.remove(&child);
        }
        self.thread_chip.set_visible(messages > 1);
        if messages <= 1 {
            return;
        }
        let keymap = self.keymap.borrow();
        self.thread_chip.append(&gtk::Label::new(Some(&format!(
            "Latest of {messages} in this thread"
        ))));
        if let Some(key) = hints::key(&keymap, CommandId::PrevInConversation) {
            self.thread_chip.append(&keyhint::cap(&key));
            self.thread_chip
                .append(&gtk::Label::new(Some("earlier message")));
        }
    }

    /// The conversation's label pills, and "+ Label".
    fn show_labels(&self, labels: &[postio_model::Label]) {
        while let Some(child) = self.labels.first_child() {
            self.labels.remove(&child);
        }
        let mut dots = Vec::new();
        for label in labels {
            // The pill: the label's colour dot, as the list's pills draw it,
            // and its name.
            let colour = crate::places::label_rgb(label);
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, focus_dialog::LABEL_GAP);
            pill.add_css_class("focus-open-label-pill");
            let dot = crate::places::colour_dot(colour);
            dot.add_css_class("focus-open-label-dot");
            // The dot's own size, so the pill's 8px inset reaches it.
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

    /// The fold line under the body: what the first closed fold hides, and
    /// the raw source's key.
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

    /// Open the first closed fold: what the fold line does.
    fn open_first_fold(&self) {
        let folds = self.reader.view().folds();
        if let Some((id, _, _)) = folds.iter().find(|(_, _, open)| !open) {
            self.reader.view().open_fold(id);
        }
    }

    /// Close the dialog. The list behind it has not moved.
    pub fn close(&self) {
        self.dialog.close();
        self.open.set(false);
    }

    /// Whether the dialog is up.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The dialog, for a test to read.
    pub fn dialog(&self) -> adw::Dialog {
        self.dialog.clone()
    }

    /// The header's title: the subject.
    pub fn title(&self) -> String {
        self.title.text().to_string()
    }

    /// The header's position line.
    pub fn subtitle(&self) -> String {
        self.subtitle.text().to_string()
    }

    /// What `o` offers for the message on screen: its links, each with the
    /// words it was written behind and its full target, then its parts.
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

    /// Press the card's Dismiss, as a click does; whether there was one.
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

    /// What the marker card says, piece by piece; empty with no card.
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

    /// The marker card under the header, for `message` when the marker is
    /// its (a thread's other messages have none).
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

    /// The card for `marker` (contracts/focus-surface.md, 04.5): its kind,
    /// its date, its sentence, and the actions that answer it with their
    /// keys -- or what is true instead.
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
            // The whole sentence, wrapping onto a second line when it needs
            // one (SPEC section 5): it is what the card is about, and cut
            // short it no longer says what was asked.
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
        // `-`: the marker was wrong, or is done with (T118). Quieter than
        // the answers: it answers nothing.
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

    /// Take the marker card away: its marker was dismissed.
    pub fn clear_marker(&self) {
        self.marker.replace(None);
        if let Some(shown) = self.shown.get() {
            self.show_marker_card(shown);
        }
        self.reader.view().set_highlight(None);
    }

    /// Highlight the marker's sentence in the body, where it is drawn
    /// (spec 007 FR-035, research R2): found by its words through
    /// `TextIndex::locate`, whose offset is chars into the body's own text,
    /// the text the detector read.
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
        // Where the sentence starts in that text. The first place its words
        // stand: the marker keeps its span, the listing does not carry it.
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

    /// How many of the conversation's messages `[` and `]` can step
    /// through: none until the conversation has been read.
    pub fn thread_known(&self) -> usize {
        self.thread.borrow().len()
    }

    /// The message on screen.
    pub fn shown(&self) -> Option<MessageId> {
        self.shown.get()
    }

    /// The words of the body as drawn.
    pub fn body_text(&self) -> String {
        self.reader
            .view()
            .document()
            .map(|document| document.text.text.clone())
            .unwrap_or_default()
    }

    /// Each label's dot: its name and colour as `#rrggbb`.
    pub fn label_dots(&self) -> Vec<(String, String)> {
        self.dots.borrow().clone()
    }

    /// The message view.
    pub fn reader(&self) -> &Reader {
        &self.reader
    }
}

/// The dialog's widget name, so it can be told from another dialog.
pub const DIALOG_NAME: &str = "focus-open-message";

/// The MIME type the attachment chips hang from: the stored one, or what
/// the body and the parts imply (the desktop app's rule, `postio-app`'s
/// `reading::root_type`).
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

/// `RemoteImageAllowList`'s own file: the one every reader of the app shares.
pub fn allowlist_path() -> std::path::PathBuf {
    RemoteImageAllowList::path()
}

/// The words drawn inside `rect`.
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

/// The reader's remote-image fetch, on `runtime`: the desktop app's fetcher
/// (`postio_runtime::remote_images`), which the reader asks only for URLs
/// its allow list or a "show once" has cleared.
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

/// The inline parts of the message on screen, by content id.
type Inline = Rc<RefCell<std::collections::HashMap<String, (Vec<u8>, String)>>>;

/// A content id as a `cid:` URI and a part header both spell it: without
/// the angle brackets a `Content-ID` header wears, in any case.
fn cid_key(content_id: &str) -> String {
    content_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_ascii_lowercase()
}

/// Close each keycap under `root` up to its words: the handoff's 6px, where
/// the shared keycap button leaves 8.
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

/// One of the header's steps: a button showing `icon` and, after it, the
/// holder its key's cap is put in from the keymap. The shared icon button's
/// dress -- its name as the tooltip and accessible label -- with words
/// inside, so it is sized by them rather than as a square.
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
