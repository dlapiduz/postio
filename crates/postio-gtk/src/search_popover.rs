//! The results' filter popovers and their month chart (spec 010 step 4,
//! design §3.3 and §3.6, screens 08 and 09).
//!
//! The controller holds which popover is open, the query it will put back,
//! its rows and which are checked; every check, preset and word goes down
//! and the next `Intent::Popover` redraws it whole. What is GTK's is the
//! toolkit's own, as on the Mac (`FilterPopoverModel.swift`): which row the
//! arrows rest on (the highlight Space toggles), and what the fields hold
//! while they are typed in. A close the controller made is never said
//! back; one the toolkit made -- Escape, a click away -- is, once.
//!
//! The month chart is the timeline's and the Date popover's: a bar a month,
//! oldest first, as tall as the controller says, and a drag across them
//! said once, on release, as the bars it covered.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;
use postio_focus::{Input, MonthBar, PopoverRow, PopoverView};
use postio_ui::search_view::FilterKind;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The tallest bar, in the timeline and the Date popover.
const BAR_HEIGHT: i32 = 36;
/// A bar's width.
const BAR_WIDTH: i32 = 18;

type MonthsHandler = Rc<dyn Fn(u32, u32)>;

/// A bar a month, and a drag across them.
pub struct MonthChart {
    root: gtk::Box,
    report: RefCell<Option<MonthsHandler>>,
    /// Where a drag began, in the chart's coordinates.
    from: Cell<Option<f64>>,
}

impl MonthChart {
    /// An empty chart.
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        root.add_css_class("focus-month-chart");
        root.set_valign(gtk::Align::End);
        let chart = Rc::new(MonthChart {
            root,
            report: RefCell::default(),
            from: Cell::new(None),
        });
        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&chart);
        drag.connect_drag_begin({
            let weak = weak.clone();
            move |_, x, _| {
                if let Some(chart) = weak.upgrade() {
                    chart.from.set(Some(x));
                }
            }
        });
        drag.connect_drag_end(move |_, dx, _| {
            let Some(chart) = weak.upgrade() else {
                return;
            };
            let Some(from) = chart.from.take() else {
                return;
            };
            let (Some(a), Some(b)) = (chart.bar_at(from), chart.bar_at(from + dx)) else {
                return;
            };
            chart.say(a.min(b), a.max(b));
        });
        chart.root.add_controller(drag);
        chart
    }

    /// What lays it out.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// A drag ended over bars `first..=last`, 0 the oldest.
    pub fn connect_months(&self, report: impl Fn(u32, u32) + 'static) {
        self.report.replace(Some(Rc::new(report)));
    }

    /// Draw `months`, oldest first.
    pub fn show(&self, months: &[MonthBar]) {
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        let any_selected = months.iter().any(|month| month.selected);
        for month in months {
            let column = gtk::Box::new(gtk::Orientation::Vertical, S1);
            column.add_css_class("focus-month");
            if month.selected {
                column.add_css_class("focus-month-selected");
            } else if any_selected {
                column.add_css_class("focus-month-faded");
            }
            let room = gtk::Box::new(gtk::Orientation::Vertical, 0);
            room.set_size_request(BAR_WIDTH, BAR_HEIGHT);
            // The bar fills its room and no more: unset, its expansion
            // would be handed up to the page and take its height.
            room.set_vexpand(false);
            column.set_vexpand(false);
            let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
            bar.add_css_class("focus-month-bar");
            let height = (month.height.clamp(0.0, 1.0) * f64::from(BAR_HEIGHT)).round() as i32;
            // A month with matches is never drawn as nothing.
            let height = if month.conversations > 0 {
                height.max(2)
            } else {
                0
            };
            bar.set_size_request(BAR_WIDTH, height);
            bar.set_valign(gtk::Align::End);
            bar.set_vexpand(true);
            room.append(&bar);
            column.append(&room);
            let label = gtk::Label::new(Some(&month.label));
            label.add_css_class("focus-month-label");
            column.append(&label);
            column.set_tooltip_text(Some(&format!("{} \u{b7} {}", month.label, month.count)));
            column.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} {}",
                month.label, month.count
            ))]);
            self.root.append(&column);
        }
    }

    /// The months drawn, and whether each is selected: what a test reads.
    pub fn bars(&self) -> Vec<(String, bool)> {
        let mut bars = Vec::new();
        let mut child = self.root.first_child();
        while let Some(column) = child {
            let label = column
                .last_child()
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string())
                .unwrap_or_default();
            bars.push((label, column.has_css_class("focus-month-selected")));
            child = column.next_sibling();
        }
        bars
    }

    /// A drag across bars `first..=last`, as a person's would end.
    pub fn drag(&self, first: u32, last: u32) {
        self.say(first, last);
    }

    /// The bar under `x`, the nearest at either end.
    fn bar_at(&self, x: f64) -> Option<u32> {
        let mut index = 0u32;
        let mut last = None;
        let mut child = self.root.first_child();
        while let Some(column) = child {
            let bounds = column.compute_bounds(&self.root)?;
            if x < f64::from(bounds.x() + bounds.width()) {
                return Some(index);
            }
            last = Some(index);
            index += 1;
            child = column.next_sibling();
        }
        last
    }

    fn say(&self, first: u32, last: u32) {
        let report = self.report.borrow().clone();
        if let Some(report) = report {
            report(first, last);
        }
    }
}

/// The filter popover that is up.
pub struct FilterPopover {
    popover: gtk::Popover,
    field: gtk::Entry,
    parsed: gtk::Label,
    presets: gtk::Box,
    chart: Rc<MonthChart>,
    range: gtk::Label,
    rows: gtk::ListBox,
    hints: gtk::Box,
    view: RefCell<Option<PopoverView>>,
    /// The row the arrows rest on, an index into the view's rows.
    highlight: Cell<Option<usize>>,
    /// Set while the controller closes it or puts its words in the field,
    /// so neither is said back.
    quiet: Cell<bool>,
    say: Rc<dyn Fn(Input)>,
    me: RefCell<Weak<FilterPopover>>,
}

impl FilterPopover {
    /// A popover that tells the controller what happens in it through
    /// `say`.
    pub fn new(say: impl Fn(Input) + 'static) -> Rc<Self> {
        let field = gtk::Entry::new();
        field.add_css_class("focus-popover-field");
        let parsed = gtk::Label::new(None);
        parsed.add_css_class("focus-popover-parsed");
        parsed.add_css_class("dim-label");
        parsed.set_xalign(0.0);
        let presets = gtk::Box::new(gtk::Orientation::Vertical, 0);
        presets.add_css_class("focus-popover-presets");
        let chart = MonthChart::new();
        let range = gtk::Label::new(None);
        range.add_css_class("dim-label");
        range.set_xalign(0.0);
        let rows = gtk::ListBox::new();
        rows.add_css_class("focus-popover-rows");
        rows.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&rows)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(320)
            .build();
        let hints = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        hints.add_css_class("focus-popover-footer");
        let content = gtk::Box::new(gtk::Orientation::Vertical, S2);
        content.add_css_class("focus-popover");
        content.append(&field);
        content.append(&parsed);
        content.append(&presets);
        content.append(chart.widget());
        content.append(&range);
        content.append(&scrolled);
        content.append(&hints);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&content));
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_has_arrow(false);
        // From its button's start, never centred on it: the buttons sit at
        // the window's edge, and a compositor that does not slide a popup
        // back inside would draw it cut off.
        popover.set_halign(gtk::Align::Start);
        popover.add_css_class("focus-filter-popover");

        let pop = Rc::new(FilterPopover {
            popover,
            field,
            parsed,
            presets,
            chart,
            range,
            rows,
            hints,
            view: RefCell::default(),
            highlight: Cell::new(None),
            quiet: Cell::new(false),
            say: Rc::new(say),
            me: RefCell::default(),
        });
        pop.me.replace(Rc::downgrade(&pop));
        pop.connect();
        pop
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.field.connect_changed({
            let weak = weak.clone();
            move |field| {
                let Some(pop) = weak.upgrade() else {
                    return;
                };
                if pop.quiet.get() {
                    return;
                }
                let text = field.text().to_string();
                let date = pop
                    .view
                    .borrow()
                    .as_ref()
                    .is_some_and(|view| view.kind == FilterKind::Date);
                (pop.say)(if date {
                    Input::DateWords(text)
                } else {
                    Input::PopoverFilter(text)
                });
            }
        });
        self.field.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(pop) = weak.upgrade() {
                    pop.apply();
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, state| {
                let Some(pop) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if pop.press(key, state) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        self.popover.add_controller(keys);
        self.rows.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(pop) = weak.upgrade()
                    && let Ok(index) = usize::try_from(row.index())
                {
                    pop.toggle(index, false);
                }
            }
        });
        self.chart.connect_months({
            let weak = weak.clone();
            move |first, last| {
                if let Some(pop) = weak.upgrade() {
                    (pop.say)(Input::SearchEdit(postio_focus::TermEdit::SetMonths {
                        first,
                        last,
                    }));
                }
            }
        });
        self.popover.connect_closed(move |_| {
            if let Some(pop) = weak.upgrade()
                && !pop.quiet.get()
                && pop.view.take().is_some()
            {
                pop.highlight.set(None);
                (pop.say)(Input::PopoverDone { apply: false });
            }
        });
    }

    /// Draw `view` hung from `button`, or close with `None`: the
    /// controller's `Intent::Popover`.
    pub fn show(&self, view: Option<&PopoverView>, button: Option<&gtk::Widget>) {
        let Some(view) = view else {
            if self.view.take().is_some() {
                self.highlight.set(None);
                self.quiet.set(true);
                self.popover.popdown();
                self.quiet.set(false);
            }
            return;
        };
        let opened = self
            .view
            .borrow()
            .as_ref()
            .is_none_or(|was| was.kind != view.kind);
        let date = view.kind == FilterKind::Date;
        self.quiet.set(true);
        if opened {
            self.field
                .set_text(if date { &view.words } else { &view.filter });
            self.highlight.set((!view.rows.is_empty()).then_some(0));
        } else if let Some(at) = self.highlight.get() {
            self.highlight
                .set((!view.rows.is_empty()).then(|| at.min(view.rows.len() - 1)));
        } else if !view.rows.is_empty() {
            self.highlight.set(Some(0));
        }
        self.quiet.set(false);
        self.field.set_placeholder_text(Some(&view.placeholder));
        self.parsed.set_visible(date);
        self.parsed.set_text(&if date {
            view.parsed
                .clone()
                .unwrap_or_else(|| view.words_hint.clone())
        } else {
            String::new()
        });
        self.show_presets(view);
        self.chart
            .widget()
            .set_visible(date && !view.months.is_empty());
        self.chart.show(&view.months);
        let range: Vec<&str> = [view.range.as_deref(), view.result.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        self.range.set_visible(date && !range.is_empty());
        self.range.set_text(&range.join(" \u{b7} "));
        self.show_rows(&view.rows);
        while let Some(child) = self.hints.first_child() {
            self.hints.remove(&child);
        }
        for hint in &view.hints {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            line.append(&keyhint::cap(&hint.key));
            let words = gtk::Label::new(Some(&hint.label));
            words.add_css_class("dim-label");
            line.append(&words);
            self.hints.append(&line);
        }
        self.view.replace(Some(view.clone()));
        if let Some(button) = button
            && (opened || !self.popover.is_visible())
        {
            if self.popover.parent().as_ref() != Some(button) {
                self.popover.unparent();
                self.popover.set_parent(button);
            }
            self.popover.popup();
            self.field.grab_focus();
        }
    }

    fn show_presets(&self, view: &PopoverView) {
        while let Some(child) = self.presets.first_child() {
            self.presets.remove(&child);
        }
        self.presets.set_visible(!view.presets.is_empty());
        for preset in &view.presets {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            let label = gtk::Label::new(Some(&preset.label));
            label.set_hexpand(true);
            label.set_xalign(0.0);
            line.append(&label);
            if let Some(count) = &preset.count {
                let count = gtk::Label::new(Some(count));
                count.add_css_class("dim-label");
                line.append(&count);
            }
            let button = gtk::Button::new();
            button.set_child(Some(&line));
            button.add_css_class("flat");
            button.add_css_class("focus-popover-preset");
            if preset.selected {
                button.add_css_class("focus-popover-preset-selected");
            }
            button.set_focus_on_click(false);
            let token = preset.token;
            let weak = self.me.borrow().clone();
            button.connect_clicked(move |_| {
                if let Some(pop) = weak.upgrade() {
                    pop.quiet.set(true);
                    pop.field.set_text("");
                    pop.quiet.set(false);
                    (pop.say)(Input::DatePreset(token));
                }
            });
            self.presets.append(&button);
        }
    }

    fn show_rows(&self, rows: &[PopoverRow]) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        // The list's scroller with it: the Date popover has no rows, and an
        // empty scroller still holds its room.
        if let Some(scroller) = self.rows.ancestor(gtk::ScrolledWindow::static_type()) {
            scroller.set_visible(!rows.is_empty());
        }
        for row in rows {
            self.rows.append(&row_widget(row));
        }
        if let Some(row) = self
            .highlight
            .get()
            .and_then(|at| self.rows.row_at_index(at as i32))
        {
            self.rows.select_row(Some(&row));
        }
    }

    /// One key in the popover: the arrows move the highlight, Space
    /// toggles it, Return keeps what it previews. Escape is the toolkit's,
    /// which closes it.
    fn press(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        let count = self
            .view
            .borrow()
            .as_ref()
            .map_or(0, |view| view.rows.len());
        match key {
            gtk::gdk::Key::Down | gtk::gdk::Key::Up if count > 0 => {
                let step: i64 = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                let at = self.highlight.get().map_or(-1, |at| at as i64) + step;
                let at = at.clamp(0, count as i64 - 1) as usize;
                self.highlight.set(Some(at));
                if let Some(row) = self.rows.row_at_index(at as i32) {
                    self.rows.select_row(Some(&row));
                }
                true
            }
            gtk::gdk::Key::space if count > 0 && self.field.text().is_empty() => {
                if let Some(at) = self.highlight.get() {
                    self.toggle(at, state.contains(gtk::gdk::ModifierType::ALT_MASK));
                }
                true
            }
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                self.apply();
                true
            }
            _ => false,
        }
    }

    /// Row `index` toggled: Space or a click, excluded with Alt held.
    fn toggle(&self, index: usize, exclude: bool) {
        let token = self
            .view
            .borrow()
            .as_ref()
            .and_then(|view| view.rows.get(index).map(|row| row.token));
        if let Some(token) = token {
            self.highlight.set(Some(index));
            (self.say)(Input::PopoverToggle { token, exclude });
        }
    }

    /// Its button is about to be drawn again: take the popover off it, so
    /// it does not go with it.
    pub fn unhang(&self) {
        if self.popover.parent().is_some() {
            self.quiet.set(true);
            self.popover.popdown();
            self.popover.unparent();
            self.quiet.set(false);
        }
    }

    /// Its button was drawn again: hang it from the new one, up again while
    /// the controller still has it open.
    pub fn rehang(&self, button: &gtk::Widget) {
        if self.view.borrow().is_none() {
            return;
        }
        if self.popover.parent().as_ref() != Some(button) {
            self.popover.unparent();
            self.popover.set_parent(button);
        }
        // Up once the new button has been laid out: popped up on a button
        // with no place yet, it hangs from nowhere and is drawn cut off.
        let weak = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(pop) = weak.upgrade()
                && pop.view.borrow().is_some()
            {
                pop.popover.popup();
                pop.field.grab_focus();
            }
        });
    }

    /// Which button's kind it hangs from, while it is open.
    pub fn kind(&self) -> Option<FilterKind> {
        self.view.borrow().as_ref().map(|view| view.kind)
    }

    /// Return: keep what it previewed. The controller closes it.
    pub fn apply(&self) {
        if self.view.borrow().is_some() {
            (self.say)(Input::PopoverDone { apply: true });
        }
    }

    /// Whether a popover is up.
    pub fn is_open(&self) -> bool {
        self.view.borrow().is_some() && self.popover.is_visible()
    }

    /// The rows' titles, and whether each is checked: what a test reads.
    pub fn rows(&self) -> Vec<(String, bool)> {
        self.view
            .borrow()
            .as_ref()
            .map(|view| {
                view.rows
                    .iter()
                    .map(|row| (row.title.clone(), row.checked))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The row the arrows rest on.
    pub fn highlighted(&self) -> Option<usize> {
        self.highlight.get()
    }

    /// Press `key` in the popover, as its key controller hears it.
    pub fn key(&self, key: gtk::gdk::Key) -> bool {
        self.press(key, gtk::gdk::ModifierType::empty())
    }

    /// Type `text` into its field.
    pub fn type_in(&self, text: &str) {
        self.field.set_text(text);
    }

    /// The Date popover's chart.
    pub fn chart(&self) -> &MonthChart {
        &self.chart
    }

    /// Close it as a click away does.
    pub fn dismiss(&self) {
        self.popover.popdown();
    }
}

/// One row: its check, who or what it is, how many, and its share bar.
fn row_widget(row: &PopoverRow) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    line.add_css_class("focus-popover-row");
    let check = gtk::Label::new(Some(if row.excluded {
        "\u{2296}"
    } else if row.checked {
        "\u{2611}"
    } else {
        "\u{2610}"
    }));
    check.add_css_class("focus-popover-check");
    check.set_accessible_role(gtk::AccessibleRole::Presentation);
    line.append(&check);
    if let Some(initials) = &row.initials {
        let avatar = gtk::Label::new(Some(initials));
        avatar.add_css_class("focus-dropdown-avatar");
        avatar.set_accessible_role(gtk::AccessibleRole::Presentation);
        line.append(&avatar);
    }
    let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
    words.set_hexpand(true);
    let title = gtk::Label::new(Some(&row.title));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if row.excluded {
        title.set_markup(&format!("<s>{}</s>", glib::markup_escape_text(&row.title)));
    }
    words.append(&title);
    if let Some(detail) = &row.detail {
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("dim-label");
        detail.add_css_class("focus-popover-detail");
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        words.append(&detail);
    }
    line.append(&words);
    let share = gtk::LevelBar::for_interval(0.0, 1.0);
    share.set_value(row.share.clamp(0.0, 1.0));
    share.add_css_class("focus-popover-share");
    share.set_valign(gtk::Align::Center);
    share.set_size_request(48, -1);
    line.append(&share);
    let count = gtk::Label::new(Some(&row.count_label));
    count.add_css_class("focus-popover-count");
    count.add_css_class("dim-label");
    line.append(&count);
    line
}
