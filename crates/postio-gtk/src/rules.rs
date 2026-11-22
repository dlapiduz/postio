//! The digest rules list, `g r`, in Filtered's full-view frame. Removing asks first, because it
//! releases what the rule held into the inbox and no undo takes a rule's mail back into it.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_config::DigestRule;
use postio_core::{CommandId, Keymap};
use postio_ui::digest;
use postio_widgets::widgets::keyhint::{self, KeyLine};
use postio_widgets::widgets::space::{S1, S3};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesAction {
    Back,
    Edit(String),
    Remove { name: String, holds: u32 },
}

type Handler = Rc<dyn Fn(RulesAction)>;

pub struct RulesView {
    client: Client,
    root: gtk::Box,
    count: gtk::Label,
    list: gtk::ListBox,
    empty: gtk::Box,
    empty_hint: gtk::Label,
    footer: KeyLine,
    keymap: RefCell<Keymap>,
    rules: RefCell<Vec<DigestRule>>,
    holds: RefCell<Vec<u32>>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<RulesView>>,
}

impl RulesView {
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let back = gtk::Button::new();
        back.add_css_class("flat");
        let back_row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        back_row.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        back_row.append(&gtk::Label::new(Some("Inbox")));
        back.set_child(Some(&back_row));
        let title = gtk::Label::new(Some(digest::RULES_TITLE));
        title.add_css_class("focus-filtered-title");
        let subtitle = gtk::Label::new(Some(digest::RULES_SUBTITLE));
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("focus-filtered-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&title);
        titles.append(&subtitle);
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-filtered-header");
        header.set_start_widget(Some(&back));
        header.set_center_widget(Some(&titles));

        let count = gtk::Label::new(None);
        count.add_css_class("focus-day-heading");
        count.add_css_class("focus-filtered-day");
        count.set_xalign(0.0);

        let list = gtk::ListBox::new();
        list.add_css_class("focus-filtered-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let empty_title = gtk::Label::new(Some(digest::RULES_EMPTY));
        empty_title.add_css_class("focus-rules-empty-title");
        let empty_hint = gtk::Label::new(None);
        empty_hint.add_css_class("dim-label");
        let empty = gtk::Box::new(gtk::Orientation::Vertical, S1);
        empty.set_valign(gtk::Align::Center);
        empty.set_vexpand(true);
        empty.append(&empty_title);
        empty.append(&empty_hint);
        empty.set_visible(false);
        let footer = KeyLine::new("focus-filtered-footer");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-filtered");
        root.append(&header);
        root.append(&count);
        root.append(&scrolled);
        root.append(&empty);
        root.append(footer.widget());

        let view = Rc::new(RulesView {
            client,
            root,
            count,
            list,
            empty,
            empty_hint,
            footer,
            keymap: RefCell::new(keymap.clone()),
            rules: RefCell::default(),
            holds: RefCell::default(),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        view.me.replace(Rc::downgrade(&view));
        let weak = Rc::downgrade(&view);
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(RulesAction::Back);
                }
            }
        });
        view.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(view) = weak.upgrade()
                    && let Some(rule) = view.rule_at(row.index())
                {
                    view.emit(RulesAction::Edit(rule.name));
                }
            }
        });
        view.list.connect_row_selected(|list, selected| {
            let mut child = list.first_child();
            while let Some(row) = child {
                child = row.next_sibling();
                if let Some(actions) = actions_of(&row) {
                    actions.set_visible(selected.is_some_and(|selected| *selected == row));
                }
            }
        });
        view.set_keymap(keymap);
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_action(&self, handler: impl Fn(RulesAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.footer.set(&digest::rules_footer(keymap));
        self.empty_hint.set_text(&digest::rules_empty_hint(keymap));
        self.show();
    }

    pub fn open(&self, rules: Vec<DigestRule>) {
        self.holds.replace(vec![0; rules.len()]);
        self.rules.replace(rules);
        self.show();
        self.read_holds();
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
            first.grab_focus();
        }
    }

    pub fn forget(&self, name: &str) {
        let index = self
            .rules
            .borrow()
            .iter()
            .position(|rule| rule.name == name);
        if let Some(index) = index {
            self.rules.borrow_mut().remove(index);
            self.holds.borrow_mut().remove(index);
        }
        self.show();
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        }
    }

    pub fn step(&self, by: i32) {
        let at = self.list.selected_row().map_or(-1, |row| row.index());
        if let Some(row) = self.list.row_at_index((at + by).max(0)) {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    pub fn focused(&self) -> Option<(String, u32)> {
        let row = self.list.selected_row()?;
        let index = usize::try_from(row.index()).ok()?;
        let name = self.rules.borrow().get(index)?.name.clone();
        Some((name, self.holds.borrow().get(index).copied().unwrap_or(0)))
    }

    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.root.clone().upcast::<gtk::Widget>()];
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

    fn emit(&self, action: RulesAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    fn rule_at(&self, index: i32) -> Option<DigestRule> {
        let index = usize::try_from(index).ok()?;
        self.rules.borrow().get(index).cloned()
    }

    fn read_holds(&self) {
        let names: Vec<String> = self
            .rules
            .borrow()
            .iter()
            .map(|rule| rule.name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.digest_waiting(names.clone()).await;
            let (Some(view), Ok(holds)) = (weak.upgrade(), read) else {
                return;
            };
            let still: Vec<String> = view
                .rules
                .borrow()
                .iter()
                .map(|rule| rule.name.clone())
                .collect();
            if still == names {
                let selected = view.list.selected_row().map(|row| row.index());
                view.holds.replace(holds);
                view.show();
                if let Some(row) = selected.and_then(|at| view.list.row_at_index(at)) {
                    view.list.select_row(Some(&row));
                }
            }
        });
    }

    fn show(&self) {
        crate::rebuild::keeping_focus(&self.list, || self.rebuild());
    }

    fn rebuild(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let rules = self.rules.borrow();
        let holds = self.holds.borrow();
        let rule_count = rules.len();
        self.count.set_text(&digest::rule_count(rule_count));
        self.empty.set_visible(rules.is_empty());
        let keymap = self.keymap.borrow();
        let now = postio_ui::clock::now();
        for (index, rule) in rules.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-filtered-row");
            let name = gtk::Label::new(Some(&rule.name));
            name.add_css_class("focus-rules-name");
            name.set_xalign(0.0);
            name.set_width_chars(22);
            name.set_max_width_chars(22);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&name);
            let matches = gtk::Label::new(Some(&rule.queries.join(", ")));
            matches.add_css_class("focus-rules-match");
            matches.set_xalign(0.0);
            matches.set_hexpand(true);
            matches.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&matches);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            actions.add_css_class("focus-rules-actions");
            actions.set_visible(false);
            for (command, words) in digest::RULE_ROW_BUTTONS {
                let button = gtk::Button::new();
                button.add_css_class("flat");
                button.set_valign(gtk::Align::Center);
                button.set_child(Some(&keyhint::labelled(
                    words,
                    postio_ui::hints::key(&keymap, command).as_deref(),
                )));
                let weak = self.me.borrow().clone();
                let (rule_name, held) = (rule.name.clone(), holds.get(index).copied().unwrap_or(0));
                button.connect_clicked(move |_| {
                    if let Some(view) = weak.upgrade() {
                        view.emit(match command {
                            CommandId::Delete => RulesAction::Remove {
                                name: rule_name.clone(),
                                holds: held,
                            },
                            _ => RulesAction::Edit(rule_name.clone()),
                        });
                    }
                });
                actions.append(&button);
            }
            line.append(&actions);
            if let Some(when) = digest::rule_when(rule) {
                let label = gtk::Label::new(Some(&when));
                label.add_css_class("dim-label");
                line.append(&label);
            }
            if let Some(next) = rule
                .due()
                .ok()
                .and_then(|due| postio_ui::schedule::next_due(&due, &now))
            {
                let label = gtk::Label::new(Some(&digest::next_delivery(next)));
                label.add_css_class("dim-label");
                line.append(&label);
            }
            let held =
                gtk::Label::new(Some(&digest::holds(holds.get(index).copied().unwrap_or(0))));
            held.add_css_class("focus-filtered-pill");
            held.set_valign(gtk::Align::Center);
            line.append(&held);
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
        }
        // Rows are rebuilt here, including after `read_holds`'s fetch lands, so a cap taught only
        // once after `open()` would miss every row this redraw replaces.
        crate::a11y::teach_shortcuts(&self.root);
        crate::motion::keep_to_budget(&self.root);
    }
}

fn actions_of(row: &gtk::Widget) -> Option<gtk::Widget> {
    let mut stack = vec![row.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("focus-rules-actions") {
            return Some(widget);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}
