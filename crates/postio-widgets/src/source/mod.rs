//! The Source surface as one dialog: a tab strip over three pages drawn from
//! `postio_ui::source::SourceView`. Every page is plain text in non-editable views with no link
//! detection and no markup, so a hostile header is characters and nothing else (spec 016 D3).

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use adw::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::source::{RawSegment, Section, SourceView, Tab};

use crate::widgets::button::{self, Kind, Size};
use crate::widgets::{keyhint, space};

pub const DIALOG_NAME: &str = "focus-source";

const WIDTH: i32 = 860;
const HEIGHT: i32 = 680;
const TAB_COMMANDS: [CommandId; 3] = [
    CommandId::SourceTabVerification,
    CommandId::SourceTabHeaders,
    CommandId::SourceTabRaw,
];

#[derive(Clone)]
pub struct SourceDialog {
    inner: Rc<Inner>,
}

struct Inner {
    dialog: adw::Dialog,
    title: gtk::Label,
    tabs: Vec<gtk::Button>,
    stack: gtk::Stack,
    verification: gtk::Box,
    headers: gtk::TextView,
    raw: gtk::TextView,
    find: gtk::SearchEntry,
    view: RefCell<SourceView>,
    folds: RefCell<Vec<Fold>>,
    tab: Cell<Tab>,
}

struct Fold {
    start: gtk::TextMark,
    end: gtk::TextMark,
    range: Range<usize>,
}

impl SourceDialog {
    /// `command` runs a registry command, as a key would: the tab buttons and close say what to
    /// do and the controller decides.
    pub fn new(
        view: SourceView,
        keymap: &Keymap,
        command: impl Fn(CommandId) + 'static,
    ) -> SourceDialog {
        let command = Rc::new(command);
        let title = gtk::Label::new(None);
        title.add_css_class("postio-plate-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let close = button::close_button();
        close.set_valign(gtk::Align::Center);
        let header = gtk::CenterBox::new();
        header.add_css_class("postio-source-header");
        header.set_center_widget(Some(&title));
        header.set_end_widget(Some(&close));

        let strip = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
        strip.add_css_class("postio-source-tabs");
        let tabs: Vec<gtk::Button> = Tab::ALL
            .iter()
            .zip(TAB_COMMANDS)
            .map(|(tab, id)| {
                let tab_button = gtk::Button::new();
                button::style(&tab_button, Kind::Ghost, Size::Regular);
                tab_button.add_css_class("postio-source-tab");
                keyhint::dress(
                    &tab_button,
                    tab.title(),
                    postio_ui::hints::key(keymap, id).as_deref(),
                );
                let run = Rc::clone(&command);
                tab_button.connect_clicked(move |_| run(id));
                strip.append(&tab_button);
                tab_button
            })
            .collect();

        let find = gtk::SearchEntry::new();
        find.set_placeholder_text(Some("Find in this tab"));
        find.set_visible(false);

        let verification = gtk::Box::new(gtk::Orientation::Vertical, space::S6);
        verification.add_css_class("postio-source-verification");
        let headers = text_view("Headers");
        let raw = text_view("Raw source");
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.set_vexpand(true);
        for (tab, page) in [
            (
                Tab::Verification,
                verification.clone().upcast::<gtk::Widget>(),
            ),
            (Tab::Headers, headers.clone().upcast()),
            (Tab::Raw, raw.clone().upcast()),
        ] {
            let scrolled = gtk::ScrolledWindow::builder()
                .child(&page)
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();
            stack.add_named(&scrolled, Some(tab.id()));
        }

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("postio-source");
        content.append(&header);
        content.append(&strip);
        content.append(&find);
        content.append(&stack);
        let dialog = adw::Dialog::builder()
            .content_width(WIDTH)
            .content_height(HEIGHT)
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);
        let back = Rc::clone(&command);
        close.connect_clicked(move |_| back(CommandId::Back));

        let inner = Rc::new(Inner {
            dialog,
            title,
            tabs,
            stack,
            verification,
            headers,
            raw,
            find,
            view: RefCell::new(view),
            folds: RefCell::new(Vec::new()),
            tab: Cell::new(Tab::Verification),
        });
        let weak = Rc::downgrade(&inner);
        inner.find.connect_search_changed(move |entry| {
            if let Some(inner) = weak.upgrade() {
                inner.search(&entry.text(), false);
            }
        });
        let weak = Rc::downgrade(&inner);
        inner.find.connect_activate(move |entry| {
            if let Some(inner) = weak.upgrade() {
                inner.search(&entry.text(), true);
            }
        });
        let weak = Rc::downgrade(&inner);
        inner.find.connect_stop_search(move |entry| {
            entry.set_text("");
            entry.set_visible(false);
            if let Some(inner) = weak.upgrade() {
                inner.page().grab_focus();
            }
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&inner);
        click.connect_released(move |_, _, x, y| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let (bx, by) =
                inner
                    .raw
                    .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let Some(at) = inner.raw.iter_at_location(bx, by) else {
                return;
            };
            let folded = inner.raw.buffer().tag_table().lookup("fold");
            if folded.is_some_and(|tag| at.has_tag(&tag)) {
                inner.raw.buffer().place_cursor(&at);
                SourceDialog {
                    inner: Rc::clone(&inner),
                }
                .unfold();
            }
        });
        inner.raw.add_controller(click);
        let dialog = SourceDialog { inner };
        dialog.draw();
        dialog.show_tab(Tab::Verification);
        dialog
    }

    pub fn dialog(&self) -> &adw::Dialog {
        &self.inner.dialog
    }

    /// The subject the surface is titled with.
    pub fn subject(&self) -> String {
        let view = self.inner.view.borrow();
        view.title
            .strip_prefix("Source · ")
            .unwrap_or(&view.title)
            .to_owned()
    }

    pub fn tab(&self) -> Tab {
        self.inner.tab.get()
    }

    /// The bytes arrived, or the view changed under the surface; the tab stays.
    pub fn set_view(&self, view: SourceView) {
        self.inner.view.replace(view);
        self.draw();
    }

    pub fn show_tab(&self, tab: Tab) {
        let inner = &self.inner;
        inner.tab.set(tab);
        inner.stack.set_visible_child_name(tab.id());
        for (button, each) in inner.tabs.iter().zip(Tab::ALL) {
            if each == tab {
                button.add_css_class("postio-source-tab-on");
            } else {
                button.remove_css_class("postio-source-tab-on");
            }
        }
        inner.find.set_visible(false);
        inner.page().grab_focus();
    }

    /// Every stored byte, folded or not; empty when the bytes are not here yet.
    pub fn bytes(&self) -> Vec<u8> {
        self.inner.view.borrow().raw.bytes.clone()
    }

    pub fn find(&self) {
        self.inner.find.set_visible(true);
        self.inner.find.grab_focus();
    }

    /// The fold under the Raw tab's cursor, or the first one after it. `false` when none is left.
    pub fn unfold(&self) -> bool {
        let inner = &self.inner;
        let buffer = inner.raw.buffer();
        let at = buffer.iter_at_mark(&buffer.get_insert()).offset();
        let mut folds = inner.folds.borrow_mut();
        let chosen = folds
            .iter()
            .position(|fold| buffer.iter_at_mark(&fold.end).offset() >= at)
            .or(if folds.is_empty() { None } else { Some(0) });
        let Some(chosen) = chosen else {
            return false;
        };
        let fold = folds.remove(chosen);
        let text = {
            let view = inner.view.borrow();
            String::from_utf8_lossy(view.raw.bytes.get(fold.range.clone()).unwrap_or_default())
                .into_owned()
        };
        let mut start = buffer.iter_at_mark(&fold.start);
        let mut end = buffer.iter_at_mark(&fold.end);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &text);
        buffer.delete_mark(&fold.start);
        buffer.delete_mark(&fold.end);
        true
    }

    pub fn text_of(&self, tab: Tab) -> String {
        let inner = &self.inner;
        match tab {
            Tab::Verification => {
                let mut out = Vec::new();
                let mut child = inner.verification.first_child();
                while let Some(widget) = child {
                    collect_labels(&widget, &mut out);
                    child = widget.next_sibling();
                }
                out.join("\n")
            }
            Tab::Headers => all_text(&inner.headers),
            Tab::Raw => all_text(&inner.raw),
        }
    }

    fn draw(&self) {
        let inner = &self.inner;
        let view = inner.view.borrow();
        inner.title.set_text(&view.title);
        inner.dialog.set_title(&view.title);
        while let Some(child) = inner.verification.first_child() {
            inner.verification.remove(&child);
        }
        for section in &view.verification.sections {
            inner.verification.append(&section_widget(section));
        }
        draw_headers(&inner.headers, &view);
        let folds = draw_raw(&inner.raw, &view);
        drop(view);
        inner.folds.replace(folds);
    }
}

impl Inner {
    fn page(&self) -> gtk::Widget {
        match self.tab.get() {
            Tab::Verification => self.verification.clone().upcast(),
            Tab::Headers => self.headers.clone().upcast(),
            Tab::Raw => self.raw.clone().upcast(),
        }
    }

    fn search(&self, needle: &str, onward: bool) {
        let text = match self.tab.get() {
            Tab::Headers => &self.headers,
            Tab::Raw => &self.raw,
            Tab::Verification => return,
        };
        if needle.is_empty() {
            return;
        }
        let buffer = text.buffer();
        let mut from = buffer.iter_at_mark(&buffer.get_insert());
        if onward {
            from.forward_char();
        }
        let flags = gtk::TextSearchFlags::CASE_INSENSITIVE;
        let found = from
            .forward_search(needle, flags, None)
            .or_else(|| buffer.start_iter().forward_search(needle, flags, None));
        if let Some((start, end)) = found {
            buffer.select_range(&start, &end);
            let mut start = start;
            text.scroll_to_iter(&mut start, 0.1, false, 0.0, 0.0);
        }
    }
}

fn text_view(name: &str) -> gtk::TextView {
    let text = gtk::TextView::new();
    text.set_editable(false);
    text.set_cursor_visible(true);
    text.set_monospace(true);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.add_css_class("postio-source-text");
    text.update_property(&[gtk::accessible::Property::Label(name)]);
    text
}

fn plain(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_selectable(true);
    label
}

fn section_widget(section: &Section) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
    block.add_css_class("postio-source-section");
    let title = plain(&section.title);
    title.add_css_class("heading");
    block.append(&title);
    if let Some(intro) = &section.intro {
        block.append(&plain(intro));
    }
    if let Some(mono) = &section.mono {
        let code = plain(mono);
        code.add_css_class("monospace");
        block.append(&code);
    }
    for line in &section.lines {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        row.append(&plain(&line.text));
        if let Some(mono) = &line.mono {
            let code = plain(mono);
            code.add_css_class("monospace");
            code.add_css_class("dim-label");
            row.append(&code);
        }
        if section.quiet {
            row.add_css_class("dim-label");
        }
        block.append(&row);
    }
    block
}

fn tag(buffer: &gtk::TextBuffer, name: &str, build: impl FnOnce(&gtk::TextTag)) -> gtk::TextTag {
    if let Some(tag) = buffer.tag_table().lookup(name) {
        return tag;
    }
    let tag = gtk::TextTag::new(Some(name));
    build(&tag);
    buffer.tag_table().add(&tag);
    tag
}

fn draw_headers(text: &gtk::TextView, view: &SourceView) {
    let buffer = text.buffer();
    buffer.set_text("");
    let name = tag(&buffer, "name", |tag| tag.set_weight(700));
    let quiet = tag(&buffer, "quiet", |tag| tag.set_scale(0.9));
    let mut end = buffer.end_iter();
    if let Some(note) = &view.headers.note {
        buffer.insert_with_tags(&mut end, &format!("{note}\n\n"), &[&quiet]);
    }
    if !view.headers.hops.is_empty() {
        buffer.insert_with_tags(&mut end, "Received, newest first\n", &[&name]);
        for hop in &view.headers.hops {
            let mut line = String::from("  ");
            if let Some(from) = &hop.from {
                line.push_str(&format!("from {from} "));
            }
            if let Some(by) = &hop.by {
                line.push_str(&format!("by {by}"));
            }
            if let Some(at) = &hop.at {
                line.push_str(&format!(" · {at}"));
            }
            if let Some(delay) = &hop.delay {
                line.push_str(&format!(" · {delay} after the hop before"));
            }
            line.push('\n');
            buffer.insert(&mut end, &line);
        }
        buffer.insert(&mut end, "\n");
    }
    for row in &view.headers.rows {
        if let Some(boundary) = &row.boundary_above {
            buffer.insert_with_tags(&mut end, &format!("──── {boundary}\n"), &[&quiet]);
        }
        buffer.insert_with_tags(&mut end, &format!("{}: ", row.name), &[&name]);
        buffer.insert(&mut end, &format!("{}\n", row.value));
        if let Some(raw) = &row.raw {
            buffer.insert_with_tags(&mut end, &format!("    as written: {raw}\n"), &[&quiet]);
        }
    }
}

fn draw_raw(text: &gtk::TextView, view: &SourceView) -> Vec<Fold> {
    let buffer = text.buffer();
    buffer.set_text("");
    let folded = tag(&buffer, "fold", |tag| tag.set_weight(700));
    let mut end = buffer.end_iter();
    if let Some(state) = &view.raw.state_line {
        buffer.insert(&mut end, state);
        return Vec::new();
    }
    let mut folds = Vec::new();
    for segment in &view.raw.segments {
        match segment {
            RawSegment::Text(text) => buffer.insert(&mut end, text),
            RawSegment::Fold { label, range } => {
                let start = buffer.create_mark(None, &end, true);
                buffer.insert_with_tags(&mut end, &format!("{label} — Show"), &[&folded]);
                let stop = buffer.create_mark(None, &end, false);
                folds.push(Fold {
                    start,
                    end: stop,
                    range: range.clone(),
                });
            }
        }
    }
    buffer.place_cursor(&buffer.start_iter());
    folds
}

fn all_text(text: &gtk::TextView) -> String {
    let buffer = text.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn collect_labels(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.text().to_string());
        return;
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        collect_labels(&next, out);
        child = next.next_sibling();
    }
}
