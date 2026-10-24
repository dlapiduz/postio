//! The command bar (screens 07-09; contracts/focus-surface.md, "The command
//! bar"): `/` or `Ctrl K` opens one bar for search, places and commands.
//!
//! What the bar says is the controller's (`postio_focus`, spec 009 slice
//! 8): what is typed, the lines it answers with, which of them runs what,
//! the chips the words were lowered to, and which results are current. This
//! draws [`BarView`] and hands back what a person does -- the words as they
//! change, Return or a click on a line, `Tab` into the chips, a saved
//! search's pill -- through [`Bar::connect_input`]. The arrows' highlight is
//! the toolkit's, as on the Mac: the view says only where a run or a
//! landing moved it.
//!
//! Everything here is local. Nothing typed leaves the machine.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_focus::{BarLine, BarLineKind, BarView};
use postio_model::{AccountScope, MessageId};
use postio_ui::command_bar::{self as rules, SAVED};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The bar's width, as the screens draw it.
const WIDTH: i32 = 860;
/// The top bar's field, which the bar's input takes the place of.
const FIELD_WIDTH: i32 = 480;

/// What a person did in the bar, for the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarInput {
    /// The field holds these words now.
    Typed(String),
    /// Return on the highlighted line, or a click on one: its token.
    Run(u64),
    /// A saved search's pill, or another command the bar has a control for.
    Command(CommandId),
}

type Handler = Rc<dyn Fn(BarInput)>;
type TabHandler = Rc<dyn Fn() -> bool>;

/// The bar.
pub struct Bar {
    root: gtk::Box,
    /// The bar over its scrim: what the window lays over the list.
    over: gtk::Overlay,
    entry: gtk::Entry,
    chips: gtk::Box,
    echo: gtk::Label,
    heading: gtk::Label,
    list: gtk::ListBox,
    keymap: RefCell<Keymap>,
    /// The lines drawn now, one per row of `list`.
    lines: RefCell<Vec<BarLine>>,
    /// The view drawn now.
    view: RefCell<BarView>,
    /// The messages the last lines with any listed, and their subjects:
    /// kept when the bar closes, since a hit opens as it does, and `j`/`k`
    /// step to the others after.
    found: RefCell<Vec<(MessageId, String)>>,
    open: Cell<bool>,
    /// Set while the bar puts the controller's own words in the field, so
    /// they are not said back as typing.
    quiet: Cell<bool>,
    /// Whether the places have been read since the bar last opened.
    places_known: Cell<bool>,
    /// The top bar's field, which the bar's input stands in for while open.
    field: RefCell<Option<gtk::Widget>>,
    handler: RefCell<Option<Handler>>,
    tab: RefCell<Option<TabHandler>>,
    /// This bar, weakly: what a signal comes back to.
    me: RefCell<std::rc::Weak<Bar>>,
    /// The saved row across the top.
    saved_row: gtk::Box,
    /// Where the keyboard goes when the bar closes: the list, once the
    /// window says which widget that is.
    home: RefCell<Option<gtk::Widget>>,
}

impl Bar {
    /// The commands the bar has a control for beyond its command rows: a
    /// pill for each saved search, and a row for each message found.
    pub fn controls() -> Vec<CommandId> {
        SAVED
            .into_iter()
            .chain([CommandId::OpenMessage, CommandId::ToggleResultOrder])
            .collect()
    }

    /// A closed bar, its keys from `keymap`.
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let icon = gtk::Image::from_icon_name("system-search-symbolic");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        chips.add_css_class("focus-bar-chips");
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_has_frame(false);
        entry.set_placeholder_text(Some("Search mail, go to a folder, or run a command"));
        entry.add_css_class("focus-bar-entry");
        let input = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        input.add_css_class("focus-bar-input");
        input.append(&icon);
        input.append(&chips);
        input.append(&entry);
        if let Some(key) = postio_ui::hints::key(keymap, CommandId::Back) {
            input.append(&keyhint::cap(&key));
        }

        let echo = gtk::Label::new(None);
        echo.add_css_class("dim-label");
        echo.add_css_class("focus-bar-echo");
        echo.set_xalign(0.0);
        echo.set_visible(false);
        let heading = gtk::Label::new(None);
        heading.add_css_class("focus-bar-heading");
        heading.set_xalign(0.0);
        heading.set_visible(false);
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
        // The bar opens in place (C24): its input is drawn where the top
        // bar's own field is -- same width, centred the same way -- and the
        // results hang below it as a panel of their own.
        input.set_halign(gtk::Align::Center);
        input.set_width_request(FIELD_WIDTH);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, S2);
        panel.add_css_class("focus-bar-panel");
        panel.append(&saved_row);
        panel.append(&echo);
        panel.append(&heading);
        panel.append(&scrolled);
        panel.append(&footer);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-bar");
        root.set_width_request(WIDTH);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Start);
        root.append(&input);
        root.append(&panel);
        // The list dims behind the bar (screens 07-09). The dimming is
        // paint only: it takes no pointer, so the list under it behaves as
        // it did.
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
            entry,
            chips,
            echo,
            heading,
            list,
            keymap: RefCell::new(keymap.clone()),
            lines: RefCell::default(),
            view: RefCell::default(),
            found: RefCell::default(),
            open: Cell::new(false),
            quiet: Cell::new(false),
            places_known: Cell::new(false),
            field: RefCell::default(),
            handler: RefCell::default(),
            tab: RefCell::default(),
            me: RefCell::default(),
            saved_row,
            home: RefCell::default(),
        });
        bar.me.replace(Rc::downgrade(&bar));
        let weak = Rc::downgrade(&bar);
        // The scrim is the bar's: a press on it, outside the bar, closes
        // the bar as Escape does, rather than reaching the list beneath.
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
        bar.entry.connect_changed({
            let weak = weak.clone();
            move |entry| {
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

    /// The top bar's field: the input takes its place while the bar is open,
    /// so the field is hidden (still laid out) until the bar closes.
    pub fn set_field(&self, field: gtk::Widget) {
        field.set_opacity(if self.open.get() { 0.0 } else { 1.0 });
        self.field.replace(Some(field));
    }

    fn show_field(&self, shown: bool) {
        if let Some(field) = self.field.borrow().as_ref() {
            field.set_opacity(if shown { 1.0 } else { 0.0 });
        }
    }

    /// The bar, to lay over the window.
    pub fn widget(&self) -> &gtk::Overlay {
        &self.over
    }

    /// Hand what a person does in the bar to `handler`.
    pub fn connect_input(&self, handler: impl Fn(BarInput) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Ask `tab` what `Tab` in the field does: `false` when it is the
    /// toolkit's key, with no chip to step into.
    pub fn connect_tab(&self, tab: impl Fn() -> bool + 'static) {
        self.tab.replace(Some(Rc::new(tab)));
    }

    /// Read the keys the bar shows of its own from `keymap`: the saved
    /// searches'. A line's key comes with the line.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.show_saved();
    }

    /// Hand the keyboard to `home` whenever the bar closes.
    pub fn set_home(&self, home: gtk::Widget) {
        self.home.replace(Some(home));
    }

    /// Show the bar with `text` in its field and `select` of it selected:
    /// the controller's [`postio_focus::Intent::OpenBar`]. Also how the
    /// controller puts new words in the field of a bar already up.
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

    /// Close the bar, as the controller says. Nothing it showed is kept.
    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        // The keyboard leaves the entry with the bar: left in a hidden
        // entry, every single-letter key after it would be taken for typing.
        if let Some(root) = self.entry.root()
            && root
                .focus()
                .is_some_and(|focus| focus.is_ancestor(&self.root) || focus == self.root)
        {
            // To the list when the window said where that is, to nothing
            // otherwise: a window's own keys are heard from either.
            let home = self.home.borrow().clone();
            root.set_focus(home.as_ref());
        }
        self.over.set_visible(false);
        self.show_field(true);
        self.quiet.set(true);
        self.entry.set_text("");
        self.quiet.set(false);
        self.view.replace(BarView::default());
        self.lines.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
    }

    /// Closed by the toolkit -- a click outside -- rather than by the
    /// controller, which hears it as Back.
    fn dismiss(&self) {
        self.say(BarInput::Command(CommandId::Back));
    }

    /// The places the bar can go have been read since it opened.
    pub fn places_read(&self) {
        if self.open.get() {
            self.places_known.set(true);
        }
    }

    /// Draw `view`, whole: the controller's
    /// [`postio_focus::Intent::BarLines`].
    pub fn show(&self, view: &BarView) {
        let kept = self.highlighted_line();
        self.show_chips(&view.chips, view.editing);
        self.echo.set_visible(view.echo.is_some());
        self.echo.set_text(view.echo.as_deref().unwrap_or_default());
        self.heading.set_visible(view.heading.is_some());
        self.heading
            .set_text(view.heading.as_deref().unwrap_or_default());
        // The same lines under new tokens -- a later answer drawing what is
        // already there -- keep their rows: only what runs them changes.
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
            let found: Vec<(MessageId, String)> = view
                .lines
                .iter()
                .filter_map(|line| Some((line.message?, line.title.clone())))
                .collect();
            if !found.is_empty() {
                self.found.replace(found);
            }
        }
        let saved_changed = self.view.borrow().saved != view.saved;
        self.view.replace(view.clone());
        if saved_changed {
            self.show_saved();
        }
        // Where a run or a landing put it; else where the arrows left it,
        // while that line is still drawn -- the same line, though a later
        // answer drew it under a new token; else the first line that runs.
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

    fn show_chips(&self, chips: &[String], editing: Option<u32>) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for (index, chip) in chips.iter().enumerate() {
            let label = gtk::Label::new(Some(chip));
            label.add_css_class("focus-bar-chip");
            if editing == Some(index as u32) {
                label.add_css_class("focus-bar-chip-editing");
            }
            self.chips.append(&label);
        }
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

    /// One line's row.
    fn line_row(&self, line: &BarLine) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        match line.kind {
            BarLineKind::Heading => {
                let label = gtk::Label::new(Some(&line.title));
                label.add_css_class("focus-bar-section");
                label.set_xalign(0.0);
                row.set_child(Some(&label));
            }
            BarLineKind::Message => {
                row.add_css_class("focus-bar-row");
                row.set_child(Some(&message_line(line)));
            }
            _ => {
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

    /// Whether the bar is up.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Whether the places the bar can go have been read since it opened.
    pub fn places_known(&self) -> bool {
        self.places_known.get()
    }

    /// Handle one key in the bar's entry, as its key controller does;
    /// `true` when the bar used it. The arrows walk the lines; `Tab` steps
    /// into the chips, and on from chip to chip.
    pub fn press(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        match key {
            gtk::gdk::Key::Down => self.step(1),
            gtk::gdk::Key::Up => self.step(-1),
            gtk::gdk::Key::Tab if state.is_empty() => {
                let tab = self.tab.borrow().clone();
                return tab.is_some_and(|tab| tab());
            }
            // Typing wins: `O` is a letter for the entry. The order key
            // carries `alt` and is the window's command.
            _ => return false,
        }
        true
    }

    /// Run the search row: search for what is typed, as choosing
    /// "Search mail for …" does.
    pub fn run_search(&self) {
        let at = self
            .lines
            .borrow()
            .iter()
            .position(|line| line.kind == BarLineKind::Search);
        if let Some(at) = at {
            self.run(at);
        }
    }

    /// Run the highlighted row, as Return does.
    pub fn run_search_row(&self) {
        self.run_selected();
    }

    /// The bar's input, where the words are typed.
    pub fn input(&self) -> gtk::Widget {
        self.entry.clone().upcast()
    }

    /// What the entry holds now.
    pub fn typed(&self) -> String {
        self.entry.text().to_string()
    }

    /// The query a search would ask: the chips the words were lowered to,
    /// or the words themselves when they lower to no chip.
    pub fn query(&self) -> String {
        let chips = self.chips();
        if chips.is_empty() {
            self.entry.text().trim().to_owned()
        } else {
            chips.join(" ")
        }
    }

    /// Put `text` in the bar, as typing it would.
    pub fn set_text(&self, text: &str) {
        self.entry.set_text(text);
        self.entry.set_position(-1);
    }

    /// The heading over a folder's conversations, or over a search's.
    pub fn heading(&self) -> String {
        if self.heading.is_visible() {
            return self.heading.text().to_string();
        }
        // A search's: the first heading under the search line, which the
        // results hang from.
        let lines = self.lines.borrow();
        let Some(search) = lines
            .iter()
            .position(|line| line.kind == BarLineKind::Search)
        else {
            return String::new();
        };
        lines[search..]
            .iter()
            .find(|line| line.kind == BarLineKind::Heading)
            .map(|line| line.title.clone())
            .unwrap_or_default()
    }

    /// Every command the bar can list: what typing its name finds.
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
            .filter_map(|entry| match entry.id {
                ActionId::Builtin(command) => Some(command),
                ActionId::Ext(_) => None,
            })
            .collect()
    }

    /// The subjects of the messages listed, top to bottom.
    pub fn result_subjects(&self) -> Vec<String> {
        self.lines
            .borrow()
            .iter()
            .filter(|line| line.kind == BarLineKind::Message)
            .map(|line| line.title.clone())
            .collect()
    }

    /// The subject of `message`, as the bar last listed it.
    pub fn subject_of(&self, message: MessageId) -> Option<String> {
        self.found
            .borrow()
            .iter()
            .find(|(id, _)| *id == message)
            .map(|(_, subject)| subject.clone())
    }

    /// The highlighted row, as a storyboard observes it: what kind of row
    /// it is, and the message it would open when it is one.
    ///
    /// The text is the row's own labels, as a person reads it ("Sorted by
    /// relevance"), and a message's subject.
    pub fn highlighted(&self) -> Option<(&'static str, Option<MessageId>, String)> {
        let selected = self.list.selected_row()?;
        let index = usize::try_from(selected.index()).ok()?;
        let line = self.lines.borrow().get(index)?.clone();
        let kind = match line.kind {
            BarLineKind::Heading => "heading",
            BarLineKind::Hint => "hint",
            BarLineKind::Command => "command",
            BarLineKind::Place => "place",
            BarLineKind::Search => "search",
            BarLineKind::Instead => "instead",
            BarLineKind::Order => "order",
            BarLineKind::Correspondent => "correspondent",
            BarLineKind::Message => "message",
        };
        let text = match line.kind {
            BarLineKind::Message | BarLineKind::Place => line.title.clone(),
            _ => said(selected.upcast_ref()),
        };
        Some((kind, line.message, text))
    }

    /// How many lines of the empty bar's hints are showing.
    pub fn hints(&self) -> usize {
        self.lines
            .borrow()
            .iter()
            .filter(|line| line.kind == BarLineKind::Hint)
            .count()
    }

    /// The chips the words were lowered to.
    pub fn chips(&self) -> Vec<String> {
        self.view.borrow().chips.clone()
    }

    /// Everything the bar says, as a person reads it.
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

    /// Move the selection `by` rows, skipping what runs nothing.
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

    /// Scroll the results so `row` is wholly in view, moving them no further
    /// than that. The keyboard stays in the entry while the arrows walk the
    /// results, so the list never scrolls to its selection by itself: the
    /// highlight walked off the bottom and the results stayed put.
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
        if let Some(row) = self.list.selected_row()
            && let Ok(index) = usize::try_from(row.index())
        {
            self.run(index);
        }
    }

    /// Run row `index`: the controller says what it does.
    fn run(&self, index: usize) {
        let Some(line) = self.lines.borrow().get(index).cloned() else {
            return;
        };
        if line.selectable {
            self.say(BarInput::Run(line.token));
        }
    }
}

/// Whether `line` is `was` drawn again: the same token, or the same message,
/// or the same kind of line saying the same thing.
fn same_line(line: &BarLine, was: &BarLine) -> bool {
    line.token == was.token
        || match (line.message, was.message) {
            (Some(now), Some(then)) => now == then,
            _ => line.kind == was.kind && line.title == was.title,
        }
}

/// Every label under `widget`, as a person reads it, top to bottom.
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

/// A command's, a place's or the search's line: its title, its dimmer
/// detail, and its key.
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
        detail.set_ellipsize(pango::EllipsizeMode::End);
        row.append(&detail);
    } else {
        name.set_hexpand(true);
    }
    if let Some(key) = &line.key {
        row.append(&keyhint::cap(key));
    }
    row
}

/// A message's line: who, the subject, the first line, where it is, when.
fn message_line(line: &BarLine) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    let sender = gtk::Label::new(line.sender.as_deref());
    sender.add_css_class("focus-bar-sender");
    sender.set_width_chars(18);
    sender.set_max_width_chars(18);
    sender.set_ellipsize(pango::EllipsizeMode::End);
    sender.set_xalign(0.0);
    let title = gtk::Label::new(Some(&line.title));
    title.add_css_class("focus-bar-subject");
    // The first line gives way before the subject (gtk-design): the
    // subject keeps room for an ordinary one, and only longer ones are
    // cut; the first line has what is left.
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.set_width_chars(36);
    let first = gtk::Label::new(line.detail.as_deref());
    first.add_css_class("dim-label");
    first.set_width_chars(8);
    first.set_ellipsize(pango::EllipsizeMode::End);
    first.set_hexpand(true);
    first.set_xalign(0.0);
    row.append(&sender);
    row.append(&title);
    row.append(&first);
    // The folder, and under it the account when there are several: one
    // column two lines tall, not one long line.
    if !line.wheres.is_empty() {
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lines.set_valign(gtk::Align::Center);
        for text in &line.wheres {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("focus-bar-place");
            label.set_xalign(1.0);
            lines.append(&label);
        }
        row.append(&lines);
    }
    if let Some(time) = &line.time {
        let date = gtk::Label::new(Some(time));
        date.add_css_class("dim-label");
        row.append(&date);
    }
    row
}
