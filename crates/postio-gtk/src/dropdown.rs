//! The search dropdown (spec 010 step 2, screens 01-05): the bar's search
//! half on a frontend with a results view.
//!
//! It draws [`DropdownView`] and decides nothing: every word is the
//! controller's, every keycap the keymap's, as on the Mac
//! (`macos/Sources/PostioKit/DropdownView.swift`). Sections have a bold
//! secondary title and a tertiary note; rows sit 38 tall, inset from the
//! panel's edge; the saved searches and "Narrow to" are pills; the cheat
//! sheet is a grid with one example lowered live; a footer of keys carries
//! the count on its right.
//!
//! The arrows' highlight is the toolkit's selection, kept by token: a
//! redraw (the passages landing) leaves it where it was, a run's `select`
//! moves it, and otherwise the view's default `highlight` takes it. Where
//! it rests is said back through [`Dropdown::connect_highlighted`], so the
//! controller's forget and exclude keys act on the row a person sees.
//!
//! Matched words get the find highlight (FR-005), never the accent.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use postio_focus::{
    DropdownPill, DropdownRow, DropdownRowKind, DropdownSection, DropdownView, Run, RunStyle,
    UnderstoodTile,
};
use postio_ui::search_view as words;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The find highlight (FR-005), light and dark: the design's yellow.
const FIND_LIGHT: (&str, &str) = ("#ffcc00", "38%");
const FIND_DARK: (&str, &str) = ("#ffd60a", "28%");

/// Columns of the cheat sheet (design §2).
const SHEET_COLUMNS: i32 = 4;
/// The folder column's width, and the right column's.
const FOLDER_WIDTH: i32 = 100;
const RIGHT_WIDTH: i32 = 110;

type TokenHandler = Rc<dyn Fn(u64)>;

/// The dropdown.
pub struct Dropdown {
    root: gtk::Box,
    understood: gtk::Box,
    list: gtk::ListBox,
    hints: gtk::Box,
    count: gtk::Label,
    /// The token of each row of `list`; `None` for a row that runs nothing.
    tokens: RefCell<Vec<Option<u64>>>,
    /// The view drawn now.
    view: RefCell<Option<DropdownView>>,
    /// Set while a redraw moves the selection, so it is not said back.
    quiet: Cell<bool>,
    run: RefCell<Option<TokenHandler>>,
    highlighted: RefCell<Option<TokenHandler>>,
    /// This dropdown, weakly: what a pill's click comes back to.
    me: RefCell<std::rc::Weak<Dropdown>>,
}

impl Dropdown {
    /// An empty dropdown.
    pub fn new() -> Rc<Self> {
        let understood = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        understood.add_css_class("focus-dropdown-understood");
        understood.set_visible(false);
        let list = gtk::ListBox::new();
        list.add_css_class("focus-dropdown-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_accessible_role(gtk::AccessibleRole::ListBox);
        list.update_property(&[gtk::accessible::Property::Label(words::A11Y_SUGGESTIONS)]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(560)
            .build();
        let hints = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        hints.set_hexpand(true);
        let count = gtk::Label::new(None);
        count.add_css_class("focus-dropdown-count");
        count.add_css_class("dim-label");
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        footer.add_css_class("focus-dropdown-footer");
        footer.append(&hints);
        footer.append(&count);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-dropdown");
        root.append(&understood);
        root.append(&scrolled);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&footer);

        let dropdown = Rc::new(Dropdown {
            root,
            understood,
            list,
            hints,
            count,
            tokens: RefCell::default(),
            view: RefCell::default(),
            quiet: Cell::new(false),
            run: RefCell::default(),
            highlighted: RefCell::default(),
            me: RefCell::default(),
        });
        dropdown.me.replace(Rc::downgrade(&dropdown));
        let weak = Rc::downgrade(&dropdown);
        dropdown.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(dropdown) = weak.upgrade()
                    && let Some(token) = dropdown.token_at(row.index())
                {
                    dropdown.say_run(token);
                }
            }
        });
        dropdown.list.connect_row_selected(move |_, row| {
            if let Some(dropdown) = weak.upgrade()
                && !dropdown.quiet.get()
                && let Some(token) = row.and_then(|row| dropdown.token_at(row.index()))
            {
                let handler = dropdown.highlighted.borrow().clone();
                if let Some(handler) = handler {
                    handler(token);
                }
            }
        });
        dropdown
    }

    /// What the bar lays out.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Return on a row, or a click on a row or a pill: its token.
    pub fn connect_run(&self, run: impl Fn(u64) + 'static) {
        self.run.replace(Some(Rc::new(run)));
    }

    /// The arrows came to rest on a row: its token.
    pub fn connect_highlighted(&self, highlighted: impl Fn(u64) + 'static) {
        self.highlighted.replace(Some(Rc::new(highlighted)));
    }

    /// Draw `view`, whole: the controller's
    /// [`postio_focus::Intent::Dropdown`].
    pub fn show(&self, view: &DropdownView) {
        let kept = self.highlighted();
        self.show_understood(&view.understood);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut tokens = Vec::new();
        for section in &view.sections {
            self.show_section(section, &mut tokens);
        }
        self.tokens.replace(tokens);
        self.show_footer(view);
        self.view.replace(Some(view.clone()));
        // A run's `select` wins; else the row the arrows rested on, while
        // it is still drawn; else the view's default.
        let at = view
            .select
            .filter(|token| self.index_of(*token).is_some())
            .or(kept.filter(|token| self.index_of(*token).is_some()))
            .or(view.highlight)
            .and_then(|token| self.index_of(token));
        self.quiet.set(true);
        match at.and_then(|at| self.list.row_at_index(at as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.quiet.set(false);
    }

    /// The token of the row the arrows rest on.
    pub fn highlighted(&self) -> Option<u64> {
        self.list
            .selected_row()
            .and_then(|row| self.token_at(row.index()))
    }

    /// Move the highlight `by` rows that run something, staying in the
    /// list at either end.
    pub fn step(&self, by: i32) {
        let tokens = self.tokens.borrow();
        let runnable: Vec<usize> = (0..tokens.len()).filter(|i| tokens[*i].is_some()).collect();
        drop(tokens);
        if runnable.is_empty() {
            return;
        }
        let now = self
            .list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
            .and_then(|index| runnable.iter().position(|i| *i == index));
        let next = match now {
            None if by > 0 => 0,
            None => runnable.len() - 1,
            Some(now) => (now as i32 + by).clamp(0, runnable.len() as i32 - 1) as usize,
        };
        if let Some(row) = self.list.row_at_index(runnable[next] as i32) {
            self.list.select_row(Some(&row));
            reveal(&self.list, &row);
        }
    }

    /// Run the highlighted row, as Return does; whether there was one.
    pub fn run_highlighted(&self) -> bool {
        match self.highlighted() {
            Some(token) => {
                self.say_run(token);
                true
            }
            None => false,
        }
    }

    /// Every word drawn, top to bottom, a row or pill to an entry.
    pub fn texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        let mut child = self.list.first_child();
        while let Some(row) = child {
            texts.push(said(&row));
            child = row.next_sibling();
        }
        texts
    }

    /// The subjects of the hits drawn, top to bottom: each hit's title is
    /// "Sender · Subject", or the subject alone when it has no sender.
    pub fn hit_subjects(&self) -> Vec<String> {
        let view = self.view.borrow();
        let Some(view) = view.as_ref() else {
            return Vec::new();
        };
        view.sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter(|row| row.kind == DropdownRowKind::Hit)
            .map(|row| {
                let title: String = row.title.iter().map(|run| run.text.as_str()).collect();
                match title.split_once(" \u{b7} ") {
                    Some((_, subject)) => subject.to_owned(),
                    None => title,
                }
            })
            .collect()
    }

    /// The section titles, in order.
    pub fn titles(&self) -> Vec<String> {
        collect(&self.list, "focus-dropdown-title")
    }

    /// The words drawn with the find highlight, read from each label's
    /// markup.
    pub fn found(&self) -> Vec<String> {
        let mut found = Vec::new();
        for label in labels(&self.list) {
            if !label.uses_markup() {
                continue;
            }
            let Ok((attributes, text, _)) = pango::parse_markup(&label.label(), '\0') else {
                continue;
            };
            for attribute in attributes.attributes() {
                if attribute.type_() == pango::AttrType::Background {
                    let (start, end) = (
                        attribute.start_index() as usize,
                        attribute.end_index() as usize,
                    );
                    if let Some(words) = text.get(start..end.min(text.len())) {
                        found.push(words.to_owned());
                    }
                }
            }
        }
        found
    }

    /// The pills' words: `from: Ada 12`, `Receipts 3`.
    pub fn pills(&self) -> Vec<String> {
        let mut pills = Vec::new();
        for button in descendants(&self.list) {
            if button.has_css_class("focus-dropdown-pill") {
                pills.push(said(&button));
            }
        }
        pills
    }

    /// Click the pill whose words start with `text`, as a person does.
    pub fn click_pill(&self, text: &str) -> bool {
        for widget in descendants(&self.list) {
            if widget.has_css_class("focus-dropdown-pill")
                && said(&widget).starts_with(text)
                && let Ok(button) = widget.downcast::<gtk::Button>()
            {
                button.emit_clicked();
                return true;
            }
        }
        false
    }

    /// The footer: its keys' words, and the count on its right.
    pub fn footer(&self) -> (Vec<String>, String) {
        let mut hints = Vec::new();
        let mut child = self.hints.first_child();
        while let Some(hint) = child {
            hints.push(said(&hint));
            child = hint.next_sibling();
        }
        (hints, self.count.text().to_string())
    }

    /// The "Understood as" tiles' words.
    pub fn understood(&self) -> Vec<String> {
        let mut tiles = Vec::new();
        let mut child = self.understood.first_child();
        while let Some(tile) = child {
            if tile.has_css_class("focus-dropdown-tile") {
                tiles.push(said(&tile));
            }
            child = tile.next_sibling();
        }
        tiles
    }

    fn token_at(&self, index: i32) -> Option<u64> {
        let index = usize::try_from(index).ok()?;
        self.tokens.borrow().get(index).copied().flatten()
    }

    fn index_of(&self, token: u64) -> Option<usize> {
        self.tokens
            .borrow()
            .iter()
            .position(|held| *held == Some(token))
    }

    fn say_run(&self, token: u64) {
        let handler = self.run.borrow().clone();
        if let Some(handler) = handler {
            handler(token);
        }
    }

    fn show_understood(&self, tiles: &[UnderstoodTile]) {
        while let Some(child) = self.understood.first_child() {
            self.understood.remove(&child);
        }
        self.understood.set_visible(!tiles.is_empty());
        if tiles.is_empty() {
            return;
        }
        let title = gtk::Label::new(Some(words::UNDERSTOOD_AS));
        title.add_css_class("focus-dropdown-title");
        self.understood.append(&title);
        for tile in tiles {
            let term = gtk::Label::new(None);
            term.set_markup(&format!(
                "<span alpha=\"60%\">{}</span>{}",
                glib::markup_escape_text(&tile.op),
                glib::markup_escape_text(&tile.value)
            ));
            term.add_css_class("focus-dropdown-term");
            term.set_xalign(0.0);
            let origin = gtk::Label::new(Some(&tile.origin));
            origin.add_css_class("focus-dropdown-origin");
            origin.add_css_class("dim-label");
            origin.set_xalign(0.0);
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
            box_.add_css_class("focus-dropdown-tile");
            box_.append(&term);
            box_.append(&origin);
            self.understood.append(&box_);
        }
        let note = gtk::Label::new(Some(words::UNDERSTOOD_NOTE));
        note.add_css_class("dim-label");
        note.set_hexpand(true);
        note.set_xalign(1.0);
        note.set_ellipsize(pango::EllipsizeMode::End);
        self.understood.append(&note);
    }

    fn show_section(&self, section: &DropdownSection, tokens: &mut Vec<Option<u64>>) {
        let narrow = section.pills.iter().any(|pill| pill.op.is_some());
        if narrow {
            // Narrow to: its title and its pills on one line.
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            line.add_css_class("focus-dropdown-narrow");
            line.append(&title_label(&section.title));
            line.append(&self.pills_box(&section.pills));
            self.append_fixed(&line, tokens);
        } else {
            if !section.title.is_empty() {
                self.append_fixed(&self.header(section), tokens);
            }
            if !section.pills.is_empty() {
                self.append_fixed(&self.pills_box(&section.pills), tokens);
            }
        }
        let sheet: Vec<&DropdownRow> = section
            .rows
            .iter()
            .filter(|row| row.kind == DropdownRowKind::CheatSheet)
            .collect();
        if !sheet.is_empty() {
            self.append_fixed(&cheat_sheet(&sheet), tokens);
        }
        for row in section.rows.iter().filter(|row| {
            !matches!(
                row.kind,
                DropdownRowKind::CheatSheet | DropdownRowKind::Example
            )
        }) {
            let widget = gtk::ListBoxRow::new();
            widget.add_css_class("focus-dropdown-row");
            if row.kind == DropdownRowKind::Person {
                widget.add_css_class("focus-dropdown-person");
            }
            widget.set_child(Some(&row_line(row)));
            widget.update_property(&[gtk::accessible::Property::Label(
                &words::dropdown_row_accessible(
                    &plain(&row.title),
                    &plain(&row.detail),
                    row.folder.as_deref(),
                    row.right.as_deref(),
                ),
            )]);
            widget.set_selectable(row.selectable);
            widget.set_activatable(row.selectable);
            self.list.append(&widget);
            tokens.push(row.selectable.then_some(row.token));
        }
        if let Some(example) = section
            .rows
            .iter()
            .find(|row| row.kind == DropdownRowKind::Example)
        {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            line.add_css_class("focus-dropdown-example");
            line.append(&runs_label(&example.title, true));
            line.append(&runs_label(&example.detail, true));
            self.append_fixed(&line, tokens);
        }
    }

    /// A row that runs nothing: a header, the pills, the sheet.
    fn append_fixed(&self, child: &impl IsA<gtk::Widget>, tokens: &mut Vec<Option<u64>>) {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("focus-dropdown-fixed");
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_focusable(false);
        row.set_child(Some(child));
        self.list.append(&row);
        tokens.push(None);
    }

    fn header(&self, section: &DropdownSection) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        line.add_css_class("focus-dropdown-header");
        let title = title_label(&section.title);
        title.set_hexpand(true);
        line.append(&title);
        if let Some(key) = &section.note_key {
            line.append(&keyhint::cap(key));
        }
        if let Some(note) = &section.note {
            let note = gtk::Label::new(Some(note));
            note.add_css_class("focus-dropdown-note");
            note.add_css_class("dim-label");
            note.set_ellipsize(pango::EllipsizeMode::End);
            line.append(&note);
        }
        line
    }

    fn pills_box(&self, pills: &[DropdownPill]) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        line.add_css_class("focus-dropdown-pills");
        for pill in pills {
            let said = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            if let Some(op) = &pill.op {
                let op = gtk::Label::new(Some(op));
                op.add_css_class("focus-dropdown-op");
                said.append(&op);
            }
            let label = gtk::Label::new(Some(&pill.label));
            label.add_css_class("focus-dropdown-pill-label");
            said.append(&label);
            if let Some(count) = &pill.count {
                let count = gtk::Label::new(Some(count));
                count.add_css_class("dim-label");
                count.add_css_class("focus-dropdown-pill-count");
                said.append(&count);
            }
            if let Some(fresh) = &pill.fresh {
                let fresh = gtk::Label::new(Some(fresh));
                fresh.add_css_class("focus-dropdown-fresh");
                said.append(&fresh);
            }
            if let Some(key) = &pill.key {
                said.append(&keyhint::cap(key));
            }
            let button = gtk::Button::new();
            button.add_css_class("focus-dropdown-pill");
            button.set_valign(gtk::Align::Center);
            if pill.op.is_some() {
                button.add_css_class("focus-dropdown-pill-narrow");
            }
            button.set_child(Some(&said));
            button.set_focus_on_click(false);
            let token = pill.token;
            let me = self.me.borrow().clone();
            button.connect_clicked(move |_| {
                if let Some(dropdown) = me.upgrade() {
                    dropdown.say_run(token);
                }
            });
            line.append(&button);
        }
        line
    }

    fn show_footer(&self, view: &DropdownView) {
        while let Some(child) = self.hints.first_child() {
            self.hints.remove(&child);
        }
        for hint in &view.hints {
            // The cap, then what it does, on one line, as the Mac's footer.
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            line.add_css_class("focus-dropdown-hint");
            line.append(&keyhint::cap(&hint.key));
            let words = gtk::Label::new(Some(&hint.label));
            words.set_single_line_mode(true);
            words.add_css_class("dim-label");
            line.append(&words);
            self.hints.append(&line);
        }
        self.count
            .set_text(view.count.as_deref().unwrap_or_default());
    }
}

/// Scroll `list` so `row` is wholly in view, no further: the keyboard
/// stays in the bar's entry, so the list never follows its selection.
fn reveal(list: &gtk::ListBox, row: &gtk::ListBoxRow) {
    let Some(scroller) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    else {
        return;
    };
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let adjustment = scroller.vadjustment();
    let (top, bottom) = (
        f64::from(bounds.y()),
        f64::from(bounds.y() + bounds.height()),
    );
    let (shown, page) = (adjustment.value(), adjustment.page_size());
    if top < shown {
        adjustment.set_value(top);
    } else if bottom > shown + page {
        adjustment.set_value(bottom - page);
    }
}

/// A section's bold secondary title.
fn title_label(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.add_css_class("focus-dropdown-title");
    label.set_xalign(0.0);
    label.set_accessible_role(gtk::AccessibleRole::Heading);
    label
}

/// One row: what it is, its words, the folder, the right column, its key.
fn row_line(row: &DropdownRow) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    line.append(&leading(row));
    let words_box = if row.kind == DropdownRowKind::Person {
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stack.append(&runs_label(&row.title, false));
        let detail = runs_label(&row.detail, true);
        detail.add_css_class("focus-dropdown-detail");
        stack.append(&detail);
        stack
    } else {
        let flow = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let title = runs_label(&row.title, false);
        title.add_css_class("focus-dropdown-row-title");
        flow.append(&title);
        if !row.detail.is_empty() {
            let detail = runs_label(&row.detail, true);
            detail.add_css_class("focus-dropdown-detail");
            detail.set_hexpand(true);
            flow.append(&detail);
        }
        flow
    };
    words_box.set_hexpand(true);
    line.append(&words_box);
    if row.kind != DropdownRowKind::ShowAll {
        let folder = gtk::Label::new(row.folder.as_deref());
        folder.add_css_class("focus-dropdown-folder");
        folder.set_width_request(FOLDER_WIDTH);
        folder.set_xalign(0.0);
        folder.set_ellipsize(pango::EllipsizeMode::End);
        line.append(&folder);
    }
    let right = gtk::Box::new(gtk::Orientation::Horizontal, S1);
    right.set_width_request(RIGHT_WIDTH);
    right.set_halign(gtk::Align::End);
    right.set_hexpand(false);
    let said = gtk::Label::new(row.right.as_deref());
    said.add_css_class("focus-dropdown-right");
    said.add_css_class("dim-label");
    // Fills the column, which does not expand: the column refuses to hand
    // the expansion up to the row, where it would split the spare width
    // with the words and pull the folder column off its place.
    said.set_hexpand(true);
    said.set_xalign(1.0);
    right.append(&said);
    if let Some(hint) = &row.highlighted {
        // "↩ adds" while the arrows rest here (screen 04); CSS shows it
        // on the selected row only, in place of the date.
        let adds = gtk::Label::new(Some(&format!("{} {}", hint.key, hint.label)));
        adds.add_css_class("focus-dropdown-adds");
        right.append(&adds);
    }
    if let Some(key) = &row.key {
        right.append(&keyhint::cap(key));
    }
    line.append(&right);
    line
}

/// The row's kind, drawn: an avatar, a tile, or a quiet icon.
fn leading(row: &DropdownRow) -> gtk::Widget {
    match row.kind {
        DropdownRowKind::Person => {
            let initials = gtk::Label::new(Some(row.initials.as_deref().unwrap_or("?")));
            initials.add_css_class("focus-dropdown-avatar");
            initials.set_accessible_role(gtk::AccessibleRole::Presentation);
            initials.upcast()
        }
        DropdownRowKind::Word => {
            let tile = gtk::Label::new(Some("Aa"));
            tile.add_css_class("focus-dropdown-tile-icon");
            tile.set_accessible_role(gtk::AccessibleRole::Presentation);
            tile.upcast()
        }
        // The theme has no plain envelope, only the read and unread
        // marks, which would say something about the hit it does not
        // mean: a hit keeps the column, empty, so the words align.
        DropdownRowKind::Hit => {
            let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            room.add_css_class("focus-dropdown-icon");
            room.upcast()
        }
        kind => {
            let image = gtk::Image::from_icon_name(icon(kind));
            image.add_css_class("focus-dropdown-icon");
            image.set_accessible_role(gtk::AccessibleRole::Presentation);
            image.upcast()
        }
    }
}

/// The symbolic icon for a row of `kind`.
fn icon(kind: DropdownRowKind) -> &'static str {
    match kind {
        DropdownRowKind::Recent => "document-open-recent-symbolic",
        DropdownRowKind::ShowAll => "view-list-symbolic",
        DropdownRowKind::Label => "tag-symbolic",
        DropdownRowKind::List => "view-list-bullet-symbolic",
        DropdownRowKind::File => "text-x-generic-symbolic",
        DropdownRowKind::Folder => "folder-symbolic",
        _ => "system-search-symbolic",
    }
}

/// The cheat sheet: an operator and what it is for, four to a line.
fn cheat_sheet(rows: &[&DropdownRow]) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("focus-dropdown-sheet");
    grid.set_column_spacing(S3 as u32);
    grid.set_column_homogeneous(true);
    for (index, row) in rows.iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        cell.append(&runs_label(&row.title, false));
        cell.append(&runs_label(&row.detail, true));
        let index = index as i32;
        grid.attach(&cell, index % SHEET_COLUMNS, index / SHEET_COLUMNS, 1, 1);
    }
    grid
}

/// `runs` in one label: bold, mono and the find highlight by attributes,
/// so the words wrap and ellipsize as one line.
pub(crate) fn runs_label(runs: &[Run], secondary: bool) -> gtk::Label {
    let (colour, alpha) = if adw::StyleManager::default().is_dark() {
        FIND_DARK
    } else {
        FIND_LIGHT
    };
    let mut markup = String::new();
    for run in runs {
        let text = glib::markup_escape_text(&run.text);
        let mut open = String::new();
        match run.style {
            RunStyle::Strong => open.push_str(" weight=\"semibold\""),
            RunStyle::Mono => open.push_str(" font_family=\"monospace\""),
            _ => {}
        }
        if run.highlighted {
            open.push_str(&format!(" background=\"{colour}\" bgalpha=\"{alpha}\""));
        }
        if open.is_empty() {
            markup.push_str(&text);
        } else {
            markup.push_str(&format!("<span{open}>{text}</span>"));
        }
    }
    let label = gtk::Label::new(None);
    label.set_markup(&markup);
    label.set_xalign(0.0);
    label.set_single_line_mode(true);
    label.set_ellipsize(pango::EllipsizeMode::End);
    if secondary {
        label.add_css_class("focus-dropdown-secondary");
    }
    label
}

/// `runs`' words, without their styling.
fn plain(runs: &[Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// Every label under `root`, depth first.
fn labels(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Label> {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .collect()
}

/// The words of the labels under `root` carrying `class`.
fn collect(root: &impl IsA<gtk::Widget>, class: &str) -> Vec<String> {
    labels(root)
        .into_iter()
        .filter(|label| label.has_css_class(class))
        .map(|label| label.text().to_string())
        .collect()
}

fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        let mut child = widget.last_child();
        while let Some(next) = child {
            stack.push(next.clone());
            child = next.prev_sibling();
        }
        found.push(widget);
    }
    found
}

/// The words a widget shows, its labels' texts joined by spaces.
fn said(widget: &impl IsA<gtk::Widget>) -> String {
    labels(widget)
        .into_iter()
        .filter(|label| label.is_visible())
        .map(|label| label.text().to_string())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
