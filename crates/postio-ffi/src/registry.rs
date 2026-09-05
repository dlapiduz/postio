//! The command vocabulary, as the frontend sees it.

/// How the user gets back from a command that changed something.
///
/// Crosses because the frontend has to honour the invariant, not merely be
/// told about it: `PRODUCT.md` requires that destructive operations are
/// confirmed or undoable, and a frontend that cannot tell "ask first" from
/// "offer an undo" cannot do either correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UiRecovery {
    /// Nothing to recover from; the command changed no durable state.
    None,
    /// Reversible from the undo stack, and worth an "— Undo" toast.
    Undo,
    /// Irreversible enough to ask first.
    Confirm,
}

impl From<postio_core::registry::Recovery> for UiRecovery {
    fn from(recovery: postio_core::registry::Recovery) -> Self {
        use postio_core::registry::Recovery;
        match recovery {
            Recovery::None => UiRecovery::None,
            Recovery::Undo => UiRecovery::Undo,
            Recovery::Confirm => UiRecovery::Confirm,
        }
    }
}

/// The surface a command is reachable from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UiContext {
    /// The message list: rows, selection, bulk actions.
    List,
    /// The conversation pane: the whole thread stacked beside the list.
    Conversation,
    /// The reading pane showing one message.
    Reader,
    /// Compose, in the pane or a window of its own.
    Composer,
    /// The search field and its results.
    Search,
    /// The command palette overlay.
    Palette,
    /// The folder list, once the keyboard is in it.
    Sidebar,
    /// The parts panel: a message's MIME tree.
    Parts,
    /// The account list in settings.
    Accounts,
    /// The keybinding list in settings.
    Keys,
}

impl From<postio_core::Context> for UiContext {
    fn from(context: postio_core::Context) -> Self {
        use postio_core::Context;
        match context {
            Context::List => UiContext::List,
            Context::Conversation => UiContext::Conversation,
            Context::Reader => UiContext::Reader,
            Context::Composer => UiContext::Composer,
            Context::Search => UiContext::Search,
            Context::Palette => UiContext::Palette,
            Context::Sidebar => UiContext::Sidebar,
            Context::Parts => UiContext::Parts,
            Context::Accounts => UiContext::Accounts,
            Context::Keys => UiContext::Keys,
        }
    }
}

impl From<UiContext> for postio_core::Context {
    /// The way back, for a frontend reporting where the keyboard is.
    ///
    /// Written out rather than derived, and the pair is checked by
    /// `every_context_survives_the_round_trip`: a mapping that silently sent
    /// two surfaces to one would make a key resolve against the wrong context,
    /// which looks like a binding that stopped working rather than like a
    /// conversion bug.
    fn from(context: UiContext) -> Self {
        use postio_core::Context;
        match context {
            UiContext::List => Context::List,
            UiContext::Conversation => Context::Conversation,
            UiContext::Reader => Context::Reader,
            UiContext::Composer => Context::Composer,
            UiContext::Search => Context::Search,
            UiContext::Palette => Context::Palette,
            UiContext::Sidebar => Context::Sidebar,
            UiContext::Parts => Context::Parts,
            UiContext::Accounts => Context::Accounts,
            UiContext::Keys => Context::Keys,
        }
    }
}

/// A top-level menu, in the order a menu bar shows them.
///
/// The grouping is `postio_core::menu`'s, not Swift's, and #657 is explicit
/// about why: a menu bar is a grouped rendering of a flat registry, and a
/// frontend that decided the grouping itself would disagree with the other
/// frontend the first time a command was added. Crossing it means adding a
/// command in Rust puts it in the macOS menu with no Swift change, which is
/// the acceptance criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MenuSectionFfi {
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
    /// The cheat sheet, where a Mac user looks for the keys.
    Help,
}

impl From<postio_core::menu::MenuSection> for MenuSectionFfi {
    fn from(section: postio_core::menu::MenuSection) -> Self {
        use postio_core::menu::MenuSection;
        match section {
            MenuSection::File => MenuSectionFfi::File,
            MenuSection::Edit => MenuSectionFfi::Edit,
            MenuSection::View => MenuSectionFfi::View,
            MenuSection::Go => MenuSectionFfi::Go,
            MenuSection::Message => MenuSectionFfi::Message,
            MenuSection::Format => MenuSectionFfi::Format,
            MenuSection::Help => MenuSectionFfi::Help,
        }
    }
}

/// Every menu, in menu-bar order, with the title each shows.
///
/// The order crosses as a list rather than being reconstructed from the enum,
/// because an enum's declaration order is not part of a uniffi contract and a
/// frontend sorting by it would be relying on something nothing promises.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuFfi {
    /// Which menu this is.
    pub section: MenuSectionFfi,
    /// The title a menu bar shows.
    pub title: String,
}

/// The menu bar's own shape: every section, in order.
#[uniffi::export]
pub fn menus() -> Vec<MenuFfi> {
    postio_core::menu::MenuSection::ALL
        .iter()
        .map(|section| MenuFfi {
            section: MenuSectionFfi::from(*section),
            title: section.title().to_string(),
        })
        .collect()
}

/// One row of the registry, on its way to a palette, a cheat sheet or a menu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommandSpecFfi {
    /// The stable id. `[keys]` in `config.toml` names this exact string, and
    /// so does every log line — which is why commands cross as strings rather
    /// than as a mirrored enum: a new command reaches the frontend without the
    /// boundary or the frontend changing.
    pub id: String,
    /// The title, as the palette and cheat sheet show it.
    pub title: String,
    /// The built-in binding, in the syntax the keymap resolver parses.
    ///
    /// The *default*, not necessarily what is in force: `[keys]` can override
    /// it, and a frontend drawing a menu accelerator from this alone would
    /// show the wrong key for a rebound command. Reading the binding actually
    /// in force needs the config to cross, which it does not yet.
    pub default_binding: String,
    /// Secondary bindings for the same command — the arrows beside `j`/`k`.
    pub alternate_bindings: Vec<String>,
    /// The surfaces this command is reachable from.
    ///
    /// Crosses so the palette can offer what the focused surface can actually
    /// run. Offering a command that will be ignored is worse than omitting it.
    pub contexts: Vec<UiContext>,
    /// Whether the command destroys something the user would have to rebuild.
    pub destructive: bool,
    /// How the user gets back. Never [`UiRecovery::None`] when `destructive`.
    pub recovery: UiRecovery,
    /// Which menu this belongs under, or `None` when it is deliberately not
    /// a menu item.
    ///
    /// `postio_core::menu::section_for`'s answer, which is exhaustive over
    /// the command vocabulary — so this is never "nobody has got round to it
    /// yet", always a decision somebody had to write down.
    pub menu: Option<MenuSectionFfi>,
}

impl From<&'static postio_core::registry::CommandSpec> for CommandSpecFfi {
    fn from(spec: &'static postio_core::registry::CommandSpec) -> Self {
        CommandSpecFfi {
            id: spec.id.as_str().to_string(),
            title: spec.title.to_string(),
            default_binding: spec.default_binding.to_string(),
            alternate_bindings: spec
                .alternate_bindings
                .iter()
                .map(|binding| (*binding).to_string())
                .collect(),
            contexts: postio_core::Context::ALL
                .iter()
                .filter(|context| spec.available_in(**context))
                .map(|context| UiContext::from(*context))
                .collect(),
            destructive: spec.destructive,
            recovery: spec.recovery.into(),
            menu: postio_core::menu::section_for(spec.id).map(MenuSectionFfi::from),
        }
    }
}

/// Every command, in cheat-sheet order.
pub fn commands() -> Vec<CommandSpecFfi> {
    postio_core::registry::all()
        .map(CommandSpecFfi::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_dropped_on_the_way_across() {
        assert_eq!(commands().len(), postio_core::registry::all().count());
    }

    #[test]
    fn the_menu_grouping_crosses_with_the_commands() {
        // #657's acceptance criterion, asserted where a Linux runner can see
        // it: a command added in Rust reaches the macOS menu with no Swift
        // change. What makes that true is that the *grouping* crosses, not
        // just the commands -- a frontend given a flat list would have to
        // decide the arrangement itself, which is the second copy the whole
        // boundary exists to avoid.
        let crossed = commands();
        assert!(
            crossed.iter().any(|spec| spec.menu.is_some()),
            "no command carried a menu section, so the menu would be empty"
        );
        let archive = crossed
            .iter()
            .find(|spec| spec.id == "archive")
            .expect("`archive` is in the registry");
        assert_eq!(
            archive.menu,
            Some(MenuSectionFfi::Message),
            "archive is a thing you do to a message"
        );
        // Every section a command names is one the frontend was told about,
        // or the menu bar has a command it cannot place.
        let offered: Vec<MenuSectionFfi> = menus().into_iter().map(|menu| menu.section).collect();
        for spec in &crossed {
            if let Some(section) = spec.menu {
                assert!(
                    offered.contains(&section),
                    "`{}` names {section:?}, which `menus()` does not offer",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn every_context_survives_the_round_trip() {
        // Both directions are hand-written `match`es over ten variants, and
        // the compiler checks that each is exhaustive but not that they are
        // inverses. A pair that sent `Reader` out and `List` back would give
        // the resolver the wrong context for every key pressed in the reading
        // pane -- which reads as bindings that do not work, not as a
        // conversion bug, and would be looked for in the keymap.
        for context in postio_core::Context::ALL {
            let crossed = UiContext::from(*context);
            assert_eq!(
                postio_core::Context::from(crossed),
                *context,
                "{context:?} did not come back as itself"
            );
        }
    }

    #[test]
    fn a_command_reachable_everywhere_lists_every_context() {
        // Guards the `available_in` filter above: a bug that returned an empty
        // context set would make every palette empty, and a bug that returned
        // all of them would offer archive inside the composer. Both are
        // invisible without a command whose real answer is known.
        let all = commands();
        assert!(
            all.iter().any(|spec| !spec.contexts.is_empty()),
            "no command reported any context, so the filter is inverted"
        );
        assert!(
            all.iter()
                .any(|spec| spec.contexts.len() < postio_core::Context::ALL.len()),
            "every command reported every context, so the filter is not filtering"
        );
    }
}
