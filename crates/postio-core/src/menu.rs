//! Where each command belongs in a platform menu bar.
//!
//! A menu bar is a *grouped* rendering of a flat registry, and the grouping
//! has to live somewhere. It lives here — beside the registry, in the crate
//! that owns the vocabulary — rather than as a `switch` in a frontend, for the
//! reason ADR 0019 keeps returning to: two frontends that each decide where
//! `archive` goes will disagree the first time a command is added, and the
//! disagreement is invisible until somebody looks at both machines.
//!
//! # Why this is a `match` and not a field
//!
//! [`section_for`] is exhaustive over [`CommandId`], so **adding a command
//! does not compile until somebody says where it goes.** That is the whole
//! mechanism behind #657's "adding a command in Rust puts it in the menu with
//! no Swift change": the decision is forced at the moment the command is
//! written, in the same file-adjacent place as the rest of its metadata,
//! rather than being noticed later as a gap in a menu nobody was looking at.
//!
//! A field on `CommandSpec` would do the same and would also make the
//! registry's eighty-one literals wider for something only one platform reads
//! today. This can become a field the moment a second thing needs it.
//!
//! # Why some commands have no section
//!
//! [`None`] is a decision, not an oversight, and every one of them is
//! commented. `PRODUCT.md` §8's rule is that *a command that is not in the
//! registry does not exist* — it is not that every command must be in a menu.
//! Cursor motion, chord completion and the composer's own text editing are
//! reachable, discoverable through the palette and the cheat sheet, and would
//! turn a menu into a list of everything.

use crate::command::CommandId;

/// A top-level menu, in the order a menu bar shows them.
///
/// Apple's conventions rather than Postio's preferences: a Mac user looks for
/// undo under Edit and for "reply" under a message-shaped menu, and an
/// application that invents its own arrangement reads as ported rather than
/// native. `Postio` and `Window` are AppKit's own and carry nothing from the
/// registry, so they are not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MenuSection {
    /// New mail, drafts, attachments, and getting things out of Postio.
    File,
    /// Undo, selection, and the editing verbs a text surface expects.
    Edit,
    /// What is on screen: panes, sidebars, ordering, folding.
    View,
    /// Moving: between messages, folders, panes and parts.
    Go,
    /// Acting on mail: reply, archive, flag, move, snooze, label.
    Message,
    /// The composer's rich-text verbs.
    Format,
    /// The cheat sheet and the rest of what a Help menu is for.
    Help,
}

impl MenuSection {
    /// Every section, in menu-bar order.
    pub const ALL: &'static [MenuSection] = &[
        MenuSection::File,
        MenuSection::Edit,
        MenuSection::View,
        MenuSection::Go,
        MenuSection::Message,
        MenuSection::Format,
        MenuSection::Help,
    ];

    /// The title a menu bar shows.
    pub fn title(self) -> &'static str {
        match self {
            MenuSection::File => "File",
            MenuSection::Edit => "Edit",
            MenuSection::View => "View",
            MenuSection::Go => "Go",
            MenuSection::Message => "Message",
            MenuSection::Format => "Format",
            MenuSection::Help => "Help",
        }
    }
}

/// Which menu `command` belongs under, or `None` when it is deliberately not
/// a menu item.
///
/// Exhaustive on purpose: see the module note. A new command is a compile
/// error here until it is placed.
pub fn section_for(command: CommandId) -> Option<MenuSection> {
    use CommandId as C;
    use MenuSection as M;
    match command {
        // ── Go: moving around ────────────────────────────────────────────
        // Cursor motion is `j`/`k` and the arrows. It is in the palette and
        // the cheat sheet; a menu item for "next message" is a row nobody
        // has ever clicked, and seven of them crowd out the ones people do.
        C::NextMessage | C::PrevMessage | C::ExtendSelectionDown | C::ExtendSelectionUp => None,
        C::FirstMessage | C::LastMessage => Some(M::Go),
        C::NextFolder | C::PrevFolder | C::FocusSidebar => Some(M::Go),
        C::CyclePane | C::CyclePaneBack => Some(M::Go),
        C::NextScope => Some(M::Go),
        C::NextInConversation | C::PrevInConversation => Some(M::Go),
        C::NextPart | C::PrevPart => Some(M::Go),
        C::Back | C::PrevView => Some(M::Go),

        // ── File ─────────────────────────────────────────────────────────
        C::Compose | C::Send | C::ScheduleSend | C::SaveDraft | C::DiscardDraft => Some(M::File),
        C::AttachFile | C::DetachComposer | C::MarkSent => Some(M::File),
        C::SavePart | C::SaveAllParts | C::OpenPartExternally => Some(M::File),
        C::Refresh => Some(M::File),

        // ── Edit ─────────────────────────────────────────────────────────
        C::Undo | C::SelectAll | C::ToggleSelection => Some(M::Edit),
        C::Search | C::SaveSearch => Some(M::Edit),
        C::Settings | C::EditConfig | C::AddAccount => Some(M::Edit),
        // Settings surfaces act on the row the settings list has focus on.
        // They are commands so `[keys]` can reach them and so the palette
        // can offer them where they apply; a menu bar item for "rename the
        // saved search you are looking at" is meaningless anywhere else, and
        // a menu is global.
        C::RenameSavedSearch
        | C::MoveSavedSearchUp
        | C::MoveSavedSearchDown
        | C::DeleteSavedSearch
        | C::ToggleAccountEnabled
        | C::RemoveAccount
        | C::UpdateCredential
        | C::RebuildAccountIndex => None,

        // ── View ─────────────────────────────────────────────────────────
        C::ToggleSidebar | C::ToggleFolder | C::ToggleFold | C::ExpandAll => Some(M::View),
        C::ToggleResultOrder => Some(M::View),
        C::OpenParts | C::ViewOriginal => Some(M::View),
        C::CommandPalette => Some(M::View),

        // ── Help ─────────────────────────────────────────────────────────
        // The cheat sheet, where a Mac user looks for it: every application
        // has a Help menu and "what are the keys" is the question it answers.
        // It is also the one menu item that is *about* the menu bar's own
        // limitation -- a sequence like `g g` has no key equivalent, so the
        // cheat sheet is where the keyboard is described in full.
        C::CheatSheet => Some(M::Help),
        // Scrolling the reader is what a scroll wheel and the space bar are
        // for. A menu item that scrolls by one screen is a control, not a
        // command, and putting it in a menu invites the reader to be driven
        // from one.
        C::ScrollReaderDown | C::ScrollReaderUp => None,
        // The one-off render of a part the reader would not draw by itself.
        // Deliberately *not* a menu item: `PRODUCT.md`'s privacy rule is that
        // this happens on a deliberate activation on the part itself, and a
        // menu item is a way to do it without having looked at what it
        // applies to.
        C::RenderPartOnce => None,

        // ── Message ──────────────────────────────────────────────────────
        C::Reply | C::ReplyAll | C::Forward => Some(M::Message),
        C::Archive | C::ArchiveThread | C::Delete | C::Move => Some(M::Message),
        C::Flag | C::MarkUnread | C::AddLabel => Some(M::Message),
        C::Snooze | C::Unsnooze => Some(M::Message),
        C::OpenMessage | C::OpenPart => Some(M::Message),

        // ── Format ───────────────────────────────────────────────────────
        C::Bold | C::Italic | C::BulletList | C::NumberedList => Some(M::Format),
        C::InsertLink | C::QuoteBlock => Some(M::Format),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_holds_something() {
        // A section with no commands draws an empty menu, which looks like a
        // bug in the application rather than a gap in a table.
        for section in MenuSection::ALL {
            assert!(
                crate::registry::all().any(|spec| section_for(spec.id) == Some(*section)),
                "{section:?} would draw an empty menu"
            );
        }
    }

    #[test]
    fn most_of_the_registry_reaches_a_menu() {
        // Not "all": the `None` arms above are deliberate and commented. What
        // this guards is the opposite failure -- a table that fell out of
        // date and started answering `None` broadly, leaving a menu bar that
        // is technically generated and practically empty.
        let total = crate::registry::all().count();
        let placed = crate::registry::all()
            .filter(|spec| section_for(spec.id).is_some())
            .count();
        assert!(
            placed * 2 > total,
            "only {placed} of {total} commands reach a menu"
        );
    }
}
