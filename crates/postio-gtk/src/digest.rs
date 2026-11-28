//! The digest window: a 980x820 dialog for one delivery, opening on a plain list or, once a summary
//! is written, on the summary.
//! `Tab` switches; `Enter` on a reference opens its email with the cited passage highlighted, and
//! `Esc` returns to the same reference.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::Body;
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_model::listing::MessageSummary;
use postio_model::summary::DigestSummary;
use postio_ui::hints;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3, S4, S6};

use crate::list::Digest;

pub const DIALOG_NAME: &str = "focus-digest";

const WIDTH: i32 = 980;
const HEIGHT: i32 = 820;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestAction {
    ArchiveAll,
    EditRule,
    Open { message: MessageId, subject: String },
}

type Handler = Rc<dyn Fn(DigestAction)>;

pub use postio_ui::digest::Page as DigestPage;

pub struct DigestWindow {
    client: Client,
    dialog: adw::Dialog,
    title: gtk::Label,
    subtitle: gtk::Label,
    back_button: gtk::Button,
    archive: gtk::Button,
    archive_words: gtk::Box,
    rule_line: gtk::Box,
    sub_row: gtk::Box,
    tab_row: gtk::Box,
    tab_list_label: gtk::Label,
    scrolled: gtk::ScrolledWindow,
    list: gtk::ListBox,
    summary_scroll: gtk::ScrolledWindow,
    summary_box: gtk::Box,
    reference_card: gtk::Box,
    email_box: gtk::Widget,
    banner: gtk::Label,
    keymap: RefCell<Keymap>,
    digest: RefCell<Option<Digest>>,
    rule_when: RefCell<Option<String>>,
    rows: RefCell<Vec<MessageSummary>>,
    generation: Cell<u64>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<DigestWindow>>,
    reader: postio_widgets::reader::Reader,
    page: Cell<DigestPage>,
    summary: RefCell<Option<DigestSummary>>,
    focused_reference: Cell<Option<usize>>,
    paragraph_labels: RefCell<Vec<gtk::Label>>,
    reference_chips: RefCell<Vec<gtk::Label>>,
    statement_rows: RefCell<Vec<gtk::Box>>,
    email_shown: Cell<Option<MessageId>>,
    email_body: RefCell<Option<postio_model::MessageBody>>,
    email_excerpt: RefCell<Option<String>>,
}

impl DigestWindow {
    pub fn new(client: Client, keymap: &Keymap, allowlist: &std::path::Path) -> Rc<Self> {
        let reader = postio_widgets::reader::Reader::sharing(Rc::new(|_: &str| None), allowlist);
        let close = postio_widgets::widgets::close_button();
        close.add_css_class("focus-digest-close");
        close.set_valign(gtk::Align::Center);
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("focus-digest-back");
        back.set_label("Summary");
        back.set_visible(false);
        let title = gtk::Label::new(None);
        title.add_css_class("focus-digest-title");
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("focus-digest-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&title);
        titles.append(&subtitle);
        let archive = gtk::Button::new();
        postio_widgets::widgets::button::style(
            &archive,
            postio_widgets::widgets::Kind::Primary,
            postio_widgets::widgets::Size::Regular,
        );
        archive.add_css_class("focus-digest-archive");
        let archive_words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        archive.set_child(Some(&archive_words));
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-digest-header");
        let trailing = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        trailing.append(&archive);
        trailing.append(&close);
        header.set_start_widget(Some(&back));
        header.set_center_widget(Some(&titles));
        header.set_end_widget(Some(&trailing));

        let edit = gtk::Button::new();
        edit.add_css_class("flat");
        edit.add_css_class("focus-digest-edit");
        let rule_line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        edit.set_child(Some(&rule_line));
        edit.set_halign(gtk::Align::End);

        let tab_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        tab_row.add_css_class("focus-digest-tabs");
        tab_row.set_visible(false);
        let tab_summary = gtk::Button::new();
        tab_summary.add_css_class("flat");
        tab_summary.set_child(Some(&gtk::Label::new(Some("Summary"))));
        let tab_list_label = gtk::Label::new(None);
        let tab_list = gtk::Button::new();
        tab_list.add_css_class("flat");
        tab_list.set_child(Some(&tab_list_label));
        tab_row.append(&tab_summary);
        tab_row.append(&tab_list);
        if let Some(key) = hints::key(keymap, CommandId::ToggleDigestSummary) {
            tab_row.append(&keyhint::cap(&key));
        }

        let sub_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        sub_row.add_css_class("focus-digest-sub-row");
        sub_row.append(&tab_row);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        sub_row.append(&spacer);
        sub_row.append(&edit);

        let list = gtk::ListBox::new();
        list.add_css_class("focus-digest-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let summary_box = gtk::Box::new(gtk::Orientation::Vertical, S3);
        summary_box.add_css_class("focus-digest-summary");
        summary_box.set_margin_start(S6);
        summary_box.set_margin_end(S6);
        summary_box.set_margin_top(S4);
        summary_box.set_margin_bottom(S4);
        let summary_scroll = gtk::ScrolledWindow::builder()
            .child(&summary_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        summary_scroll.set_visible(false);
        let reference_card = gtk::Box::new(gtk::Orientation::Vertical, S1);
        reference_card.add_css_class("focus-digest-reference-card");

        let banner = gtk::Label::new(None);
        banner.add_css_class("focus-digest-reference-banner");
        banner.set_xalign(0.0);
        banner.set_wrap(true);
        let reader_widget = reader.widget();
        reader_widget.set_vexpand(true);
        let email_column = gtk::Box::new(gtk::Orientation::Vertical, S2);
        email_column.append(&banner);
        email_column.append(&reader_widget);
        let email_box = gtk::ScrolledWindow::builder()
            .child(&email_column)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        email_box.add_css_class("focus-digest-email");
        email_box.set_visible(false);
        // The scrolled window above is what visibility toggles; the box holding the banner and the
        // reader is what a test reads.
        let email_box: gtk::Widget = email_box.upcast();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-digest");
        content.append(&header);
        content.append(&sub_row);
        content.append(&scrolled);
        content.append(&summary_scroll);
        content.append(&email_box);
        let dialog = adw::Dialog::builder()
            .content_width(WIDTH)
            .content_height(HEIGHT)
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);

        let window = Rc::new(DigestWindow {
            client,
            dialog,
            title,
            subtitle,
            back_button: back.clone(),
            archive: archive.clone(),
            archive_words,
            rule_line,
            sub_row,
            tab_row,
            tab_list_label,
            scrolled,
            list,
            summary_scroll,
            summary_box,
            reference_card,
            email_box,
            banner,
            keymap: RefCell::new(keymap.clone()),
            digest: RefCell::default(),
            rule_when: RefCell::default(),
            rows: RefCell::default(),
            generation: Cell::new(0),
            handler: RefCell::default(),
            me: RefCell::default(),
            reader,
            page: Cell::new(DigestPage::List),
            summary: RefCell::default(),
            focused_reference: Cell::new(None),
            paragraph_labels: RefCell::default(),
            reference_chips: RefCell::default(),
            statement_rows: RefCell::default(),
            email_shown: Cell::default(),
            email_body: RefCell::default(),
            email_excerpt: RefCell::default(),
        });
        window.me.replace(Rc::downgrade(&window));
        let weak = Rc::downgrade(&window);
        close.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
            }
        });
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.back();
                }
            }
        });
        archive.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.emit(DigestAction::ArchiveAll);
                }
            }
        });
        edit.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.emit(DigestAction::EditRule);
                }
            }
        });
        tab_summary.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.show_page(DigestPage::Summary);
                }
            }
        });
        tab_list.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.show_page(DigestPage::List);
                }
            }
        });
        window.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let found = usize::try_from(row.index())
                    .ok()
                    .and_then(|index| window.rows.borrow().get(index).cloned());
                if let Some(found) = found {
                    window.emit(DigestAction::Open {
                        message: found.id,
                        subject: found.subject.unwrap_or_default(),
                    });
                }
            }
        });
        window.reader.view().connect_rendered({
            let weak = weak.clone();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    window.highlight_reference();
                }
            }
        });
        window
    }

    pub fn controls() -> Vec<CommandId> {
        vec![CommandId::Back, CommandId::Archive, CommandId::DigestRule]
    }

    pub fn connect_action(&self, handler: impl Fn(DigestAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show_header();
    }

    pub fn show(&self, parent: &impl IsA<gtk::Widget>, digest: Digest, rule_when: Option<String>) {
        self.digest.replace(Some(digest));
        self.rule_when.replace(rule_when);
        self.rows.borrow_mut().clear();
        self.summary.replace(None);
        self.focused_reference.set(None);
        self.page.set(DigestPage::List);
        self.show_rows();
        self.show_header();
        self.apply_page();
        self.dialog.present(Some(parent));
        self.read();
    }

    pub fn close(&self) {
        self.dialog.close();
    }

    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    pub fn digest(&self) -> Option<Digest> {
        self.digest.borrow().clone()
    }

    /// What `D` and `M` act on: the list's cursor, kept in step with the focused reference while
    /// the summary or its email is on screen.
    pub fn focused(&self) -> Option<MessageSummary> {
        let row = self.list.selected_row()?;
        let index = usize::try_from(row.index()).ok()?;
        self.rows.borrow().get(index).cloned()
    }

    pub fn step(&self, by: i32) {
        // The list behind the summary is not on screen, and a key that moved its cursor showed
        // nothing.
        if self.page.get() == DigestPage::Summary {
            self.step_reference(by);
            return;
        }
        let at = self.list.selected_row().map_or(-1, |row| row.index());
        if let Some(row) = self.list.row_at_index((at + by).max(0)) {
            self.list.select_row(Some(&row));
            row.grab_focus();
            if self.page.get() == DigestPage::Email {
                self.open_source_at_selection();
            }
        }
    }

    pub fn activate(&self) {
        match self.page.get() {
            DigestPage::List => {
                if let Some(row) = self.list.selected_row() {
                    row.activate();
                }
            }
            DigestPage::Summary => self.open_focused_reference(),
            DigestPage::Email => {}
        }
    }

    pub fn open_focused(&self) {
        self.activate();
    }

    pub fn showing(&self) -> DigestPage {
        self.page.get()
    }

    pub fn toggle_summary(&self) {
        let has_summary = self.summary.borrow().is_some();
        if let Some(next) = self.page.get().toggled(has_summary) {
            self.show_page(next);
        }
    }

    /// Clamped to the summary's ends.
    pub fn step_reference(&self, by: i32) {
        let len = self
            .summary
            .borrow()
            .as_ref()
            .map_or(0, |summary| summary.statements.len());
        let Some(next) = postio_ui::digest::step_reference(self.focused_reference.get(), by, len)
        else {
            return;
        };
        self.focused_reference.set(Some(next));
        self.show_focused_reference();
    }

    /// Answers whether it acted, so a plain `Esc` still closes the window everywhere else.
    pub fn back(&self) -> bool {
        let Some(page) = self.page.get().back() else {
            return false;
        };
        self.page.set(page);
        self.apply_page();
        true
    }

    pub fn references(&self) -> Vec<u32> {
        self.summary
            .borrow()
            .as_ref()
            .map(|summary| {
                summary
                    .statements
                    .iter()
                    .map(|statement| statement.reference.number)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn focused_reference(&self) -> Option<usize> {
        self.focused_reference.get()
    }

    pub fn statement_rows(&self) -> Vec<gtk::Box> {
        self.statement_rows.borrow().clone()
    }

    pub fn keyboard_on_statement(&self, index: usize) -> bool {
        let focus = self.dialog.focus();
        self.statement_rows
            .borrow()
            .get(index)
            .is_some_and(|row| focus.as_ref().is_some_and(|focus| focus == row))
    }

    pub fn paragraphs(&self) -> Vec<gtk::Label> {
        self.paragraph_labels.borrow().clone()
    }

    pub fn shown(&self) -> Option<MessageId> {
        self.email_shown.get()
    }

    pub fn reader(&self) -> &postio_widgets::reader::Reader {
        &self.reader
    }

    pub fn subjects(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .map(|row| row.subject.clone().unwrap_or_default())
            .collect()
    }

    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack: Vec<gtk::Widget> = self.dialog.child().into_iter().collect();
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && !label.text().is_empty()
            {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(previous) = child {
                child = previous.prev_sibling();
                stack.push(previous);
            }
        }
        said
    }

    fn emit(&self, action: DigestAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    fn show_page(&self, page: DigestPage) {
        self.page.set(page);
        self.apply_page();
    }

    fn apply_page(&self) {
        let page = self.page.get();
        self.scrolled.set_visible(page == DigestPage::List);
        self.summary_scroll.set_visible(page == DigestPage::Summary);
        self.email_box.set_visible(page == DigestPage::Email);
        self.sub_row.set_visible(page.shows_header());
        self.archive.set_visible(page.shows_header());
        self.back_button.set_visible(!page.shows_header());
        if page.shows_header() {
            self.show_header();
        }
        if page == DigestPage::Summary {
            self.show_focused_reference();
        } else if page == DigestPage::List
            && let Some(row) = self.list.selected_row()
        {
            row.grab_focus();
        }
        crate::a11y::teach_shortcuts(&self.dialog);
        crate::motion::keep_to_budget(&self.dialog);
    }

    fn show_header(&self) {
        let Some(digest) = self.digest.borrow().clone() else {
            return;
        };
        let keymap = self.keymap.borrow().clone();
        self.title.set_text(&postio_ui::digest::window_title(
            digest.cadence,
            &digest.rule,
        ));
        let now = postio_ui::clock::now();
        self.subtitle.set_text(&postio_ui::digest::window_subtitle(
            digest.count,
            digest.senders.len(),
            digest.at.with_timezone(&chrono::Local),
            now,
        ));
        while let Some(child) = self.archive_words.first_child() {
            self.archive_words.remove(&child);
        }
        self.archive_words
            .append(&gtk::Label::new(Some(&postio_ui::digest::archive_all(
                digest.count,
            ))));
        if let Some(key) = hints::key(&keymap, CommandId::Archive) {
            self.archive_words.append(&keyhint::cap(&key));
        }
        while let Some(child) = self.rule_line.first_child() {
            self.rule_line.remove(&child);
        }
        let said = postio_ui::digest::rule_line(self.rule_when.borrow().as_deref());
        self.rule_line.append(&gtk::Label::new(Some(&said)));
        if let Some(key) = hints::key(&keymap, CommandId::DigestRule) {
            self.rule_line.append(&keyhint::cap(&key));
        }
    }

    fn read(&self) {
        let Some(digest) = self.digest.borrow().clone() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        let delivery = digest.delivery;
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let rows_read = client.delivery_messages(delivery).await;
            // POSTIO-GLIB-SAFE: as above.
            let summary_read = client.digest_summary(delivery).await;
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.generation.get() != generation {
                return;
            }
            match rows_read {
                Ok(rows) => {
                    window.rows.replace(rows);
                    window.show_rows();
                    if let Some(first) = window.list.row_at_index(0) {
                        window.list.select_row(Some(&first));
                        first.grab_focus();
                    }
                }
                Err(error) => tracing::warn!(%error, "Focus could not read a digest: {error}"),
            }
            let summary = summary_read.ok().flatten();
            let has_summary = summary.as_ref().is_some_and(|summary| !summary.is_empty());
            window.summary.replace(summary);
            window.focused_reference.set(has_summary.then_some(0));
            window.show_summary();
            window.page.set(DigestPage::opening(has_summary));
            window.apply_page();
        });
    }

    fn show_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let now = postio_ui::clock::now();
        for row in self.rows.borrow().iter() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-digest-row");
            let sender = gtk::Label::new(Some(
                &row.from
                    .as_ref()
                    .map(|from| from.display().to_owned())
                    .unwrap_or_default(),
            ));
            sender.set_xalign(0.0);
            sender.set_width_chars(20);
            sender.set_max_width_chars(20);
            sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&sender);
            let subject = gtk::Label::new(row.subject.as_deref());
            subject.add_css_class("focus-digest-subject");
            subject.set_xalign(0.0);
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&subject);
            let preview = gtk::Label::new(row.preview.as_deref());
            preview.add_css_class("dim-label");
            preview.set_xalign(0.0);
            preview.set_hexpand(true);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&preview);
            let time = gtk::Label::new(Some(&postio_ui::row::timestamp(row.received_at, now)));
            line.append(&time);
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
        }
        self.tab_list_label
            .set_text(&format!("{} messages", self.rows.borrow().len()));
    }

    fn show_summary(&self) {
        while let Some(child) = self.summary_box.first_child() {
            self.summary_box.remove(&child);
        }
        self.paragraph_labels.borrow_mut().clear();
        self.reference_chips.borrow_mut().clear();
        self.statement_rows.borrow_mut().clear();
        let summary = self.summary.borrow().clone();
        let Some(summary) = summary.filter(|summary| !summary.is_empty()) else {
            self.tab_row.set_visible(false);
            return;
        };
        self.tab_row.set_visible(true);
        for topic in postio_ui::digest::topics(&summary.statements) {
            let heading = gtk::Label::new(Some(&postio_ui::digest::topic_heading(&topic)));
            heading.add_css_class("focus-digest-summary-topic");
            heading.set_xalign(0.0);
            self.summary_box.append(&heading);
            for statement in topic.statements {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
                row.add_css_class("focus-digest-summary-statement");
                // A real place for the keyboard: the focused statement holds it, so the outline and
                // ring show where a key goes.
                row.set_focusable(true);
                // Plain text, never markup: a model's `<a href=…>` or bare URL reads as its own
                // characters, never a live link.
                let text = gtk::Label::new(Some(&statement.text));
                text.set_wrap(true);
                text.set_xalign(0.0);
                text.set_hexpand(true);
                row.append(&text);
                let chip = gtk::Label::new(Some(&statement.reference.number.to_string()));
                chip.add_css_class("focus-digest-summary-reference");
                chip.set_valign(gtk::Align::Start);
                row.append(&chip);
                self.summary_box.append(&row);
                self.paragraph_labels.borrow_mut().push(text);
                self.reference_chips.borrow_mut().push(chip);
                self.statement_rows.borrow_mut().push(row);
            }
        }
        self.summary_box.append(&self.reference_card);
        let footer = gtk::Label::new(Some(&postio_ui::digest::summary_footer(summary.messages)));
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-digest-summary-footer");
        footer.set_wrap(true);
        footer.set_xalign(0.0);
        self.summary_box.append(&footer);
    }

    fn show_focused_reference(&self) {
        for chip in self.reference_chips.borrow().iter() {
            chip.remove_css_class("focus-digest-summary-reference-focused");
        }
        for row in self.statement_rows.borrow().iter() {
            row.remove_css_class("focus-digest-summary-focused");
        }
        while let Some(child) = self.reference_card.first_child() {
            self.reference_card.remove(&child);
        }
        let Some(index) = self.focused_reference.get() else {
            return;
        };
        let Some(statement) = self
            .summary
            .borrow()
            .as_ref()
            .and_then(|summary| summary.statements.get(index).cloned())
        else {
            return;
        };
        if let Some(chip) = self.reference_chips.borrow().get(index) {
            chip.add_css_class("focus-digest-summary-reference-focused");
        }
        if let Some(row) = self.statement_rows.borrow().get(index) {
            row.add_css_class("focus-digest-summary-focused");
            if self.page.get() == DigestPage::Summary {
                // The scroller brings a focused child into view.
                row.grab_focus();
            }
        }
        let message = statement.reference.message;
        let row = self
            .rows
            .borrow()
            .iter()
            .position(|row| row.id == message)
            .and_then(|position| {
                if let Some(list_row) = self.list.row_at_index(i32::try_from(position).ok()?) {
                    self.list.select_row(Some(&list_row));
                }
                self.rows.borrow().get(position).cloned()
            });
        let Some(row) = row else {
            return;
        };
        let sender = row.from.as_ref().map(|from| from.display().to_owned());
        let subject = gtk::Label::new(Some(&postio_ui::digest::reference_title(
            statement.reference.number,
            sender.as_deref(),
            row.subject.as_deref(),
        )));
        subject.set_wrap(true);
        subject.add_css_class("focus-digest-reference-subject");
        subject.set_xalign(0.0);
        self.reference_card.append(&subject);
        let hint = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        hint.append(&gtk::Label::new(Some(postio_ui::digest::OPEN_FULL_EMAIL)));
        let keymap = self.keymap.borrow().clone();
        if let Some(key) = hints::key(&keymap, CommandId::OpenMessage) {
            hint.append(&keyhint::cap(&key));
        }
        self.reference_card.append(&hint);
    }

    fn open_focused_reference(&self) {
        let Some(index) = self.focused_reference.get() else {
            return;
        };
        let Some(statement) = self
            .summary
            .borrow()
            .as_ref()
            .and_then(|summary| summary.statements.get(index).cloned())
        else {
            return;
        };
        self.show_page(DigestPage::Email);
        self.load_email(
            statement.reference.message,
            statement.reference.number,
            Some(statement.reference.excerpt),
        );
    }

    fn open_source_at_selection(&self) {
        let Some(row) = self.focused() else {
            return;
        };
        let cited = self.summary.borrow().as_ref().and_then(|summary| {
            summary
                .statements
                .iter()
                .find(|statement| statement.reference.message == row.id)
                .map(|statement| {
                    (
                        statement.reference.number,
                        statement.reference.excerpt.clone(),
                    )
                })
        });
        let (number, excerpt) = cited.unwrap_or((0, String::new()));
        self.load_email(row.id, number, (!excerpt.is_empty()).then_some(excerpt));
    }

    fn load_email(&self, message: MessageId, number: u32, excerpt: Option<String>) {
        self.email_shown.set(Some(message));
        self.email_body.replace(None);
        self.email_excerpt.replace(excerpt);
        self.reader.view().set_highlight(None);
        self.banner
            .set_text(&postio_ui::digest::cited_banner(number));
        let index = self.rows.borrow().iter().position(|row| row.id == message);
        let total = self.rows.borrow().len();
        let subject = self
            .rows
            .borrow()
            .iter()
            .find(|row| row.id == message)
            .and_then(|row| row.subject.clone())
            .unwrap_or_default();
        self.title.set_text(&subject);
        self.subtitle
            .set_text(&postio_ui::digest::source_line(index, total));
        self.reader
            .show_absent(postio_ui::reader::document::Absent::Partial);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.readings(vec![message], false).await;
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.generation.get() != generation {
                return;
            }
            let Some(reading) = read.ok().and_then(|mut readings| readings.pop()) else {
                window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Missing);
                return;
            };
            if let Some(row) = reading.row.as_deref() {
                window.reader.set_message_header(
                    &row.from,
                    &row.to,
                    &row.cc,
                    row.subject.as_deref(),
                    row.date.unwrap_or(row.received_at),
                );
                window
                    .reader
                    .set_sender_warning(reading.authenticity.warning.as_deref());
            }
            let sender = reading
                .row
                .as_deref()
                .and_then(|row| row.from.first())
                .map(|from| from.address.clone());
            match reading.body {
                Body::Ready {
                    body,
                    encoding_problems,
                } => {
                    window.email_body.replace(Some(body.clone()));
                    window.reader.render(&body, sender.as_deref());
                    window.reader.set_encoding_problems(encoding_problems);
                }
                Body::Partial => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Partial),
                Body::Offline => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Offline),
                Body::Missing => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Missing),
                Body::Empty => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Empty),
                Body::ForeignDraft => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::ForeignDraft),
                Body::Failed => window
                    .reader
                    .show_absent(postio_ui::reader::document::Absent::Failed),
            }
            crate::a11y::teach_shortcuts(&window.dialog);
            crate::motion::keep_to_budget(&window.dialog);
        });
    }

    fn highlight_reference(&self) {
        let (Some(excerpt), Some(body), Some(document)) = (
            self.email_excerpt.borrow().clone(),
            self.email_body.borrow().clone(),
            self.reader.view().document(),
        ) else {
            return;
        };
        let own = postio_body::own_text(&body);
        let offset = own.find(&excerpt).map_or(0, |at| own[..at].chars().count());
        let range = document.text.locate(postio_render::Excerpt {
            text: &excerpt,
            offset,
            source_len: own.chars().count(),
        });
        self.reader.view().set_highlight(range);
    }
}
