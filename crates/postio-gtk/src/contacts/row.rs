//! A person in the Contacts list, drawn in one `snapshot()`.
//!
//! The same technique as the message row (`crate::row`), for the same
//! reasons: a list of twenty thousand people scrolls at the speed text and
//! rectangles draw, and every colour and face is read off an invisible probe
//! label wearing the message row's own CSS roles, so the tokens, dark mode
//! and high contrast reach this row exactly as they reach that one.
//!
//! The anatomy: an initials chip, the person's name with a `saved` mark for
//! the ones the user made or imported (FR-005), and beneath it how many
//! addresses they own and when they were last in touch. The cursor is the
//! 3px accent edge, the selection its own ground -- two devices, so a row
//! that is both shows both.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::Local;
use gtk::{gdk, glib, graphene, gsk, pango};
use postio_config::Density;
use postio_model::{ContactListRow, EmailAddress};
use postio_ui::row::Metrics;

use crate::row::IconLookup;

/// One role's resolved paint.
#[derive(Clone, Debug)]
struct Ink {
    color: gdk::RGBA,
    font: pango::FontDescription,
}

/// Everything the row paints with, read off the cascade. Index 0 is a plain
/// row, 1 a selected one.
struct Palette {
    name: [Ink; 2],
    detail: [Ink; 2],
    avatar: [Ink; 2],
    badge: Ink,
    hairline: gdk::RGBA,
    hairline_strong: gdk::RGBA,
    cursor_edge: gdk::RGBA,
    cursor_bg: gdk::RGBA,
    selected_bg: gdk::RGBA,
    hover_bg: gdk::RGBA,
    checked_mark: gdk::RGBA,
    check: Option<gtk::IconPaintable>,
}

impl Palette {
    fn read(probe: &gtk::Label) -> Self {
        let ink = |classes: &[&str]| {
            probe.set_css_classes(classes);
            Ink {
                color: probe.color(),
                font: probe
                    .create_pango_context()
                    .font_description()
                    .unwrap_or_default(),
            }
        };
        let two = |role: &str| [ink(&[role]), ink(&[role, "selected"])];
        // A person's name at full strength: the message row dims a read
        // sender, and nothing about a person is "read".
        let name = [
            ink(&["postio-row-sender", "unread"]),
            ink(&["postio-row-sender", "unread", "selected"]),
        ];
        let paint = |classes: &[&str]| {
            probe.set_css_classes(classes);
            probe.color()
        };
        let palette = Palette {
            // The message row's roles: a person's name is what a sender line
            // is, and the address count sits where the snippet does.
            name,
            detail: two("postio-row-snippet"),
            avatar: two("postio-row-avatar"),
            badge: ink(&["postio-row-badge"]),
            hairline: paint(&["postio-row-edge", "hairline"]),
            hairline_strong: paint(&["postio-row-edge", "hairline", "strong"]),
            cursor_edge: paint(&["postio-row-edge", "selected"]),
            cursor_bg: paint(&["postio-row-ground", "selected"]),
            selected_bg: paint(&["postio-row-ground", "checked"]),
            hover_bg: paint(&["postio-row-ground", "hover"]),
            checked_mark: paint(&["postio-row-ground", "check-mark"]),
            check: probe.display().pipe_icon("object-select-symbolic"),
        };
        probe.set_css_classes(&[]);
        palette
    }
}

/// The word the made-or-imported mark says.
const SAVED: &str = "saved";

/// Between the name and the mark beside it.
const RUN: f32 = 8.0;

/// The cursor's accent edge.
const EDGE: f32 = 3.0;

/// The mark's padding, horizontal then vertical.
const CAP: (f32, f32) = (4.0, 1.0);

mod imp {
    use super::*;

    pub struct ContactRowView {
        pub(super) row: RefCell<Option<ContactListRow>>,
        pub(super) density: Cell<Density>,
        pub(super) cursor: Cell<bool>,
        pub(super) selected: Cell<bool>,
        pub(super) hovered: Cell<bool>,
        pub(super) first: Cell<bool>,
        pub(super) probe: gtk::Label,
        pub(super) sentinel: gtk::Label,
        pub(super) palette: RefCell<Option<Rc<Palette>>>,
    }

    impl Default for ContactRowView {
        fn default() -> Self {
            Self {
                row: RefCell::new(None),
                density: Cell::new(Density::default()),
                cursor: Cell::new(false),
                selected: Cell::new(false),
                hovered: Cell::new(false),
                first: Cell::new(false),
                probe: gtk::Label::new(None),
                sentinel: gtk::Label::new(None),
                palette: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactRowView {
        const NAME: &'static str = "PostioContactRowView";
        type Type = super::ContactRowView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ContactRowView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_css_class("postio-row");
            obj.add_css_class("postio-contact-row");
            // The list item around it carries the role and the name a screen
            // reader navigates by, as the message row's does.
            obj.set_accessible_role(gtk::AccessibleRole::Presentation);
            for probe in [&self.probe, &self.sentinel] {
                probe.set_child_visible(false);
                probe.set_parent(&*obj);
            }
            self.sentinel
                .set_css_classes(&["postio-row-edge", "hairline"]);

            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(
                #[weak(rename_to = row)]
                obj,
                move |_, _, _| row.set_hovered(true)
            ));
            motion.connect_leave(glib::clone!(
                #[weak(rename_to = row)]
                obj,
                move |_| row.set_hovered(false)
            ));
            obj.add_controller(motion);
        }

        fn dispose(&self) {
            self.probe.unparent();
            self.sentinel.unparent();
        }
    }

    impl WidgetImpl for ContactRowView {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let metrics = Metrics::for_density(self.density.get());
            if orientation == gtk::Orientation::Horizontal {
                let least = (metrics.inset * 2.0 + metrics.avatar + metrics.gap) as i32 + 96;
                return (least, least, -1, -1);
            }
            let height = self.obj().height_for(&metrics).ceil() as i32;
            (height, height, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }

        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.palette.replace(None);
            self.obj().queue_resize();
        }
    }
}

glib::wrapper! {
    /// One person in the Contacts list, drawn in a single `snapshot()`.
    pub struct ContactRowView(ObjectSubclass<imp::ContactRowView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ContactRowView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ContactRowView {
    /// A row with nothing bound: it draws a skeleton.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds a person, or `None` for a position whose page is on its way.
    pub fn set_row(&self, row: Option<ContactListRow>) {
        self.imp().row.replace(row);
        self.queue_draw();
    }

    /// The bound person.
    pub fn row(&self) -> Option<ContactListRow> {
        self.imp().row.borrow().clone()
    }

    /// What a screen reader says for this row: the name, the mark, the line.
    pub fn spoken(&self) -> String {
        self.imp()
            .row
            .borrow()
            .as_ref()
            .map(spoken)
            .unwrap_or_else(|| "Loading".to_owned())
    }

    /// Whether the keyboard is on this row.
    pub fn set_cursor(&self, cursor: bool) {
        if self.imp().cursor.replace(cursor) != cursor {
            self.queue_draw();
        }
    }

    /// Whether this row is in the selection a join would act on.
    /// Whether the row is drawn as marked.
    pub fn is_marked(&self) -> bool {
        self.imp().selected.get()
    }

    pub fn set_selected(&self, selected: bool) {
        if self.imp().selected.replace(selected) != selected {
            self.queue_draw();
        }
    }

    /// Whether this is the first row, which draws no rule above itself.
    pub fn set_first(&self, first: bool) {
        self.imp().first.set(first);
    }

    /// The density the row is laid out at.
    pub fn set_density(&self, density: Density) {
        if self.imp().density.replace(density) != density {
            self.queue_resize();
        }
    }

    fn set_hovered(&self, hovered: bool) {
        if self.imp().hovered.replace(hovered) != hovered {
            self.queue_draw();
        }
    }

    fn palette(&self) -> Rc<Palette> {
        let imp = self.imp();
        let hairline = imp.sentinel.color();
        if let Some(palette) = imp.palette.borrow().clone()
            && palette.hairline == hairline
        {
            return palette;
        }
        let palette = Rc::new(Palette::read(&imp.probe));
        imp.palette.replace(Some(palette.clone()));
        palette
    }

    fn layout(&self, ink: &Ink, text: &str) -> pango::Layout {
        let layout = pango::Layout::new(&self.pango_context());
        layout.set_font_description(Some(&ink.font));
        layout.set_text(text);
        layout
    }

    /// Two lines of text, or the avatar if it is taller.
    fn height_for(&self, metrics: &Metrics) -> f32 {
        let palette = self.palette();
        let name = self.layout(&palette.name[0], "Ag").pixel_size().1 as f32;
        let detail = self.layout(&palette.detail[0], "Ag").pixel_size().1 as f32;
        metrics.pad_y * 2.0 + (name + metrics.subject_gap + detail).max(metrics.avatar)
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let width = self.width() as f32;
        let height = self.height() as f32;
        if width <= 0.0 {
            return;
        }
        let palette = self.palette();
        let metrics = Metrics::for_density(imp.density.get());
        let rect = |x: f32, y: f32, w: f32, h: f32| graphene::Rect::new(x, y, w, h);
        let fill = |color: &gdk::RGBA, x, y, w, h| snapshot.append_color(color, &rect(x, y, w, h));
        let selected = imp.selected.get();
        let cursor = imp.cursor.get();

        if selected {
            fill(&palette.selected_bg, 0.0, 0.0, width, height);
        } else if cursor {
            fill(&palette.cursor_bg, 0.0, 0.0, width, height);
        } else if imp.hovered.get() {
            fill(&palette.hover_bg, 0.0, 0.0, width, height);
        }
        if cursor {
            fill(&palette.cursor_edge, 0.0, 0.0, EDGE, height);
        }
        if !imp.first.get() {
            let rule = if selected || cursor {
                &palette.hairline_strong
            } else {
                &palette.hairline
            };
            fill(rule, 0.0, 0.0, width, 1.0);
        }

        let row = imp.row.borrow();
        let column_x = metrics.inset + metrics.avatar + metrics.gap;
        let column = (width - column_x - metrics.inset).max(40.0);
        let Some(row) = row.as_ref() else {
            // A skeleton says "a person is coming" without pretending to be one.
            let mut ghost = palette.hairline;
            ghost.set_alpha(ghost.alpha() * 0.6);
            let y = metrics.pad_y;
            fill(&ghost, metrics.inset, y, metrics.avatar, metrics.avatar);
            fill(&ghost, column_x, y + 2.0, column * 0.34, 9.0);
            fill(&ghost, column_x, y + 19.0, column * 0.6, 9.0);
            return;
        };
        let tone = usize::from(selected);
        let text = |layout: &pango::Layout, color: &gdk::RGBA, x: f32, y: f32| {
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x, y));
            snapshot.append_layout(layout, color);
            snapshot.restore();
        };

        // The chip: initials of the name the list shows -- or, for a person
        // in the selection, the check the message row puts in the same
        // square. The two grounds are a step apart in light and the same in
        // dark (canvas 3c), so the check is what says "marked".
        let chip = rect(metrics.inset, metrics.pad_y, metrics.avatar, metrics.avatar);
        if selected {
            snapshot.append_color(&palette.cursor_edge, &chip);
        }
        snapshot.append_border(
            &gsk::RoundedRect::from_rect(chip, 0.0),
            &[1.0; 4],
            &[if selected {
                palette.cursor_edge
            } else {
                palette.hairline
            }; 4],
        );
        match (selected, &palette.check) {
            (true, Some(check)) => {
                let size = (metrics.avatar * 0.62).round();
                let inset = ((metrics.avatar - size) / 2.0).round();
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    metrics.inset + inset,
                    metrics.pad_y + inset,
                ));
                check.snapshot_symbolic(
                    snapshot,
                    size as f64,
                    size as f64,
                    &[palette.checked_mark],
                );
                snapshot.restore();
            }
            // No check glyph in the theme: an empty filled square, never
            // initials that would say the one thing the mark overrides.
            (true, None) => {}
            (false, _) => {
                let initials = postio_ui::row::initials(Some(&EmailAddress::new(
                    Some(row.name.clone()),
                    row.preferred.clone().unwrap_or_default(),
                )));
                let avatar = self.layout(&palette.avatar[tone], &initials);
                let (aw, ah) = avatar.pixel_size();
                text(
                    &avatar,
                    &palette.avatar[tone].color,
                    metrics.inset + (metrics.avatar - aw as f32) / 2.0,
                    metrics.pad_y + (metrics.avatar - ah as f32) / 2.0,
                );
            }
        }

        // The name, with the mark beside it for the address book.
        let made = postio_ui::contacts::is_made(row.source);
        let badge = made.then(|| self.layout(&palette.badge, SAVED));
        let badge_w = badge
            .as_ref()
            .map(|badge| badge.pixel_size().0 as f32 + CAP.0 * 2.0 + RUN)
            .unwrap_or(0.0);
        let name = self.layout(&palette.name[tone], &row.name);
        name.set_ellipsize(pango::EllipsizeMode::End);
        name.set_width(((column - badge_w).max(24.0) * pango::SCALE as f32) as i32);
        let top = metrics.pad_y;
        text(&name, &palette.name[tone].color, column_x, top);
        if let Some(badge) = &badge {
            let (bw, bh) = badge.pixel_size();
            let x = column_x + name.pixel_size().0 as f32 + RUN;
            let base = name.baseline() as f32 / pango::SCALE as f32;
            let y = top + base - badge.baseline() as f32 / pango::SCALE as f32;
            snapshot.append_border(
                &gsk::RoundedRect::from_rect(
                    rect(
                        x,
                        y - CAP.1,
                        bw as f32 + CAP.0 * 2.0,
                        bh as f32 + CAP.1 * 2.0,
                    ),
                    0.0,
                ),
                &[1.0; 4],
                &[palette.hairline_strong; 4],
            );
            text(badge, &palette.badge.color, x + CAP.0, y);
        }

        // How many addresses, and when last in touch.
        let detail = self.layout(
            &palette.detail[tone],
            &postio_ui::contacts::secondary_line(row, Local::now()),
        );
        detail.set_ellipsize(pango::EllipsizeMode::End);
        detail.set_width((column * pango::SCALE as f32) as i32);
        let detail_y = top + name.pixel_size().1 as f32 + metrics.subject_gap;
        text(&detail, &palette.detail[tone].color, column_x, detail_y);
    }
}

/// What a screen reader says for `row`.
pub fn spoken(row: &ContactListRow) -> String {
    let mark = if postio_ui::contacts::is_made(row.source) {
        ", saved"
    } else {
        ""
    };
    format!(
        "{}{mark}, {}",
        row.name,
        postio_ui::contacts::secondary_line(row, Local::now())
    )
}
