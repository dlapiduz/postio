//! The results' Files tab and Quick Look (spec 010 steps 6 and 9, design
//! §3.7 and §3.8, screens 10 and 11).
//!
//! The Files tab is a grid of cards in the rows' place, each read by
//! position when the grid draws it, as the rows are. Quick Look is a panel
//! over the results showing one hit and every match in its conversation.
//! Both are the controller's to fill: which card has the ring, which match
//! card is current, what the panel's buttons are. This draws them, and
//! says a click back.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_core::CommandId;
use postio_focus::{FileCard, FilesHeader, Input, QuickLookView};
use postio_ui::search_view::FilePreview;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

use crate::list::Feed;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Positions {
        pub total: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Positions {
        const NAME: &'static str = "PostioResultPositions";
        type Type = super::Positions;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for Positions {}

    impl ListModelImpl for Positions {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.total.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position < self.total.get())
                .then(|| gtk::StringObject::new(&position.to_string()).upcast())
        }
    }
}

glib::wrapper! {
    /// How many cards or people a tab holds; each is read when drawn.
    pub struct Positions(ObjectSubclass<imp::Positions>)
        @implements gio::ListModel;
}

impl Default for Positions {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Positions {
    fn set_total(&self, total: u32) {
        let before = self.imp().total.replace(total);
        self.items_changed(0, before, total);
    }

    fn refresh(&self) {
        let total = self.imp().total.get();
        self.items_changed(0, total, total);
    }
}

/// The Files tab: its header, and the grid.
pub struct FilesGrid {
    root: gtk::Box,
    title: gtk::Label,
    note: gtk::Label,
    cards: Positions,
    grid: gtk::GridView,
    feed: Feed,
    act: Rc<dyn Fn(CommandId)>,
}

impl FilesGrid {
    /// An empty grid over `feed`.
    pub fn new(feed: Feed, act: Rc<dyn Fn(CommandId)>) -> Rc<Self> {
        let title = gtk::Label::new(None);
        title.add_css_class("focus-files-title");
        title.set_xalign(0.0);
        let note = gtk::Label::new(None);
        note.add_css_class("dim-label");
        note.add_css_class("focus-files-note");
        note.set_xalign(0.0);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        head.add_css_class("focus-files-head");
        head.append(&title);
        head.append(&note);
        let cards = Positions::default();
        let factory = gtk::SignalListItemFactory::new();
        let grid = gtk::GridView::new(
            Some(gtk::NoSelection::new(Some(cards.clone()))),
            Some(factory.clone()),
        );
        grid.add_css_class("focus-files-grid");
        grid.set_max_columns(4);
        grid.set_min_columns(2);
        grid.set_single_click_activate(false);
        grid.update_property(&[gtk::accessible::Property::Label(
            postio_ui::search_view::A11Y_FILES,
        )]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&grid)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-files");
        root.append(&head);
        root.append(&scrolled);
        let files = Rc::new(FilesGrid {
            root,
            title,
            note,
            cards,
            grid,
            feed,
            act,
        });
        let weak = Rc::downgrade(&files);
        factory.connect_bind({
            let weak = weak.clone();
            move |_, item| {
                let (Some(item), Some(files)) =
                    (item.downcast_ref::<gtk::ListItem>(), weak.upgrade())
                else {
                    return;
                };
                let card = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .and_then(|object| object.string().parse::<u64>().ok())
                    .and_then(|at| files.feed.result_file(at).map(|card| (at, card)));
                match card {
                    Some((at, card)) => {
                        item.set_accessible_label(&card.accessible);
                        item.set_child(Some(&files.card(at, &card)));
                    }
                    None => item.set_child(None::<&gtk::Widget>),
                }
            }
        });
        files.grid.connect_activate(move |_, at| {
            if let Some(files) = weak.upgrade() {
                files.feed.input(Input::ResultsPoint(u64::from(at)));
                (files.act)(CommandId::OpenMessage);
            }
        });
        files
    }

    /// What the results pane lays out.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Draw the header and `count` cards, every drawn card read again.
    pub fn show(&self, header: &FilesHeader, count: u64) {
        self.title.set_text(&header.title);
        self.note.set_text(&header.note);
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        if self.cards.n_items() == count {
            self.cards.refresh();
        } else {
            self.cards.set_total(count);
        }
    }

    /// The cards drawn again: a page landed, or the ring moved.
    pub fn refresh(&self) {
        self.cards.refresh();
    }

    /// Take the keyboard.
    pub fn focus(&self) {
        self.grid.grab_focus();
    }

    /// The cards' names, in order: what a test reads.
    pub fn names(&self) -> Vec<String> {
        (0..u64::from(self.cards.n_items()))
            .filter_map(|at| self.feed.result_file(at))
            .map(|card| card.name.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    /// The card with the ring.
    pub fn focused(&self) -> Option<u64> {
        (0..u64::from(self.cards.n_items()))
            .find(|at| self.feed.result_file(*at).is_some_and(|card| card.focused))
    }

    /// One card: the type tile over its preview, the name, who and when,
    /// the matching line, and the message it came in.
    fn card(&self, at: u64, card: &FileCard) -> gtk::Box {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, S1);
        tile.add_css_class("focus-file-preview");
        tile.add_css_class(match card.preview {
            FilePreview::Sheet => "focus-file-sheet",
            FilePreview::Page => "focus-file-page",
            FilePreview::Slides => "focus-file-slides",
            FilePreview::Image => "focus-file-image",
            FilePreview::Text => "focus-file-text",
        });
        let kind = gtk::Label::new(Some(&card.kind));
        kind.add_css_class("focus-file-kind");
        kind.set_halign(gtk::Align::Start);
        tile.append(&kind);
        // The preview's lines: quiet bars, the matching one marked.
        for line in 0..4u32 {
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            bar.add_css_class("focus-file-line");
            if card.marked == Some(line) {
                bar.add_css_class("focus-file-line-marked");
            }
            tile.append(&bar);
        }
        let name = crate::dropdown::runs_label(&card.name, false);
        name.add_css_class("focus-file-name");
        let meta = gtk::Label::new(Some(&card.meta));
        meta.add_css_class("dim-label");
        meta.add_css_class("focus-file-meta");
        meta.set_xalign(0.0);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let root = gtk::Box::new(gtk::Orientation::Vertical, S1);
        root.add_css_class("focus-file-card");
        if card.focused {
            root.add_css_class("focus-file-focused");
        }
        root.append(&tile);
        root.append(&name);
        root.append(&meta);
        if !card.line.is_empty() {
            let line = crate::dropdown::runs_label(&card.line, true);
            line.add_css_class("focus-file-match");
            root.append(&line);
        }
        let subject = gtk::Label::new(Some(&card.subject));
        subject.add_css_class("dim-label");
        subject.add_css_class("focus-file-subject");
        subject.set_xalign(0.0);
        subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&subject);
        let click = gtk::GestureClick::new();
        let feed = self.feed.clone();
        click.connect_pressed(move |_, _, _, _| feed.input(Input::ResultsPoint(at)));
        root.add_controller(click);
        root
    }
}

/// The People tab (design §3.11): everyone the results are from or to,
/// most messages first, a row a person in the rows' place.
pub struct PeopleList {
    root: gtk::ScrolledWindow,
    people: Positions,
    list: gtk::ListView,
    feed: Feed,
}

impl PeopleList {
    /// An empty list over `feed`; `act` runs Return's command on a
    /// double-click.
    pub fn new(feed: Feed, act: Rc<dyn Fn(CommandId)>) -> Rc<Self> {
        let people = Positions::default();
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(people.clone()))),
            Some(factory.clone()),
        );
        list.add_css_class("focus-results-list");
        list.add_css_class("focus-people-list");
        list.set_single_click_activate(false);
        list.update_property(&[gtk::accessible::Property::Label(
            postio_ui::search_view::A11Y_PEOPLE,
        )]);
        let root = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let pane = Rc::new(PeopleList {
            root,
            people,
            list,
            feed,
        });
        let weak = Rc::downgrade(&pane);
        factory.connect_bind({
            let weak = weak.clone();
            move |_, item| {
                let (Some(item), Some(pane)) =
                    (item.downcast_ref::<gtk::ListItem>(), weak.upgrade())
                else {
                    return;
                };
                let row = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .and_then(|object| object.string().parse::<u64>().ok())
                    .and_then(|at| pane.feed.result_person(at).map(|row| (at, row)));
                match row {
                    Some((at, row)) => {
                        item.set_accessible_label(&row.accessible);
                        item.set_child(Some(&pane.row(at, &row)));
                    }
                    None => item.set_child(None::<&gtk::Widget>),
                }
            }
        });
        pane.list.connect_activate(move |_, at| {
            if let Some(pane) = weak.upgrade() {
                pane.feed.input(Input::ResultsPoint(u64::from(at)));
                act(CommandId::OpenMessage);
            }
        });
        pane
    }

    /// What the results pane lays out.
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    /// `count` people, every drawn row read again.
    pub fn show(&self, count: u64) {
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        if self.people.n_items() == count {
            self.people.refresh();
        } else {
            self.people.set_total(count);
        }
    }

    /// The rows drawn again: the ring moved.
    pub fn refresh(&self) {
        self.people.refresh();
    }

    /// Scroll the row at `at` into view.
    pub fn reveal(&self, at: u64) {
        if let Ok(at) = u32::try_from(at)
            && at < self.people.n_items()
        {
            self.list.scroll_to(at, gtk::ListScrollFlags::NONE, None);
        }
    }

    /// Take the keyboard.
    pub fn focus(&self) {
        self.list.grab_focus();
    }

    /// The people's names, in order: what a test reads.
    pub fn names(&self) -> Vec<String> {
        (0..u64::from(self.people.n_items()))
            .filter_map(|at| self.feed.result_person(at))
            .map(|row| row.name)
            .collect()
    }

    /// One person: the avatar, the name over the address, how many
    /// messages, the newest one's date.
    fn row(&self, at: u64, row: &postio_focus::PersonRow) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        line.add_css_class("focus-results-row");
        line.add_css_class("focus-people-row");
        if row.focused {
            line.add_css_class("focus-results-cursor");
        }
        let avatar = gtk::Label::new(Some(&row.initials));
        avatar.add_css_class("focus-dropdown-avatar");
        avatar.add_css_class("focus-people-avatar");
        avatar.set_accessible_role(gtk::AccessibleRole::Presentation);
        avatar.set_valign(gtk::Align::Center);
        line.append(&avatar);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        words.set_hexpand(true);
        words.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(Some(&row.name));
        name.set_xalign(0.0);
        name.add_css_class("focus-people-name");
        words.append(&name);
        if row.address != row.name {
            let address = gtk::Label::new(Some(&row.address));
            address.add_css_class("dim-label");
            address.add_css_class("focus-people-address");
            address.set_xalign(0.0);
            words.append(&address);
        }
        line.append(&words);
        let messages = gtk::Label::new(Some(&row.messages));
        messages.add_css_class("dim-label");
        messages.add_css_class("focus-people-messages");
        messages.set_valign(gtk::Align::Center);
        line.append(&messages);
        let last = gtk::Label::new(Some(&row.last));
        last.add_css_class("focus-results-date");
        last.set_xalign(1.0);
        last.set_valign(gtk::Align::Center);
        line.append(&last);
        let click = gtk::GestureClick::new();
        let feed = self.feed.clone();
        click.connect_pressed(move |_, _, _, _| feed.input(Input::ResultsPoint(at)));
        line.add_controller(click);
        line
    }
}

/// Quick Look over the results.
pub struct QuickLookPanel {
    root: gtk::Box,
    title: gtk::Label,
    position: gtk::Label,
    actions: gtk::Box,
    subject: gtk::Label,
    sender: gtk::Label,
    matches_line: gtk::Label,
    cards: gtk::Box,
    act: Rc<dyn Fn(CommandId)>,
    me: std::cell::RefCell<Weak<QuickLookPanel>>,
}

impl QuickLookPanel {
    /// A closed panel; `act` runs a header button's command.
    pub fn new(act: Rc<dyn Fn(CommandId)>) -> Rc<Self> {
        let title = gtk::Label::new(None);
        title.add_css_class("focus-quicklook-title");
        let position = gtk::Label::new(None);
        position.add_css_class("dim-label");
        position.set_hexpand(true);
        position.set_xalign(0.0);
        position.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        head.add_css_class("focus-quicklook-head");
        head.append(&title);
        head.append(&position);
        head.append(&actions);
        let subject = gtk::Label::new(None);
        subject.add_css_class("focus-quicklook-subject");
        subject.set_xalign(0.0);
        subject.set_wrap(true);
        let sender = gtk::Label::new(None);
        sender.add_css_class("focus-quicklook-sender");
        sender.set_xalign(0.0);
        let matches_line = gtk::Label::new(None);
        matches_line.add_css_class("dim-label");
        matches_line.add_css_class("focus-quicklook-matches");
        matches_line.set_xalign(0.0);
        let cards = gtk::Box::new(gtk::Orientation::Vertical, S2);
        cards.add_css_class("focus-quicklook-cards");
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&cards)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, S2);
        body.add_css_class("focus-quicklook-body");
        body.append(&subject);
        body.append(&sender);
        body.append(&matches_line);
        body.append(&scrolled);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-quicklook");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        root.set_width_request(720);
        root.append(&head);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&body);
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Dialog);
        let panel = Rc::new(QuickLookPanel {
            root,
            title,
            position,
            actions,
            subject,
            sender,
            matches_line,
            cards,
            act,
            me: std::cell::RefCell::default(),
        });
        panel.me.replace(Rc::downgrade(&panel));
        panel
    }

    /// What the results pane lays over its rows.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Draw `view`, or close with `None`: `Intent::QuickLook`. `commands`
    /// are what its buttons run, in order
    /// ([`postio_ui::search_view::quick_look_commands`]).
    pub fn show(&self, view: Option<&QuickLookView>, commands: &[CommandId]) {
        let Some(view) = view else {
            self.root.set_visible(false);
            return;
        };
        self.title.set_text(&view.title);
        self.position.set_text(&match &view.walk {
            Some(walk) => format!("{} \u{b7} {} {}", view.position, walk.key, walk.label),
            None => view.position.clone(),
        });
        while let Some(child) = self.actions.first_child() {
            self.actions.remove(&child);
        }
        for (at, action) in view.actions.iter().enumerate() {
            let button = gtk::Button::new();
            keyhint::dress(&button, &action.label, Some(&action.key));
            button.add_css_class("focus-quicklook-action");
            button.set_focus_on_click(false);
            if let Some(command) = commands.get(at).copied() {
                let weak = self.me.borrow().clone();
                button.connect_clicked(move |_| {
                    if let Some(panel) = weak.upgrade() {
                        (panel.act)(command);
                    }
                });
            }
            self.actions.append(&button);
        }
        self.subject.set_markup(&markup(&view.subject));
        self.sender.set_markup(&markup(&view.sender));
        self.matches_line.set_text(&match &view.matches_hint {
            Some(hint) => format!("{} \u{b7} {} {}", view.matches_line, hint.key, hint.label),
            None => view.matches_line.clone(),
        });
        while let Some(child) = self.cards.first_child() {
            self.cards.remove(&child);
        }
        for (at, card) in view.cards.iter().enumerate() {
            let place = gtk::Label::new(Some(&card.place));
            place.add_css_class("focus-quicklook-place");
            if card.file {
                place.add_css_class("focus-results-source-file");
            }
            place.set_xalign(0.0);
            let when = gtk::Label::new(Some(&card.when));
            when.add_css_class("dim-label");
            when.set_xalign(0.0);
            let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
            left.add_css_class("focus-quicklook-where");
            left.append(&place);
            left.append(&when);
            let passage = crate::dropdown::runs_label(&card.passage, false);
            passage.set_single_line_mode(false);
            passage.set_wrap(true);
            passage.set_ellipsize(gtk::pango::EllipsizeMode::None);
            passage.set_hexpand(true);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-quicklook-card");
            if view.current == Some(at as u32) {
                line.add_css_class("focus-quicklook-current");
            }
            line.append(&left);
            line.append(&passage);
            self.cards.append(&line);
        }
        self.root.set_visible(true);
    }

    /// Whether it is up.
    pub fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// The match cards' places, and which is current: what a test reads.
    pub fn cards(&self) -> Vec<(String, bool)> {
        let mut cards = Vec::new();
        let mut child = self.cards.first_child();
        while let Some(card) = child {
            let place = card
                .first_child()
                .and_then(|left| left.first_child())
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string())
                .unwrap_or_default();
            cards.push((place, card.has_css_class("focus-quicklook-current")));
            child = card.next_sibling();
        }
        cards
    }
}

/// `runs` as markup: strong, mono and the find highlight.
fn markup(runs: &[postio_focus::Run]) -> String {
    crate::dropdown::runs_label(runs, false).label().to_string()
}
