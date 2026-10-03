//! The digest rule dialog (spec 007 US10, T138; screen 24): "Digest this
//! sender", from `d` on a message, "Digest these…" in the bulk bar, or `d`
//! in a digest to edit its rule. The address, how often and when, what
//! the rule would have caught over the last 90 days -- through the
//! executor, as search counts it -- the note, and Create, which writes the
//! rule to `config.toml`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::{DigestRuleDraft, RuleDay};
use postio_config::{DigestRule, Due};
use postio_core::{CommandId, Keymap};
use postio_model::listing::Cadence;
use postio_model::{EmailAddress, MessageId};
use postio_ui::{digest, hints};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The dialog's name, for the window to find it by.
pub const DIALOG_NAME: &str = "focus-digest-rule";

/// The dialog's width (contracts/focus-surface.md).
const WIDTH: i32 = 620;

use postio_ui::digest::{CADENCES, PREVIEW_DAYS, WEEKDAYS};

type Saved = Rc<dyn Fn(String)>;

/// The dialog. See the module.
pub struct RuleDialog {
    client: Client,
    dialog: adw::Dialog,
    heading: gtk::Label,
    from: gtk::Label,
    match_instead: gtk::Button,
    query_entry: gtk::Entry,
    like_this_button: gtk::Button,
    cadence: gtk::DropDown,
    on: gtk::Label,
    weekday: gtk::DropDown,
    month_day: gtk::DropDown,
    at: gtk::Entry,
    preview_heading: gtk::Label,
    preview: gtk::ListBox,
    more: gtk::Label,
    note: gtk::Label,
    error: gtk::Label,
    create_words: gtk::Box,
    keymap: RefCell<Keymap>,
    name: RefCell<String>,
    queries: RefCell<Vec<String>>,
    replacing: RefCell<Option<String>>,
    /// The message "Digest mail like this" would check other mail against,
    /// when the user has brought a model with `like_this` on: `None` makes
    /// the control absent (US14, FR-171).
    like_this: RefCell<Option<MessageId>>,
    generation: Cell<u64>,
    saved: RefCell<Option<Saved>>,
    me: RefCell<std::rc::Weak<RuleDialog>>,
}

impl RuleDialog {
    /// A closed dialog, reading and writing through `client`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let cancel = gtk::Button::new();
        cancel.add_css_class("flat");
        let cancel_words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        cancel_words.append(&gtk::Label::new(Some("Cancel")));
        if let Some(key) = hints::key(keymap, CommandId::Back) {
            cancel_words.append(&keyhint::cap(&key));
        }
        cancel.set_child(Some(&cancel_words));
        let heading = gtk::Label::new(None);
        heading.add_css_class("focus-rule-heading");
        let create = gtk::Button::new();
        postio_widgets::widgets::button::style(
            &create,
            postio_widgets::widgets::Kind::Primary,
            postio_widgets::widgets::Size::Regular,
        );
        create.add_css_class("focus-rule-create");
        let create_words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        create.set_child(Some(&create_words));
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-rule-header");
        header.set_start_widget(Some(&cancel));
        header.set_center_widget(Some(&heading));
        header.set_end_widget(Some(&create));

        let from = gtk::Label::new(None);
        from.add_css_class("focus-rule-from");
        from.set_xalign(0.0);
        from.set_hexpand(true);
        from.set_selectable(true);
        from.set_wrap(true);
        // "Match a list or a search instead…" (US14, T155): swaps `from`
        // for a typed `list:` or query rule, previewed the same way.
        let query_entry = gtk::Entry::new();
        query_entry.set_hexpand(true);
        query_entry.set_placeholder_text(Some(digest::QUERY_PLACEHOLDER));
        query_entry.set_visible(false);
        let from_value = gtk::Box::new(gtk::Orientation::Vertical, S1);
        from_value.append(&from);
        from_value.append(&query_entry);
        let match_instead = gtk::Button::new();
        match_instead.add_css_class("flat");
        match_instead.add_css_class("focus-rule-match-instead");
        match_instead.set_child(Some(&gtk::Label::new(Some(
            "Match a list or a search instead…",
        ))));
        match_instead.set_halign(gtk::Align::Start);
        // "Digest mail like this" (US14, FR-171): only when a message is
        // given -- which the window only does once the user has brought a
        // model with `like_this` on (`ModelFeature::LikeThis`).
        let like_this_button = gtk::Button::new();
        like_this_button.add_css_class("flat");
        like_this_button.add_css_class("focus-rule-like-this");
        like_this_button.set_child(Some(&gtk::Label::new(Some("Digest mail like this"))));
        like_this_button.set_halign(gtk::Align::Start);
        like_this_button.set_visible(false);
        let links = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        links.append(&match_instead);
        links.append(&like_this_button);
        let cadence = gtk::DropDown::from_strings(&CADENCES.map(|(_, name)| name));
        cadence.set_selected(1);
        let on = gtk::Label::new(Some("on"));
        let weekday_names: Vec<String> = WEEKDAYS
            .iter()
            .map(|day| {
                chrono::NaiveDate::from_isoywd_opt(2026, 1, *day)
                    .map(|date| date.format("%A").to_string())
                    .unwrap_or_default()
            })
            .collect();
        let weekday_refs: Vec<&str> = weekday_names.iter().map(String::as_str).collect();
        let weekday = gtk::DropDown::from_strings(&weekday_refs);
        weekday.set_selected(6);
        let month_days: Vec<String> = (1..=28).map(|day| day.to_string()).collect();
        let month_refs: Vec<&str> = month_days.iter().map(String::as_str).collect();
        let month_day = gtk::DropDown::from_strings(&month_refs);
        month_day.set_visible(false);
        let at = gtk::Entry::new();
        at.set_text("09:00");
        at.set_width_chars(6);
        at.set_max_width_chars(6);
        let deliver = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        deliver.append(&cadence);
        deliver.append(&on);
        deliver.append(&weekday);
        deliver.append(&month_day);
        deliver.append(&gtk::Label::new(Some("at")));
        deliver.append(&at);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(S3 as u32);
        grid.set_column_spacing(S3 as u32);
        let label = |text: &str| {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("dim-label");
            label.set_xalign(0.0);
            label
        };
        grid.attach(&label("From"), 0, 0, 1, 1);
        grid.attach(&from_value, 1, 0, 1, 1);
        grid.attach(&label("Deliver"), 0, 1, 1, 1);
        grid.attach(&deliver, 1, 1, 1, 1);

        let preview_heading = gtk::Label::new(None);
        preview_heading.add_css_class("focus-rule-preview-heading");
        preview_heading.set_xalign(0.0);
        let preview = gtk::ListBox::new();
        preview.add_css_class("boxed-list");
        preview.set_selection_mode(gtk::SelectionMode::None);
        let more = gtk::Label::new(None);
        more.add_css_class("dim-label");
        more.set_xalign(0.0);
        let note = gtk::Label::new(None);
        note.add_css_class("dim-label");
        note.set_xalign(0.0);
        note.set_wrap(true);
        let error = gtk::Label::new(None);
        error.add_css_class("error");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_visible(false);

        let body = gtk::Box::new(gtk::Orientation::Vertical, S3);
        body.add_css_class("focus-rule-body");
        body.append(&grid);
        body.append(&links);
        body.append(&preview_heading);
        body.append(&preview);
        body.append(&more);
        body.append(&note);
        body.append(&error);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-rule");
        content.append(&header);
        content.append(&body);
        let dialog = adw::Dialog::builder()
            .content_width(WIDTH)
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);

        let this = Rc::new(RuleDialog {
            client,
            dialog,
            heading,
            from,
            match_instead,
            query_entry,
            like_this_button,
            cadence,
            on,
            weekday,
            month_day,
            at,
            preview_heading,
            preview,
            more,
            note,
            error,
            create_words,
            keymap: RefCell::new(keymap.clone()),
            name: RefCell::default(),
            queries: RefCell::default(),
            replacing: RefCell::default(),
            like_this: RefCell::default(),
            generation: Cell::new(0),
            saved: RefCell::default(),
            me: RefCell::default(),
        });
        this.me.replace(Rc::downgrade(&this));
        let weak = Rc::downgrade(&this);
        cancel.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.dialog.close();
                }
            }
        });
        create.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.create();
                }
            }
        });
        this.at.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.create();
                }
            }
        });
        this.cadence.connect_selected_notify({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.show_day();
                }
            }
        });
        this.match_instead.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.match_instead();
                }
            }
        });
        this.query_entry.connect_changed({
            let weak = weak.clone();
            move |entry| {
                if let Some(this) = weak.upgrade() {
                    this.apply_query(&entry.text());
                }
            }
        });
        this.like_this_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.like_this();
                }
            }
        });
        this.show_create();
        this
    }

    /// Run `saved` with the rule's name once Create has written it.
    pub fn connect_saved(&self, saved: impl Fn(String) + 'static) {
        self.saved.replace(Some(Rc::new(saved)));
    }

    /// The dialog, for the window to find.
    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    /// Open over `parent` for a new rule holding mail from `senders`.
    /// `like_this`, when given, is the message "Digest mail like this"
    /// checks other mail against -- the window gives one only once the
    /// user has brought a model with `like_this` on (US14, FR-171); with
    /// none, the control is absent.
    pub fn open_new(
        &self,
        parent: &impl IsA<gtk::Widget>,
        senders: &[EmailAddress],
        like_this: Option<MessageId>,
    ) {
        let names: Vec<String> = senders
            .iter()
            .map(|sender| sender.display().to_owned())
            .collect();
        self.name.replace(digest::rule_name(&names));
        self.queries.replace(digest::sender_queries(senders));
        self.replacing.replace(None);
        self.like_this.replace(like_this);
        self.heading
            .set_text(digest::new_rule_heading(senders.len()));
        self.show_senders(&digest::sender_line(senders));
        self.note.set_text(digest::rule_note(senders.len()));
        self.show_schedule(&digest::Schedule::new_rule());
        self.open(parent);
    }

    /// Open over `parent` to edit `rule` where it stands in `config.toml`.
    /// "Digest mail like this" is never offered while editing (US14 is
    /// about a new rule from a selected message).
    pub fn open_edit(&self, parent: &impl IsA<gtk::Widget>, rule: &DigestRule) {
        self.name.replace(rule.name.clone());
        self.queries.replace(rule.queries.clone());
        self.replacing.replace(Some(rule.name.clone()));
        self.like_this.replace(None);
        self.heading
            .set_text(&digest::edit_rule_heading(&rule.name));
        // A rule whose every query is `from:` reads as senders, editable
        // through the plain label; anything else -- a `list:` or free
        // query -- opens straight into the query entry it was made with.
        if digest::is_sender_rule(&rule.queries) {
            self.show_senders(&rule.queries.join(", "));
        } else {
            self.enter_query_mode(&rule.queries.join(", "));
        }
        self.note.set_text(digest::rule_note(rule.queries.len()));
        if let Ok(due) = rule.due() {
            self.show_schedule(&digest::Schedule::of(&due));
        }
        self.open(parent);
    }

    /// Put `schedule` in the cadence, day and time controls.
    fn show_schedule(&self, schedule: &digest::Schedule) {
        self.cadence.set_selected(schedule.cadence as u32);
        self.weekday.set_selected(schedule.weekday as u32);
        self.month_day.set_selected(schedule.month_day as u32);
        self.at.set_text(&schedule.at);
    }

    /// Show `text` as the plain-senders "From" line: the label, with the
    /// query entry, its link and "Digest mail like this" all put away.
    fn show_senders(&self, text: &str) {
        self.from.set_text(text);
        self.from.set_visible(true);
        self.query_entry.set_visible(false);
        self.match_instead.set_visible(true);
        self.like_this_button
            .set_visible(self.like_this.borrow().is_some());
    }

    /// "Match a list or a search instead…": swap the senders' label for a
    /// typed query, starting from `text` (US14 scenario 1).
    fn enter_query_mode(&self, text: &str) {
        self.from.set_visible(false);
        self.match_instead.set_visible(false);
        self.like_this_button.set_visible(false);
        self.query_entry.set_visible(true);
        self.query_entry.set_text(text);
        self.query_entry.grab_focus();
        self.apply_query(text);
    }

    /// What typing into the query entry does: one query per comma-
    /// separated piece becomes the rule's `match` list, previewed the same
    /// way a sender's `from:` is (US14 scenario 2, ADR 0008: the one query
    /// language).
    fn apply_query(&self, text: &str) {
        let queries = digest::split_queries(text);
        self.name.replace(queries.join(", "));
        self.queries.replace(queries);
        self.read_preview();
    }

    /// "Match a list or a search instead…", clicked.
    pub fn match_instead(&self) {
        self.enter_query_mode("");
    }

    /// Set the query entry's text, as if typed: what a test drives instead
    /// of a keystroke-by-keystroke `Entry`.
    pub fn set_query(&self, text: &str) {
        self.query_entry.set_text(text);
    }

    /// "Digest mail like this", clicked: ask the user's model which
    /// candidate query is alike, and preview it once it answers. Absent
    /// when `like_this` names no message (US14, FR-171); a model that
    /// finds nothing alike, or answers with none, says so rather than
    /// entering query mode on nothing.
    pub fn like_this(&self) {
        let Some(message) = *self.like_this.borrow() else {
            return;
        };
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.digest_like_this(message).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match read {
                Ok(Some(rule)) => this.enter_query_mode(&rule.queries.join(", ")),
                Ok(None) => this.say(Some(digest::NOTHING_ALIKE)),
                Err(error) => this.say(Some(&error.to_string())),
            }
        });
    }

    /// Create: write the rule, then close; or say why not.
    pub fn create(&self) {
        let draft = match self.draft() {
            Ok(draft) => draft,
            Err(sentence) => {
                self.say(Some(&sentence));
                return;
            }
        };
        let replacing = self.replacing.borrow().clone();
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        let name = draft.name.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let written = client.save_digest_rule(replacing, draft).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match written {
                Ok(()) => {
                    this.dialog.close();
                    let saved = this.saved.borrow().clone();
                    if let Some(saved) = saved {
                        saved(name);
                    }
                }
                Err(error) => this.say(Some(&error.to_string())),
            }
        });
    }

    /// Every text the dialog shows, in order: what a test reads.
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

    fn open(&self, parent: &impl IsA<gtk::Widget>) {
        self.say(None);
        self.show_day();
        self.show_create();
        self.preview_heading.set_text("");
        while let Some(child) = self.preview.first_child() {
            self.preview.remove(&child);
        }
        self.more.set_visible(false);
        self.dialog.present(Some(parent));
        self.read_preview();
    }

    /// The day menu the cadence asks for: a weekday, a day of the month,
    /// or none.
    fn show_day(&self) {
        let cadence = CADENCES[self.cadence.selected().min(2) as usize].0;
        self.on.set_visible(cadence != Cadence::Daily);
        self.weekday.set_visible(cadence == Cadence::Weekly);
        self.month_day.set_visible(cadence == Cadence::Monthly);
    }

    fn show_create(&self) {
        while let Some(child) = self.create_words.first_child() {
            self.create_words.remove(&child);
        }
        let words = digest::create_words(self.replacing.borrow().is_some());
        self.create_words.append(&gtk::Label::new(Some(words)));
        if let Some(key) = hints::key(&self.keymap.borrow(), CommandId::PickerConfirm) {
            self.create_words.append(&keyhint::cap(&key));
        }
    }

    fn say(&self, sentence: Option<&str>) {
        self.error.set_text(sentence.unwrap_or_default());
        self.error.set_visible(sentence.is_some());
    }

    /// The rule as the dialog says it, or why it cannot be one.
    fn draft(&self) -> Result<DigestRuleDraft, String> {
        let due = digest::Schedule {
            cadence: self.cadence.selected() as usize,
            weekday: self.weekday.selected() as usize,
            month_day: self.month_day.selected() as usize,
            at: self.at.text().to_string(),
        }
        .due()?;
        let (cadence, day, at) = match due {
            Due::Daily { at } => (Cadence::Daily, None, at),
            Due::Weekly { day, at } => (Cadence::Weekly, Some(RuleDay::Weekday(day)), at),
            Due::Monthly { day, at } => (Cadence::Monthly, Some(RuleDay::OfMonth(day)), at),
        };
        Ok(DigestRuleDraft {
            name: self.name.borrow().clone(),
            queries: self.queries.borrow().clone(),
            cadence,
            day,
            at,
        })
    }

    /// What the rule would have caught over the last 90 days.
    fn read_preview(&self) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let queries = self.queries.borrow().clone();
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        let since = postio_ui::clock::now().to_utc() - chrono::Duration::days(PREVIEW_DAYS);
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: as `create`'s.
            let read = client.digest_preview(queries, since).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.generation.get() != generation {
                return;
            }
            let preview = match read {
                Ok(preview) => preview,
                Err(error) => {
                    this.say(Some(&error.to_string()));
                    return;
                }
            };
            this.preview_heading
                .set_text(&digest::preview_heading(preview.count));
            let now = postio_ui::clock::now();
            for row in &preview.first {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
                line.add_css_class("focus-rule-preview-row");
                let subject = gtk::Label::new(row.subject.as_deref());
                subject.set_xalign(0.0);
                subject.set_hexpand(true);
                subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
                line.append(&subject);
                let day = gtk::Label::new(Some(&digest::preview_day(
                    row.received_at.with_timezone(&chrono::Local),
                    now,
                )));
                day.add_css_class("dim-label");
                line.append(&day);
                this.preview.append(&line);
            }
            this.preview.set_visible(!preview.first.is_empty());
            match digest::preview_more(preview.count, preview.first.len()) {
                Some(more) => {
                    this.more.set_text(&more);
                    this.more.set_visible(true);
                }
                None => this.more.set_visible(false),
            }
        });
    }
}
