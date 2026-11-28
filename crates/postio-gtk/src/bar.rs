//! The command bar: `/` or `Ctrl K` opens one bar for search, places and commands. The controller
//! (`postio_focus`) owns what it says;
//! this draws it and hands back what a person does. The arrows' highlight is the toolkit's; the
//! view says only where a run or landing moved it.
//! Everything typed stays local.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_focus::{BarLine, BarLineKind, BarView};
use postio_model::AccountScope;
use postio_ui::command_bar::{self as rules, SAVED};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

const WIDTH: i32 = 860;
const FIELD_WIDTH: i32 = 480;

const BLEND_PLACEHOLDER: &str = "Search mail, go to a folder, or run a command";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarInput {
    Typed(String),
    Run(u64),
    Command(CommandId),
    Highlighted(u64),
}

type Handler = Rc<dyn Fn(BarInput)>;
type TabHandler = Rc<dyn Fn() -> bool>;

pub struct Bar {
    root: gtk::Box,
    over: gtk::Overlay,
    input: gtk::Box,
    scrim: gtk::Box,
    entry: gtk::Entry,
    ghost: gtk::Label,
    ghost_hint: gtk::Box,
    echo: gtk::Label,
    list: gtk::ListBox,
    keymap: RefCell<Keymap>,
    lines: RefCell<Vec<BarLine>>,
    view: RefCell<BarView>,
    open: Cell<bool>,
    /// Set while the bar puts the controller's own words in the field, so they are not said back as
    /// typing.
    quiet: Cell<bool>,
    places_known: Cell<bool>,
    field: RefCell<Option<gtk::Widget>>,
    handler: RefCell<Option<Handler>>,
    tab: RefCell<Option<TabHandler>>,
    me: RefCell<std::rc::Weak<Bar>>,
    saved_row: gtk::Box,
    home: RefCell<Option<Rc<dyn Fn()>>>,
    blend: gtk::Box,
    dropdown: Rc<crate::dropdown::Dropdown>,
    dropping: Cell<bool>,
}

impl Bar {
    pub fn controls() -> Vec<CommandId> {
        SAVED
            .into_iter()
            .chain([
                CommandId::OpenMessage,
                CommandId::ToggleResultOrder,
                CommandId::ShowAllResults,
                CommandId::ForgetRecent,
                CommandId::ExcludeSuggestion,
            ])
            .collect()
    }

    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let icon = gtk::Image::from_icon_name("system-search-symbolic");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_has_frame(false);
        entry.set_placeholder_text(Some(BLEND_PLACEHOLDER));
        entry.add_css_class("focus-bar-entry");
        let input = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        input.add_css_class("focus-bar-input");
        input.append(&icon);
        // The ghost sits over the entry and takes no pointer: a click lands on the entry under it.
        let ghost = gtk::Label::new(None);
        ghost.add_css_class("focus-bar-ghost");
        ghost.add_css_class("dim-label");
        ghost.set_halign(gtk::Align::Start);
        ghost.set_valign(gtk::Align::Center);
        ghost.set_can_target(false);
        ghost.set_visible(false);
        let ghost_hint = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        ghost_hint.add_css_class("focus-bar-ghost-hint");
        ghost_hint.set_halign(gtk::Align::End);
        ghost_hint.set_margin_end(S2);
        ghost_hint.set_valign(gtk::Align::Center);
        ghost_hint.set_can_target(false);
        ghost_hint.set_visible(false);
        let field = gtk::Overlay::new();
        field.set_hexpand(true);
        field.set_child(Some(&entry));
        field.add_overlay(&ghost);
        field.add_overlay(&ghost_hint);
        input.append(&field);
        if let Some(key) = postio_ui::hints::key(keymap, CommandId::Back) {
            input.append(&keyhint::cap(&key));
        }

        let echo = gtk::Label::new(None);
        echo.add_css_class("dim-label");
        echo.add_css_class("focus-bar-echo");
        echo.set_xalign(0.0);
        echo.set_visible(false);
        let list = gtk::ListBox::new();
        list.add_css_class("focus-bar-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(520)
            .build();
        let footer = gtk::Label::new(Some(
            "\u{2191}\u{2193} move \u{b7} \u{21b5} open / run \u{b7} > commands only \u{b7} Local index",
        ));
        footer.add_css_class("dim-label");
        footer.add_css_class("focus-bar-footer");
        footer.set_xalign(0.0);

        let saved_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        saved_row.add_css_class("focus-bar-saved");
        saved_row.set_visible(false);
        input.set_halign(gtk::Align::Center);
        input.set_width_request(FIELD_WIDTH);
        let blend = gtk::Box::new(gtk::Orientation::Vertical, S2);
        blend.append(&saved_row);
        blend.append(&echo);
        blend.append(&scrolled);
        blend.append(&footer);
        let dropdown = crate::dropdown::Dropdown::new();
        dropdown.widget().set_visible(false);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, S2);
        panel.add_css_class("focus-bar-panel");
        panel.append(&blend);
        panel.append(dropdown.widget());
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-bar");
        root.set_width_request(WIDTH);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Start);
        root.append(&input);
        root.append(&panel);
        // Paint only: it takes no pointer, so the list under it behaves as before.
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("focus-bar-scrim");
        scrim.set_can_target(false);
        let over = gtk::Overlay::new();
        over.set_child(Some(&scrim));
        over.add_overlay(&root);
        over.set_visible(false);

        let bar = Rc::new(Bar {
            root,
            over,
            input: input.clone(),
            scrim: scrim.clone(),
            entry,
            ghost,
            ghost_hint,
            echo,
            list,
            keymap: RefCell::new(keymap.clone()),
            lines: RefCell::default(),
            view: RefCell::default(),
            open: Cell::new(false),
            quiet: Cell::new(false),
            places_known: Cell::new(false),
            field: RefCell::default(),
            handler: RefCell::default(),
            tab: RefCell::default(),
            me: RefCell::default(),
            saved_row,
            home: RefCell::default(),
            blend,
            dropdown,
            dropping: Cell::new(false),
        });
        bar.me.replace(Rc::downgrade(&bar));
        let weak = Rc::downgrade(&bar);
        // A press on the scrim, outside the bar, closes it as Escape does rather than reaching the
        // list.
        let outside = gtk::GestureClick::new();
        outside.connect_pressed({
            let weak = weak.clone();
            move |_, _, x, y| {
                let Some(bar) = weak.upgrade() else {
                    return;
                };
                let picked = bar.over.pick(x, y, gtk::PickFlags::DEFAULT);
                let inside = picked.is_some_and(|widget| {
                    widget == *bar.root.upcast_ref::<gtk::Widget>() || widget.is_ancestor(&bar.root)
                });
                if !inside {
                    bar.dismiss();
                }
            }
        });
        bar.over.add_controller(outside);
        bar.dropdown.connect_run({
            let weak = weak.clone();
            move |token| {
                if let Some(bar) = weak.upgrade() {
                    bar.say(BarInput::Run(token));
                }
            }
        });
        bar.dropdown.connect_highlighted({
            let weak = weak.clone();
            move |token| {
                if let Some(bar) = weak.upgrade() {
                    bar.say(BarInput::Highlighted(token));
                }
            }
        });
        bar.entry.connect_changed({
            let weak = weak.clone();
            move |entry| {
                if let Some(bar) = weak.upgrade() {
                    bar.ghost.set_text("");
                    bar.ghost.set_visible(false);
                    bar.dress_terms();
                }
                if let Some(bar) = weak.upgrade()
                    && !bar.quiet.get()
                {
                    bar.say(BarInput::Typed(entry.text().to_string()));
                }
            }
        });
        bar.entry.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(bar) = weak.upgrade() {
                    bar.run_selected();
                }
            }
        });
        bar.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let (Some(bar), Ok(index)) = (weak.upgrade(), usize::try_from(row.index())) {
                    bar.run(index);
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, state| match weak.upgrade() {
                Some(bar) if bar.press(key, state) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        bar.entry.add_controller(keys);
        bar
    }

    /// The field is hidden (still laid out) while the bar is open; the input stands in for it.
    pub fn set_field(&self, field: gtk::Widget) {
        field.set_opacity(if self.open.get() { 0.0 } else { 1.0 });
        self.field.replace(Some(field));
    }

    fn show_field(&self, shown: bool) {
        if let Some(field) = self.field.borrow().as_ref() {
            field.set_opacity(if shown { 1.0 } else { 0.0 });
        }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.over
    }

    pub fn connect_input(&self, handler: impl Fn(BarInput) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// `false` when `Tab` is the toolkit's key, with no chip to step into.
    pub fn connect_tab(&self, tab: impl Fn() -> bool + 'static) {
        self.tab.replace(Some(Rc::new(tab)));
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show_saved();
    }

    pub fn set_home(&self, home: impl Fn() + 'static) {
        self.home.replace(Some(Rc::new(home)));
    }

    pub fn open_on(&self, text: &str, select: Option<(u32, u32)>) {
        if !self.open.get() {
            self.open.set(true);
            self.places_known.set(false);
            self.over.set_visible(true);
            self.show_field(false);
        }
        if self.entry.text() != text {
            self.quiet.set(true);
            self.entry.set_text(text);
            self.quiet.set(false);
        }
        self.entry.grab_focus();
        match select {
            Some((start, end)) => self.entry.select_region(start as i32, end as i32),
            None => self.entry.set_position(-1),
        }
    }

    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        // A keyboard left in a hidden entry would make every single-letter key typing. Handed over
        // once the bar is down, so hiding
        // it cannot move focus again; a keyboard on nothing goes home too, since on the window
        // itself `j` and Escape reach nothing.
        let had_keyboard = self.entry.root().filter(|root| {
            root.focus().is_none_or(|focus| {
                focus.is_ancestor(&self.root)
                    || focus == self.root
                    || focus == *root.upcast_ref::<gtk::Widget>()
            })
        });
        self.over.set_visible(false);
        self.show_field(true);
        self.quiet.set(true);
        self.entry.set_text("");
        self.quiet.set(false);
        self.view.replace(BarView::default());
        self.drop_down(false);
        self.dropdown.clear();
        self.lines.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        if let Some(root) = had_keyboard {
            let home = self.home.borrow().clone();
            match home {
                Some(home) => home(),
                None => root.set_focus(None::<&gtk::Widget>),
            }
        }
    }

    /// Closed by the toolkit (a click outside); the controller hears it as Back.
    fn dismiss(&self) {
        self.say(BarInput::Command(CommandId::Back));
    }

    pub fn places_read(&self) {
        if self.open.get() {
            self.places_known.set(true);
        }
    }

    pub fn show(&self, view: &BarView) {
        self.drop_down(false);
        self.show_ghost(None, None);
        let kept = self.highlighted_line();
        self.echo.set_visible(view.echo.is_some());
        self.echo.set_text(view.echo.as_deref().unwrap_or_default());
        // The same lines under new tokens keep their rows: only what runs them changes.
        let same_words = {
            let lines = self.lines.borrow();
            lines.len() == view.lines.len()
                && lines.iter().zip(&view.lines).all(|(was, now)| {
                    BarLine {
                        token: now.token,
                        ..was.clone()
                    } == *now
                })
        };
        if same_words {
            self.lines.replace(view.lines.clone());
        } else {
            while let Some(child) = self.list.first_child() {
                self.list.remove(&child);
            }
            for line in &view.lines {
                self.list.append(&self.line_row(line));
            }
            self.lines.replace(view.lines.clone());
        }
        let saved_changed = self.view.borrow().saved != view.saved;
        self.view.replace(view.clone());
        if saved_changed {
            self.show_saved();
        }
        // Where a run or landing put it; else where the arrows left it while that line is still
        // drawn; else the first line that runs.
        let lines = self.lines.borrow();
        let at = view
            .highlight
            .and_then(|token| lines.iter().position(|line| line.token == token))
            .or_else(|| {
                kept.as_ref()
                    .and_then(|kept| lines.iter().position(|line| same_line(line, kept)))
            })
            .or_else(|| lines.iter().position(|line| line.selectable));
        drop(lines);
        match at.and_then(|at| self.list.row_at_index(at as i32)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                self.reveal(&row);
            }
            None => self.list.unselect_all(),
        }
    }

    pub fn show_dropdown(&self, view: &postio_focus::DropdownView) {
        self.drop_down(true);
        self.dropdown.show(view);
        self.dress_terms();
        self.show_ghost(view.ghost.as_deref(), view.ghost_hint.as_ref());
    }

    /// The text stays the text, so typing and the caret are the entry's.
    fn dress_terms(&self) {
        let attributes = gtk::pango::AttrList::new();
        if self.dropping.get() {
            let text = self.entry.text();
            let today = postio_ui::clock::now().date_naive();
            for chip in postio_ui::search_view::field_chips(&text, today) {
                let (start, end, operator_end) =
                    (chip.start as u32, chip.end as u32, chip.operator_end as u32);
                let mut ground = gtk::pango::AttrColor::new_background(0x8000, 0x8000, 0x8000);
                ground.set_start_index(start);
                ground.set_end_index(end);
                attributes.insert(ground);
                let mut faint = gtk::pango::AttrInt::new_background_alpha(0x2400);
                faint.set_start_index(start);
                faint.set_end_index(end);
                attributes.insert(faint);
                let mut mono = gtk::pango::AttrString::new_family("Adwaita Mono");
                mono.set_start_index(start);
                mono.set_end_index(operator_end);
                attributes.insert(mono);
                if chip.excluded {
                    let mut struck = gtk::pango::AttrInt::new_strikethrough(true);
                    struck.set_start_index(operator_end);
                    struck.set_end_index(end);
                    attributes.insert(struck);
                }
            }
        }
        self.entry.set_attributes(&attributes);
    }

    pub fn chip_ranges(&self) -> Vec<(u32, u32, bool)> {
        let Some(attributes) = self.entry.attributes() else {
            return Vec::new();
        };
        let struck: Vec<(u32, u32)> = attributes
            .attributes()
            .iter()
            .filter(|attr| attr.type_() == gtk::pango::AttrType::Strikethrough)
            .map(|attr| (attr.start_index(), attr.end_index()))
            .collect();
        attributes
            .attributes()
            .iter()
            .filter(|attr| attr.type_() == gtk::pango::AttrType::Background)
            .map(|attr| {
                let (start, end) = (attr.start_index(), attr.end_index());
                let excluded = struck.iter().any(|(from, to)| *from > start && *to == end);
                (start, end, excluded)
            })
            .collect()
    }

    /// Placed once the entry has its size, measured in the entry's own font.
    fn show_ghost(&self, ghost: Option<&str>, hint: Option<&postio_ui::hints::Hint>) {
        while let Some(child) = self.ghost_hint.first_child() {
            self.ghost_hint.remove(&child);
        }
        if let Some(hint) = hint.filter(|_| ghost.is_some()) {
            self.ghost_hint.append(&keyhint::cap(&hint.key));
            let words = gtk::Label::new(Some(&hint.label));
            words.add_css_class("dim-label");
            self.ghost_hint.append(&words);
        }
        self.ghost_hint
            .set_visible(ghost.is_some() && self.ghost_hint.first_child().is_some());
        self.ghost.set_text(ghost.unwrap_or_default());
        self.ghost.set_visible(false);
        if ghost.is_none() {
            return;
        }
        let me = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(bar) = me.upgrade() {
                bar.place_ghost();
            }
        });
    }

    fn place_ghost(&self) {
        if self.ghost.text().is_empty() {
            return;
        }
        // A view can land before the first frame, when the field has no width and the ghost would
        // stay hidden: place it on the first laid-out frame (#2151).
        let Some(field) = self.ghost.parent() else {
            return;
        };
        if field.width() == 0 {
            let me = self.me.borrow().clone();
            field.add_tick_callback(move |field, _| {
                if field.width() == 0 {
                    return glib::ControlFlow::Continue;
                }
                if let Some(bar) = me.upgrade() {
                    bar.place_ghost();
                }
                glib::ControlFlow::Break
            });
            return;
        }
        let Some(end) = self.typed_end() else {
            return;
        };
        let room = self.ghost.parent().map_or(0, |field| field.width()) as f32 - end;
        let (wide, _) = self.ghost.layout().pixel_size();
        self.ghost.set_margin_start(end.round() as i32);
        self.ghost.set_visible(wide as f32 <= room);
    }

    fn typed_end(&self) -> Option<f32> {
        let text = self.entry.delegate()?;
        let field = self.ghost.parent()?;
        let start = text.compute_bounds(&field)?.x();
        let (wide, _) = text
            .create_pango_layout(Some(&self.entry.text()))
            .pixel_size();
        Some(start + wide as f32)
    }

    pub fn ghost(&self) -> Option<String> {
        (self.ghost.is_visible() && !self.ghost.text().is_empty())
            .then(|| self.ghost.text().to_string())
    }

    pub fn ghost_at(&self) -> Option<(f32, f32)> {
        let at = self.ghost.compute_bounds(&self.ghost.parent()?)?.x();
        Some((at, self.typed_end()?))
    }

    pub fn dropdown(&self) -> &crate::dropdown::Dropdown {
        &self.dropdown
    }

    fn drop_down(&self, dropping: bool) {
        self.dropping.set(dropping);
        self.dropdown.widget().set_visible(dropping);
        self.blend.set_visible(!dropping);
        self.input
            .set_width_request(if dropping { WIDTH } else { FIELD_WIDTH });
        self.entry.set_placeholder_text(Some(if dropping {
            postio_ui::search_view::PLACEHOLDER
        } else {
            BLEND_PLACEHOLDER
        }));
        if dropping {
            self.scrim.add_css_class("focus-bar-scrim-clear");
        } else {
            self.scrim.remove_css_class("focus-bar-scrim-clear");
        }
    }

    pub fn dims_the_list(&self) -> bool {
        self.over.is_visible() && !self.scrim.has_css_class("focus-bar-scrim-clear")
    }

    fn show_saved(&self) {
        while let Some(child) = self.saved_row.first_child() {
            self.saved_row.remove(&child);
        }
        let saved = self.view.borrow().saved.clone();
        self.saved_row.set_visible(!saved.is_empty());
        if saved.is_empty() {
            return;
        }
        let title = gtk::Label::new(Some("Saved"));
        title.add_css_class("dim-label");
        self.saved_row.append(&title);
        let keymap = self.keymap.borrow();
        for (index, name) in saved.iter().enumerate() {
            let said = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            said.append(&gtk::Label::new(Some(name)));
            let command = SAVED.get(index).copied();
            if let Some(key) = command.and_then(|command| postio_ui::hints::key(&keymap, command)) {
                said.append(&keyhint::cap(&key));
            }
            let pill = gtk::Button::new();
            pill.add_css_class("focus-bar-saved-pill");
            pill.set_child(Some(&said));
            pill.set_focus_on_click(false);
            if let Some(command) = command {
                let weak = self.me.borrow().clone();
                pill.connect_clicked(move |_| {
                    if let Some(bar) = weak.upgrade() {
                        bar.say(BarInput::Command(command));
                    }
                });
            }
            self.saved_row.append(&pill);
        }
    }

    fn line_row(&self, line: &BarLine) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        match line.kind {
            BarLineKind::Heading => {
                let label = gtk::Label::new(Some(&line.title));
                label.add_css_class("focus-bar-section");
                label.set_xalign(0.0);
                row.set_child(Some(&label));
            }
            BarLineKind::Hint | BarLineKind::Command | BarLineKind::Correspondent => {
                row.add_css_class("focus-bar-row");
                row.set_child(Some(&plain_line(line)));
            }
        }
        if !line.selectable {
            row.set_selectable(false);
            row.set_activatable(false);
        }
        row
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    pub fn places_known(&self) -> bool {
        self.places_known.get()
    }

    /// `true` when the bar used the key.
    pub fn press(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        match key {
            gtk::gdk::Key::Down if self.dropping.get() => self.dropdown.step(1),
            gtk::gdk::Key::Up if self.dropping.get() => self.dropdown.step(-1),
            gtk::gdk::Key::Down => self.step(1),
            gtk::gdk::Key::Up => self.step(-1),
            gtk::gdk::Key::Tab if state.is_empty() => {
                let tab = self.tab.borrow().clone();
                return tab.is_some_and(|tab| tab());
            }
            // Typing wins: `O` is a letter for the entry; the order key carries `alt` and is the
            // window's.
            _ => return false,
        }
        true
    }

    pub fn run_search_row(&self) {
        self.run_selected();
    }

    pub fn input(&self) -> gtk::Widget {
        self.entry.clone().upcast()
    }

    pub fn typed(&self) -> String {
        self.entry.text().to_string()
    }

    pub fn query(&self) -> String {
        self.entry.text().trim().to_owned()
    }

    pub fn set_text(&self, text: &str) {
        self.entry.set_text(text);
        self.entry.set_position(-1);
    }

    pub fn commands(&self) -> Vec<CommandId> {
        let state = postio_core::Availability {
            scope: AccountScope::Unified,
            store_open: true,
            frontend: Frontend::Focus,
        };
        let keymap = self.keymap.borrow();
        let mut entries = postio_ui::palette::entries(&keymap, Context::List, state, "");
        rules::add_account_verbs(&mut entries, &keymap, state, "");
        entries
            .into_iter()
            .map(|entry| match entry.id {
                ActionId::Builtin(command) => command,
            })
            .collect()
    }

    pub fn result_subjects(&self) -> Vec<String> {
        if self.dropping.get() {
            return self.dropdown.hit_subjects();
        }
        Vec::new()
    }

    pub fn highlighted(&self) -> Option<(&'static str, String)> {
        if self.dropping.get() {
            use postio_focus::DropdownRowKind as Kind;
            let (kind, text) = self.dropdown.highlighted_row()?;
            let kind = match kind {
                Kind::Recent => "recent",
                Kind::Hit => "message",
                Kind::ShowAll => "show_all",
                Kind::CheatSheet | Kind::Example => "hint",
                Kind::Word => "word",
                Kind::Label => "label",
                Kind::List => "list",
                Kind::File => "file",
                Kind::Person => "person",
                Kind::Folder => "folder",
                _ => "other",
            };
            return Some((kind, text));
        }
        let selected = self.list.selected_row()?;
        let index = usize::try_from(selected.index()).ok()?;
        let line = self.lines.borrow().get(index)?.clone();
        let kind = match line.kind {
            BarLineKind::Heading => "heading",
            BarLineKind::Hint => "hint",
            BarLineKind::Command => "command",
            BarLineKind::Correspondent => "correspondent",
        };
        Some((kind, said(selected.upcast_ref())))
    }

    pub fn hints(&self) -> usize {
        self.lines
            .borrow()
            .iter()
            .filter(|line| line.kind == BarLineKind::Hint)
            .count()
    }

    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                let text = label.text().to_string();
                if !text.is_empty() {
                    said.push(text);
                }
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        said
    }

    fn say(&self, input: BarInput) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(input);
        }
    }

    fn highlighted_line(&self) -> Option<BarLine> {
        let selected = self.list.selected_row()?;
        let index = usize::try_from(selected.index()).ok()?;
        self.lines.borrow().get(index).cloned()
    }

    fn step(&self, by: i32) {
        let count = self.lines.borrow().len() as i32;
        let mut at = self.list.selected_row().map_or(-1, |row| row.index());
        loop {
            at += by;
            if at < 0 || at >= count {
                return;
            }
            if self.lines.borrow()[at as usize].selectable {
                break;
            }
        }
        if let Some(row) = self.list.row_at_index(at) {
            self.list.select_row(Some(&row));
            self.reveal(&row);
        }
    }

    /// The keyboard stays in the entry while the arrows walk the results, so the list never scrolls
    /// to its selection by itself.
    fn reveal(&self, row: &gtk::ListBoxRow) {
        let Some(scroller) = self
            .list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        let Some(bounds) = row.compute_bounds(&self.list) else {
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

    fn run_selected(&self) {
        if self.dropping.get() {
            self.dropdown.run_highlighted();
            return;
        }
        if let Some(row) = self.list.selected_row()
            && let Ok(index) = usize::try_from(row.index())
        {
            self.run(index);
        }
    }

    fn run(&self, index: usize) {
        let Some(line) = self.lines.borrow().get(index).cloned() else {
            return;
        };
        if line.selectable {
            self.say(BarInput::Run(line.token));
        }
    }
}

fn same_line(line: &BarLine, was: &BarLine) -> bool {
    line.token == was.token || (line.kind == was.kind && line.title == was.title)
}

fn said(widget: &gtk::Widget) -> String {
    let mut said = Vec::new();
    let mut pending = vec![widget.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && widget.is_visible()
            && !label.text().is_empty()
        {
            said.push(label.text().to_string());
        }
        let mut child = widget.last_child();
        while let Some(next) = child {
            child = next.prev_sibling();
            pending.push(next);
        }
    }
    said.join(" ")
}

fn plain_line(line: &BarLine) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    let name = gtk::Label::new(Some(&line.title));
    name.set_xalign(0.0);
    row.append(&name);
    if let Some(detail) = &line.detail {
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("dim-label");
        detail.set_xalign(0.0);
        detail.set_hexpand(true);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&detail);
    } else {
        name.set_hexpand(true);
    }
    if let Some(key) = &line.key {
        row.append(&keyhint::cap(key));
    }
    row
}
