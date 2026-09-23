//! The MIME tree: what a message is made of, walkable from the keyboard,
//! with nothing fetched or rendered until it is asked for.
//!
//! Canvas 3g. A message is a tree of parts, and a mail client that only ever
//! shows you the one part it decided to render is one you cannot check. This
//! panel shows the whole structure — type, name and size for every part — and
//! lets the keyboard walk it.
//!
//! # Structure before bytes
//!
//! Everything drawn here comes from `BODYSTRUCTURE`, which IMAP returns
//! without transferring a single byte of any part. [`Attachment`] already
//! carries exactly that: a `part_id` like `2.1`, a `mime_type`, a declared
//! `size`, and a `blob_id` that is `None` until the bytes have actually been
//! downloaded. So the tree is a *reading* of metadata the store already has,
//! and the panel can be complete and correct for a message nothing has been
//! fetched for.
//!
//! That is the point rather than an optimisation. "Nothing downloads until
//! the user asks" is a privacy promise (CLAUDE.md), and the way to keep it is
//! for the surface that shows attachments to have no way to fetch one: the
//! panel emits [`PartsPanel::connect_open`] and friends, and whoever owns the
//! store decides what that costs.
//!
//! # The tree comes from the part ids
//!
//! IMAP part ids are paths — `1`, `2`, `2.1`, `2.2` — so the nesting is
//! already in the data and [`tree`] only has to read it. Nothing here invents
//! a hierarchy or asks the store for one.
//!
//! # Where the answers are decided
//!
//! In [`postio_ui::reader::parts`], and re-exported below. This module is the
//! panel: the widget, its keys, its drag source and its dialogs. Everything
//! it *says* — the tree, the box drawing, each row's words, and the filename
//! a part may be written under — is shared with the macOS reader, which has
//! exactly the same things to say about exactly the same rows (ADR 0019).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use postio_core::{CommandId, Keymap};
use postio_model::Attachment;

// The tree, the words and the names moved to `postio_ui::reader::parts`.
//
// Every one of them was a pure function over `Attachment` rows with no widget
// in it, and the macOS reader needs the same answers — so keeping a copy here
// would have been the drift ADR 0019 exists to prevent, on the one rule where
// drifting is a security bug rather than an inconsistency: `save_name` is what
// stops a part called `../../.bashrc` steering a save, and a second sanitiser
// is a second chance to miss a separator.
//
// Re-exported rather than renamed at the call sites, the way `human_size`
// already was (#411): what this module *is* to the rest of `postio-gtk` and to
// `postio-app` has not changed, only where the answers are decided.
pub use postio_ui::format::human_size;
pub use postio_ui::reader::parts::{
    CONTAINER, NOT_FETCHED, Node, detail, first_part, held_back_note, note, prefix, previewable,
    root_type, save_all_failure, save_name, save_names, spoken, step, summary, tree,
};

// ---------------------------------------------------------------------------
// The panel — canvas 3g
// ---------------------------------------------------------------------------

/// The commands the panel's own keys hint at, and the label each gets — the
/// canvas' words, not the registry's titles.
///
/// `NextPart`/`PrevPart` are folded into one "walk" line below rather than
/// listed here, since a reader wants one entry for one verb even though it
/// takes two keys.
const HINT_COMMANDS: [(CommandId, &str); 5] = [
    (CommandId::OpenPart, "open"),
    (CommandId::SavePart, "save"),
    (CommandId::SaveAllParts, "save all"),
    (CommandId::OpenPartExternally, "xdg-open"),
    (CommandId::RenderPartOnce, "render once"),
];

/// The panel's own keys, drawn at the tree's foot — generated from `keymap`
/// rather than typed in once, so `postio-14b`'s fix cannot go stale the way
/// the footer it replaced already had: it never mentioned `H` at all.
fn hints_for(keymap: &Keymap) -> String {
    let mut parts = Vec::new();
    let walk = match (
        keymap.binding(CommandId::NextPart),
        keymap.binding(CommandId::PrevPart),
    ) {
        (Some(next), Some(prev)) => Some(format!("{next}/{prev} walk")),
        (Some(next), None) => Some(format!("{next} walk")),
        (None, Some(prev)) => Some(format!("{prev} walk")),
        (None, None) => None,
    };
    parts.extend(walk);
    for (id, label) in HINT_COMMANDS {
        if let Some(key) = keymap.binding(id) {
            parts.push(format!("{key} {label}"));
        }
    }
    parts.join(" · ")
}

/// The registry's own bindings, so a panel built with no keymap yet — every
/// widget test, and the first frame before `postio-gtk::config` reads
/// `config.toml` — still reads correctly rather than blank.
fn default_hints() -> String {
    hints_for(&Keymap::resolve(&Default::default()))
}

/// How wide the tree column is, from the artboard.
const TREE_WIDTH: i32 = 290;

/// How tall the tree is allowed to get before it scrolls instead of growing.
const TREE_MAX_HEIGHT: i32 = 360;

type NodeHandler = Box<dyn Fn(&Node)>;
/// A part, and where the user chose to put it.
/// What the panel needs a destination for.
///
/// The argument to a [`PartsPanel::connect_ask`] handler, which stands in for the
/// `GtkFileDialog` the panel would otherwise open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// `s` — where to put this one part.
    Part(Node),
    /// `S` — which folder to put every part in.
    Everything,
}

type AskHandler = Box<dyn Fn(&Ask)>;
type SaveHandler = Box<dyn Fn(&Node, &gio::File)>;
/// Where the user chose to put every part.
type SaveAllHandler = Box<dyn Fn(&gio::File)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PartsPanel {
        pub(super) summary: gtk::Label,
        pub(super) blocked: gtk::Label,
        pub(super) tree: gtk::ListBox,
        /// The panel's own keys, drawn at the tree's foot — generated from
        /// the live keymap by [`super::PartsPanel::set_keymap`] rather than typed
        /// in once, so a rebind in `config.toml` changes what this says.
        pub(super) keys: gtk::Label,
        pub(super) meta: gtk::Label,
        pub(super) note: gtk::Label,
        pub(super) render_once: gtk::Button,
        pub(super) save: gtk::Button,
        pub(super) external: gtk::Button,
        pub(super) nodes: RefCell<Vec<Node>>,
        /// How much the reader held back on this message: remote references,
        /// then trackers. Drawn as the canvas' `remote blocked` tag.
        pub(super) held_back: Cell<(u32, u32)>,
        pub(super) on_open: RefCell<Vec<NodeHandler>>,
        pub(super) on_ask: RefCell<Vec<AskHandler>>,
        pub(super) on_save: RefCell<Vec<SaveHandler>>,
        pub(super) on_save_all: RefCell<Vec<SaveAllHandler>>,
        pub(super) on_external: RefCell<Vec<NodeHandler>>,
        pub(super) on_render_once: RefCell<Vec<NodeHandler>>,
        /// How a dragged part becomes a file. Empty in a build that never
        /// wired it, in which case the panel offers nothing to drag.
        pub(super) export: RefCell<Option<crate::drag_out::MaterialisePart>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PartsPanel {
        const NAME: &'static str = "PostioPartsPanel";
        type Type = super::PartsPanel;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for PartsPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for PartsPanel {}
    impl BinImpl for PartsPanel {}
}

glib::wrapper! {
    /// What a message is made of, walkable from the keyboard.
    ///
    /// An overlay on the main window, the way the palette, the cheat sheet
    /// and the settings panel are — not a dialog. Nothing in a mail client is
    /// urgent enough to stop the rest of the application, and `Esc` closes
    /// this the same way it closes every other surface.
    pub struct PartsPanel(ObjectSubclass<imp::PartsPanel>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for PartsPanel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl PartsPanel {
    /// An empty panel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show `parts` as the structure of a message whose own type is `root`.
    ///
    /// Metadata only: this neither fetches nor renders anything, and there is
    /// no path from here that could. See the module docs.
    pub fn show_parts(&self, root: &str, parts: &[Attachment]) {
        self.draw(root, parts);
        // The first *part*, not the message: the row the user came to look at
        // is one of the things inside, and starting on the container would
        // cost a keystroke every time. `first_part` is where that is decided,
        // so the macOS panel opens on the same row rather than on its own
        // reading of the same sentence.
        let count = self.imp().nodes.borrow().len();
        self.select_index(first_part(count));
        self.refresh_detail();
    }

    /// Redraw the same message's tree, keeping the cursor where it is.
    ///
    /// What [`show_parts`](Self::show_parts) is for a message the user just
    /// opened, this is for one whose parts *changed under them*:
    /// `Node::downloaded` is written at runtime now (#377), so a chip that
    /// said "download" has to start saying "open" the moment the bytes land
    /// (#396).
    ///
    /// The cursor is what makes this a separate method. `show_parts` drops it
    /// on the first part, which is right on the way in and wrong here: the
    /// person who opened this panel is standing on the part they are waiting
    /// for, and a payload arriving must not move them off it. It follows the
    /// part id rather than the index, so a tree that gained or lost a row
    /// still leaves the cursor on the same part.
    pub fn update_parts(&self, root: &str, parts: &[Attachment]) {
        let was = self.cursor().map(|node| node.part_id);
        self.draw(root, parts);
        let index = was
            .and_then(|part_id| {
                self.imp()
                    .nodes
                    .borrow()
                    .iter()
                    .position(|node| node.part_id == part_id)
            })
            .unwrap_or(0);
        self.select_index(index);
        self.refresh_detail();
    }

    /// Replace the rows with `parts`, saying nothing about the cursor.
    fn draw(&self, root: &str, parts: &[Attachment]) {
        let imp = self.imp();
        let nodes = tree(root, parts);
        imp.summary.set_text(&summary(&nodes));

        while let Some(row) = imp.tree.first_child() {
            imp.tree.remove(&row);
        }
        for node in &nodes {
            imp.tree.append(&tree_row(node));
        }
        *imp.nodes.borrow_mut() = nodes;
    }

    /// How much the reader held back on this message, for the canvas'
    /// `remote blocked` tag and the held-back card.
    pub fn set_held_back(&self, remote_images: u32, trackers: u32) {
        self.imp().held_back.set((remote_images, trackers));
        self.refresh_blocked();
        self.refresh_detail();
    }

    /// The tree, in walk order.
    pub fn nodes(&self) -> Vec<Node> {
        self.imp().nodes.borrow().clone()
    }

    /// The part the keyboard is on.
    pub fn cursor(&self) -> Option<Node> {
        let index = self.imp().tree.selected_row()?.index();
        let index = usize::try_from(index).ok()?;
        self.imp().nodes.borrow().get(index).cloned()
    }

    /// Walk down the tree.
    pub fn next_part(&self) {
        self.move_cursor(true);
    }

    /// Walk up the tree.
    pub fn prev_part(&self) {
        self.move_cursor(false);
    }

    /// Open what the cursor is on — the canvas' `Ret`.
    ///
    /// A part Postio can show, it shows; anything else it hands to the
    /// desktop rather than guessing at bytes it has no business interpreting.
    /// Either way it is [`connect_open`](Self::connect_open) that decides
    /// what that costs, because this surface cannot fetch.
    pub fn open_part(&self) {
        self.emit(&self.imp().on_open);
    }

    /// Save what the cursor is on — `s`.
    ///
    /// The panel owns the dialog and hands the handler the file the user
    /// chose. `GtkFileDialog` is the portal under Flatpak and a plain dialog
    /// outside it, which is the whole reason to use it rather than building a
    /// chooser: the sandboxed and unsandboxed paths are one call.
    ///
    /// The bytes are somebody else's problem, and that is deliberate — this
    /// surface must not be able to fetch. If the part has not been downloaded
    /// the handler is what goes and gets it.
    pub fn save_part(&self) {
        let Some(node) = self.cursor().filter(Node::is_leaf) else {
            return;
        };
        if self.asked(&Ask::Part(node.clone())) {
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(format!("Save {}", node.label()))
            .initial_name(save_name(&node))
            .modal(true)
            .build();
        let panel = self.clone();
        dialog.save(
            self.root_window().as_ref(),
            gio::Cancellable::NONE,
            move |chosen| {
                // A cancelled dialog is an answer, not an error: the user
                // decided not to. Anything else is worth a line.
                let file = match chosen {
                    Ok(file) => file,
                    Err(error) if error.matches(gtk::DialogError::Dismissed) => return,
                    Err(error) => {
                        glib::g_warning!("postio", "could not choose a place to save: {error}");
                        return;
                    }
                };
                for handler in panel.imp().on_save.borrow().iter() {
                    handler(&node, &file);
                }
            },
        );
    }

    /// Save every part that holds bytes — `S`.
    ///
    /// A folder rather than a filename, because there is more than one file:
    /// [`save_name`] gives each part the name it goes in under.
    pub fn save_all(&self) {
        if self.asked(&Ask::Everything) {
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title("Save every part")
            .modal(true)
            .build();
        let panel = self.clone();
        dialog.select_folder(
            self.root_window().as_ref(),
            gio::Cancellable::NONE,
            move |chosen| {
                let folder = match chosen {
                    Ok(folder) => folder,
                    Err(error) if error.matches(gtk::DialogError::Dismissed) => return,
                    Err(error) => {
                        glib::g_warning!("postio", "could not choose a folder: {error}");
                        return;
                    }
                };
                for handler in panel.imp().on_save_all.borrow().iter() {
                    handler(&folder);
                }
            },
        );
    }

    /// The window to hang a dialog off, if the panel is in one.
    fn root_window(&self) -> Option<gtk::Window> {
        self.root().and_downcast::<gtk::Window>()
    }

    /// Hand what the cursor is on to the desktop — `x`.
    pub fn open_externally(&self) {
        self.emit(&self.imp().on_external);
    }

    /// Render the held-back part once — `H`.
    pub fn render_once(&self) {
        self.emit(&self.imp().on_render_once);
    }

    /// Called when a part should be opened.
    pub fn connect_open(&self, handler: impl Fn(&Node) + 'static) {
        self.imp().on_open.borrow_mut().push(Box::new(handler));
    }

    /// Called with the part to save and the file the user chose for it.
    /// Answer "where should this go?" instead of opening a file dialog.
    ///
    /// **Installed, this replaces the dialog entirely** — [`save_part`] and
    /// [`save_all`] call the handler and construct nothing. That is the point
    /// (#988): a `GtkFileDialog` cannot be safely opened inside `gtk_suite`.
    /// The `GtkFileChooserNative` behind it keeps the parent window alive
    /// past the case that opened it, so closing the stray dialog frees a
    /// window its finalize later calls `gtk_window_destroy` on — a
    /// use-after-free that lands on whichever *later* case next turns the
    /// main loop, reading as flakiness in code that is fine. Cancelling with
    /// a `gio::Cancellable` instead only moves the crash earlier, into GTK's
    /// xdg-foreign export of the parent handle completing after the cancel.
    ///
    /// An override rather than the default path, deliberately: with nothing
    /// installed the panel opens the real dialog exactly as before, so no
    /// wiring has to be remembered in the application for `s` to work. A
    /// forgotten wire is the failure this codebase keeps finding
    /// (`postio-bl2`), and it is worse than the thing being worked around.
    ///
    /// [`save_part`]: Self::save_part
    /// [`save_all`]: Self::save_all
    pub fn connect_ask(&self, handler: impl Fn(&Ask) + 'static) {
        self.imp().on_ask.borrow_mut().push(Box::new(handler));
    }

    /// Whether an [`Ask`] handler answered, so no dialog should be built.
    fn asked(&self, ask: &Ask) -> bool {
        let handlers = self.imp().on_ask.borrow();
        if handlers.is_empty() {
            return false;
        }
        for handler in handlers.iter() {
            handler(ask);
        }
        true
    }

    pub fn connect_save(&self, handler: impl Fn(&Node, &gio::File) + 'static) {
        self.imp().on_save.borrow_mut().push(Box::new(handler));
    }

    /// Called with the folder the user chose to save every part into.
    pub fn connect_save_all(&self, handler: impl Fn(&gio::File) + 'static) {
        self.imp().on_save_all.borrow_mut().push(Box::new(handler));
    }

    /// What a drag of `node` offers a receiver, or `None` when there is
    /// nothing to offer.
    ///
    /// A container is refused: `multipart/mixed` is a wrapper, and exporting
    /// it would write an empty file named after something that was never a
    /// file. A part that has not been downloaded is *not* refused — the drop
    /// fetches it, exactly as `s` does, because the user named it by dragging.
    ///
    /// Public because it is what the drag actually does, and a test that drove
    /// anything else would be testing a copy of it.
    pub fn drag_offer(&self, node: &Node) -> Option<gdk::ContentProvider> {
        if !node.is_leaf() {
            return None;
        }
        let export = self.imp().export.borrow().clone()?;
        Some(crate::drag_out::LazyFiles::for_part(node.clone(), export).upcast())
    }

    /// The node whose row sits at `y` in the tree's coordinates.
    ///
    /// The row under the pointer rather than the cursor: a drag starts where
    /// the hand is, and the two coincide only if the user happened to have
    /// walked there with `j`.
    fn row_at(&self, y: f64) -> Option<Node> {
        let row = self.imp().tree.row_at_y(y as i32)?;
        let index = usize::try_from(row.index()).ok()?;
        self.imp().nodes.borrow().get(index).cloned()
    }

    /// The picture that follows the pointer while a part is dragged.
    ///
    /// The part's own name, so what is being carried is never in doubt — the
    /// same reason the message list's drag image says how many messages are
    /// moving rather than ghosting one row.
    fn drag_icon(&self, node: &Node) -> Option<gdk::Paintable> {
        let layout = self.create_pango_layout(Some(node.label()));
        let (width, height) = layout.pixel_size();
        let (pad_x, pad_y) = (10.0, 6.0);
        let (w, h) = (width as f32 + pad_x * 2.0, height as f32 + pad_y * 2.0);

        let snapshot = gtk::Snapshot::new();
        snapshot.append_color(
            &self.style_probe(&["postio-row-edge", "selected"]),
            &graphene::Rect::new(0.0, 0.0, w, h),
        );
        snapshot.save();
        snapshot.translate(&graphene::Point::new(pad_x, pad_y));
        snapshot.append_layout(
            &layout,
            &self.style_probe(&["postio-row-ground", "check-mark"]),
        );
        snapshot.restore();
        snapshot.to_paintable(Some(&graphene::Size::new(w, h)))
    }

    /// Read one role's colour off a throwaway node under this widget's
    /// classes, so a scheme change moves the drag image with everything else.
    fn style_probe(&self, classes: &[&str]) -> gdk::RGBA {
        let probe = gtk::Label::new(None);
        probe.set_css_classes(classes);
        probe.set_parent(self);
        let colour = probe.color();
        probe.unparent();
        colour
    }

    /// How a dragged part becomes a file, for a drop outside Postio.
    ///
    /// The same shape as [`connect_save`](Self::connect_save) and for the same
    /// reason: this panel must not be able to fetch. It knows which part the
    /// pointer is on; the bytes are the application's half.
    ///
    /// Dragging is never the only way out — `s` saves the part through the
    /// file dialog, which is the portal under Flatpak. A pointer gesture that
    /// was the sole path to an action would fail `/ux-architect`'s rule that
    /// the mouse is equal to the keyboard rather than ahead of it.
    pub fn connect_export(&self, materialise: crate::drag_out::MaterialisePart) {
        self.imp().export.replace(Some(materialise));
    }

    /// Called when a part should go to the desktop's own handler.
    pub fn connect_external(&self, handler: impl Fn(&Node) + 'static) {
        self.imp().on_external.borrow_mut().push(Box::new(handler));
    }

    /// Called when the user asks for a held-back part once.
    pub fn connect_render_once(&self, handler: impl Fn(&Node) + 'static) {
        self.imp()
            .on_render_once
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Put the keyboard in the tree.
    pub fn focus_tree(&self) {
        let imp = self.imp();
        match imp.tree.selected_row() {
            Some(row) => row.grab_focus(),
            None => imp.tree.grab_focus(),
        };
    }

    /// Regenerate the footer's key hints from the live keymap.
    ///
    /// A rebind changes what the footer says without a restart, the same
    /// promise [`crate::row::MessageRowView::set_keymap`] already keeps for the
    /// message list's own hints.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.imp().keys.set_text(&hints_for(keymap));
    }

    // -- internals ---------------------------------------------------------

    fn emit(&self, handlers: &RefCell<Vec<NodeHandler>>) {
        let Some(node) = self.cursor() else { return };
        for handler in handlers.borrow().iter() {
            handler(&node);
        }
    }

    /// One row up or down, clamping at the ends rather than wrapping.
    ///
    /// The clamp is [`postio_ui::reader::parts::step`]'s, for the reason
    /// [`first_part`] is shared: which row `j` lands on at the bottom of a
    /// tree is a decision, and two frontends deciding it separately is how
    /// one of them ends up wrapping.
    fn move_cursor(&self, forward: bool) {
        let count = self.imp().nodes.borrow().len();
        if count == 0 {
            return;
        }
        let current = self
            .imp()
            .tree
            .selected_row()
            .map(|row| row.index().max(0) as usize)
            .unwrap_or(0);
        self.select_index(step(current, forward, count));
    }

    fn select_index(&self, index: usize) {
        let imp = self.imp();
        let Some(row) = imp.tree.row_at_index(index as i32) else {
            return;
        };
        imp.tree.select_row(Some(&row));
        self.refresh_detail();
    }

    fn refresh_blocked(&self) {
        let (images, trackers) = self.imp().held_back.get();
        let held = images + trackers;
        self.imp().blocked.set_visible(held > 0);
    }

    /// Put the right-hand pane in step with the cursor.
    fn refresh_detail(&self) {
        let imp = self.imp();
        let Some(node) = self.cursor() else {
            imp.meta.set_text("");
            imp.note.set_text("");
            imp.render_once.set_visible(false);
            imp.save.set_sensitive(false);
            imp.external.set_sensitive(false);
            return;
        };

        imp.meta.set_text(&detail(&node));
        let (images, trackers) = imp.held_back.get();

        // Which sentence a state gets is `postio_ui::reader::parts::note`'s
        // call now, not this widget's: the macOS reader has the same four
        // states to explain, and #751's oversized inline image is a case
        // where the sentence *is* the explanation the user gets.
        imp.render_once
            .set_visible(held_back_note(&node.mime, images, trackers).is_some());
        imp.note.set_text(&note(&node, images, trackers));

        // A container has no bytes, so neither button has anything to act on.
        imp.save.set_sensitive(node.is_leaf());
        imp.external.set_sensitive(node.is_leaf());
        self.update_property(&[gtk::accessible::Property::Label(&format!(
            "Parts. {} selected. {}",
            node.label(),
            detail(&node)
        ))]);
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-parts");
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Center);
        self.set_visible(false);
        self.set_accessible_role(gtk::AccessibleRole::Group);

        let kicker = crate::widgets::kicker("Parts");
        kicker.set_accessible_role(gtk::AccessibleRole::Presentation);

        imp.summary.add_css_class("postio-parts-summary");
        imp.summary.set_xalign(0.0);
        imp.summary.set_hexpand(true);

        imp.blocked.set_text("remote blocked");
        imp.blocked.add_css_class("postio-chip-base");
        imp.blocked.add_css_class("postio-parts-blocked");
        imp.blocked.set_visible(false);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-parts-header");
        header.append(&kicker);
        header.append(&imp.summary);
        header.append(&imp.blocked);

        imp.tree.set_selection_mode(gtk::SelectionMode::Single);
        imp.tree.add_css_class("postio-parts-tree");
        imp.tree
            .update_property(&[gtk::accessible::Property::Label("Message parts")]);
        imp.tree.connect_row_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, _| panel.refresh_detail()
        ));
        imp.tree.connect_row_activated(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, _| panel.open_part()
        ));

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        // The tree asks for the height it needs rather than taking what is
        // left over. Without this the panel sized itself to the detail pane
        // and clipped the tree to three rows — a MIME tree that hides parts
        // is the one thing this panel exists not to do. Capped, because a
        // pathological message can nest further than a screen.
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(TREE_MAX_HEIGHT);
        scroller.set_child(Some(&imp.tree));

        imp.keys.set_text(&default_hints());
        imp.keys.add_css_class("postio-parts-keys");
        imp.keys.set_xalign(0.0);
        imp.keys.set_wrap(true);
        imp.keys
            .set_accessible_role(gtk::AccessibleRole::Presentation);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.add_css_class("postio-parts-column");
        left.set_size_request(TREE_WIDTH, -1);
        left.append(&scroller);
        left.append(&imp.keys);

        imp.meta.add_css_class("postio-parts-meta");
        imp.meta.set_xalign(0.0);

        imp.note.add_css_class("postio-parts-note");
        imp.note.set_xalign(0.0);
        imp.note.set_wrap(true);
        imp.note.set_max_width_chars(40);
        imp.note.set_vexpand(true);
        imp.note.set_valign(gtk::Align::Start);

        imp.render_once
            .set_child(Some(&crate::header::labelled("Render once", "H")));
        imp.render_once.add_css_class("postio-parts-action");
        imp.render_once.set_halign(gtk::Align::Start);
        imp.render_once.set_visible(false);
        imp.render_once
            .update_property(&[gtk::accessible::Property::Label(
                "Render this part once, loading what it references",
            )]);
        imp.render_once.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.render_once()
        ));

        imp.save
            .set_child(Some(&crate::header::labelled("Save part", "s")));
        imp.save.add_css_class("suggested-action");
        imp.save.add_css_class("postio-parts-action");
        imp.save
            .update_property(&[gtk::accessible::Property::Label("Save this part")]);
        imp.save.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.save_part()
        ));

        imp.external
            .set_child(Some(&crate::header::labelled("Open with…", "x")));
        imp.external.add_css_class("flat");
        imp.external.add_css_class("postio-parts-action");
        imp.external
            .update_property(&[gtk::accessible::Property::Label(
                "Open this part with the desktop's own handler",
            )]);
        imp.external.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.open_externally()
        ));

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.append(&imp.save);
        actions.append(&imp.external);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 12);
        right.add_css_class("postio-parts-detail");
        right.set_hexpand(true);
        right.append(&imp.meta);
        right.append(&imp.note);
        right.append(&imp.render_once);
        right.append(&actions);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&left);
        body.append(&right);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&body);
        self.set_child(Some(&column));

        // The panel's own keys are registry commands now, reached through
        // `Context::Parts` — `postio-14b`. `Window::act` dispatches them to
        // the methods below; this widget no longer owns a key controller.

        // Dragging a part out to the desktop. The row under the pointer, not
        // the cursor: a drag starts where the hand is, and the two are only
        // the same if the user happened to have walked there with `j`.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        drag.connect_prepare(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[upgrade_or]
            None,
            move |source, _, y| {
                let node = panel.row_at(y).filter(Node::is_leaf)?;
                if let Some(icon) = panel.drag_icon(&node) {
                    source.set_icon(Some(&icon), 12, 12);
                }
                panel.drag_offer(&node)
            }
        ));
        self.imp().tree.add_controller(drag);

        self.refresh_detail();
    }
}

/// One row of the tree: the box-drawing prefix, the part, and its size.
///
/// Mono throughout, because a MIME tree is structure rather than prose and
/// the box drawing only lines up in a fixed pitch.
fn tree_row(node: &Node) -> gtk::ListBoxRow {
    let name = gtk::Label::new(Some(&format!("{}{}", prefix(node), node.label())));
    name.add_css_class("postio-parts-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::Middle);
    name.set_accessible_role(gtk::AccessibleRole::Presentation);

    let size = gtk::Label::new(None);
    size.add_css_class("postio-parts-size");
    size.set_accessible_role(gtk::AccessibleRole::Presentation);
    if node.is_leaf() {
        size.set_text(&human_size(node.size));
    }

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    line.append(&name);
    line.append(&size);

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-parts-row");
    row.set_child(Some(&line));
    row.update_property(&[gtk::accessible::Property::Label(&spoken(node))]);
    row
}

// ---------------------------------------------------------------------------
// The chips under a message — canvas 1b
// ---------------------------------------------------------------------------

/// The attachments of an open message, as a row of chips.
///
/// Canvas 1b draws these under the body: what came with the message, named
/// and sized, before anything is downloaded. They are the way into
/// [`PartsPanel`] — a message's structure is a thing you go and look at, and
/// this is the affordance that says there is something to look at.
///
/// Only parts that hold bytes get a chip. A `multipart/alternative` is real
/// and appears in the tree, but nobody wants a chip for it.
#[derive(Clone)]
pub struct Chips {
    row: gtk::Box,
    handlers: Rc<RefCell<Vec<NodeHandler>>>,
}

impl Default for Chips {
    fn default() -> Self {
        Self::new()
    }
}

impl Chips {
    /// An empty, hidden row.
    pub fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("postio-attachments");
        row.set_visible(false);
        row.update_property(&[gtk::accessible::Property::Label("Attachments")]);
        Chips {
            row,
            handlers: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// The widget to place under a message body.
    pub fn widget(&self) -> gtk::Widget {
        self.row.clone().upcast()
    }

    /// Draw a chip for every part of `parts` that holds bytes.
    ///
    /// `root` is the message's own content type, so the nodes handed to
    /// [`Chips::connect_activated`] are the same nodes [`PartsPanel`] walks.
    pub fn set_parts(&self, root: &str, parts: &[Attachment]) {
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        let mut any = false;
        for node in tree(root, parts).into_iter().filter(Node::is_leaf) {
            // The body parts came with the message and are already on screen;
            // a chip for the text you are reading is noise.
            if node.mime.starts_with("text/") && node.filename.is_none() {
                continue;
            }
            self.row.append(&self.chip(node));
            any = true;
        }
        self.row.set_visible(any);
    }

    /// Called when a chip is activated, with the part it stands for.
    pub fn connect_activated(&self, handler: impl Fn(&Node) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }

    fn chip(&self, node: Node) -> gtk::Button {
        let name = gtk::Label::new(Some(node.label()));
        name.add_css_class("postio-attachment-name");
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        name.set_max_width_chars(24);
        name.set_accessible_role(gtk::AccessibleRole::Presentation);

        let size = gtk::Label::new(Some(&human_size(node.size)));
        size.add_css_class("postio-attachment-size");
        size.set_accessible_role(gtk::AccessibleRole::Presentation);

        let line = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        line.append(&name);
        line.append(&size);

        let button = gtk::Button::new();
        button.add_css_class("postio-attachment");
        button.set_child(Some(&line));
        button.update_property(&[gtk::accessible::Property::Label(&spoken(&node))]);
        // What it *is*, not what activating it will do: activating opens the
        // parts panel, which is where the verbs live. A chip that promised to
        // save would be a second place the same verb was implemented.
        button.set_tooltip_text(Some(&format!(
            "{} — show the message's parts",
            detail(&node)
        )));

        let handlers = Rc::clone(&self.handlers);
        button.connect_clicked(move |_| {
            for handler in handlers.borrow().iter() {
                handler(&node);
            }
        });
        button
    }
}

#[cfg(test)]
mod tests {
    //! What is left here is about the *widget*: the tree, the words and the
    //! names are `postio_ui::reader::parts`' now, and are tested beside them.

    use super::*;

    #[test]
    fn hints_read_the_live_keymap_not_a_hard_coded_string() {
        assert_eq!(
            default_hints(),
            "j/k walk · Return open · s save · S save all · x xdg-open · H render once",
            "the registry's own bindings, canvas order"
        );

        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("save_part".to_string(), "y".to_string());
        let rebound = hints_for(&Keymap::resolve(&overrides));
        assert_eq!(
            rebound, "j/k walk · Return open · y save · S save all · x xdg-open · H render once",
            "a rebind of `save_part` changes what the footer teaches, live"
        );
    }
}
