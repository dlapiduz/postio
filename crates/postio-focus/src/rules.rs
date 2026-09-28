//! The digest rules list, `g d` (spec 007 T139, C15), in Filtered's
//! full-view frame (screen 21): every rule in `config.toml` with what it
//! matches, when it delivers, when it next does, and what it holds now.
//!
//! Designed with `/ux-architect`'s invariants: a full view in place of the
//! inbox rather than another dialog; its keys are the list's (`j`/`k`
//! walk it, `Enter` edits the focused rule in screen 24's dialog, `Delete`
//! removes it) so nothing new is learned; removing asks first, because it
//! releases what the rule held into the inbox and no undo takes a rule's
//! mail back into it; and with no rule it says how to make one.

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

/// What a person asked of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesAction {
    /// Go back to the inbox.
    Back,
    /// Edit this rule.
    Edit(String),
    /// Remove this rule, releasing what it holds.
    Remove {
        /// The rule's name.
        name: String,
        /// What it holds now.
        holds: u32,
    },
}

type Handler = Rc<dyn Fn(RulesAction)>;

/// The view. See the module.
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
    /// The view, reading through `client`, its keys from `keymap`.
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

    /// The view, for the window's pages.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` with what the person asks.
    pub fn connect_action(&self, handler: impl Fn(RulesAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.footer.set(&digest::rules_footer(keymap));
        self.empty_hint.set_text(&digest::rules_empty_hint(keymap));
        self.show();
    }

    /// Show `rules`, and read what each holds now.
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

    /// Take the rule called `name` off the list: it was removed.
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

    /// Move the selection `by` rows: `j` and `k`.
    pub fn step(&self, by: i32) {
        let at = self.list.selected_row().map_or(-1, |row| row.index());
        if let Some(row) = self.list.row_at_index((at + by).max(0)) {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    /// The focused rule's name and what it holds: what `Enter` edits and
    /// `Delete` removes.
    pub fn focused(&self) -> Option<(String, u32)> {
        let row = self.list.selected_row()?;
        let index = usize::try_from(row.index()).ok()?;
        let name = self.rules.borrow().get(index)?.name.clone();
        Some((name, self.holds.borrow().get(index).copied().unwrap_or(0)))
    }

    /// Every text the view shows, in order: what a test reads.
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

    /// What each rule holds now: one request, one statement.
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
            // Only if the list is still the one asked about.
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

    /// One row a rule: its name, what it matches, when it delivers, when
    /// it next does, and what it holds.
    fn show(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let rules = self.rules.borrow();
        let holds = self.holds.borrow();
        let rule_count = rules.len();
        self.count.set_text(&if rule_count == 1 {
            "1 rule".to_owned()
        } else {
            format!("{rule_count} rules")
        });
        self.empty.set_visible(rules.is_empty());
        let keymap = self.keymap.borrow();
        let now = chrono::Local::now();
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
            for (command, words) in [
                (CommandId::OpenMessage, "Edit"),
                (CommandId::Delete, "Remove"),
            ] {
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
    }
}

/// The focused row's buttons, in a row of the list.
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
