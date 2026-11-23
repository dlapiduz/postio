/// How the user gets back from a command that changed something: the frontend must tell "ask first"
/// from "offer an undo".
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UiRecovery {
    None,
    Undo,
    /// Reversible for a limited time through its own affordance, not the undo stack (#1481); a send
    /// is the case.
    Window,
    Confirm,
}

impl From<postio_core::registry::Recovery> for UiRecovery {
    fn from(recovery: postio_core::registry::Recovery) -> Self {
        use postio_core::registry::Recovery;
        match recovery {
            Recovery::None => UiRecovery::None,
            Recovery::Undo => UiRecovery::Undo,
            Recovery::Window => UiRecovery::Window,
            Recovery::Confirm => UiRecovery::Confirm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UiContext {
    List,
    Conversation,
    Reader,
    Composer,
    Search,
    Palette,
    Sidebar,
    Parts,
    Accounts,
    Keys,
    /// A picker anchored to a row; macOS has no surface for it yet, and resolves nothing there.
    Picker,
    Digest,
    Filtered,
    Capture,
    Results,
    Contacts,
    ContactPopover,
    Outline,
    ForwardChooser,
    Source,
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
            Context::Picker => UiContext::Picker,
            Context::Digest => UiContext::Digest,
            Context::Filtered => UiContext::Filtered,
            Context::Capture => UiContext::Capture,
            Context::Results => UiContext::Results,
            Context::Contacts => UiContext::Contacts,
            Context::ContactPopover => UiContext::ContactPopover,
            Context::Outline => UiContext::Outline,
            Context::ForwardChooser => UiContext::ForwardChooser,
            Context::Source => UiContext::Source,
        }
    }
}

impl From<UiContext> for postio_core::Context {
    /// The way back, written out rather than derived and checked by
    /// `every_context_survives_the_round_trip`: a mapping that sent two surfaces to one would
    /// resolve keys against the wrong context.
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
            UiContext::Picker => Context::Picker,
            UiContext::Digest => Context::Digest,
            UiContext::Filtered => Context::Filtered,
            UiContext::Capture => Context::Capture,
            UiContext::Results => Context::Results,
            UiContext::Contacts => Context::Contacts,
            UiContext::ContactPopover => Context::ContactPopover,
            UiContext::Outline => Context::Outline,
            UiContext::ForwardChooser => Context::ForwardChooser,
            UiContext::Source => Context::Source,
        }
    }
}

/// A top-level menu, in menu-bar order. The grouping is `postio_core::menu`'s (#657), so a command
/// added in Rust reaches the macOS menu with no Swift change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MenuSectionFfi {
    /// Present only where the platform has one; `menus()` leaves it out elsewhere and its commands
    /// arrive under `Edit`.
    App,
    File,
    Edit,
    View,
    Go,
    Message,
    Format,
    Help,
}

impl From<postio_core::menu::MenuSection> for MenuSectionFfi {
    fn from(section: postio_core::menu::MenuSection) -> Self {
        use postio_core::menu::MenuSection;
        match section {
            MenuSection::App => MenuSectionFfi::App,
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

/// Every menu, in menu-bar order: the order crosses as a list because an enum's declaration order
/// is not part of a uniffi contract.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuFfi {
    pub section: MenuSectionFfi,
    pub title: String,
}

#[uniffi::export]
pub fn menus() -> Vec<MenuFfi> {
    // The Mac's (`crate::PLATFORM`), whatever the host: macOS gets an application menu first and
    // the commands fold accordingly (`postio_core::menu::section_on`).
    postio_core::menu::MenuSection::bar_for(crate::PLATFORM)
        .into_iter()
        .map(|section| MenuFfi {
            section: MenuSectionFfi::from(section),
            title: section.title().to_string(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommandSpecFfi {
    /// The stable id, which `[keys]` and every log line name; commands cross as strings so a new
    /// one reaches the frontend with no change.
    pub id: String,
    pub title: String,
    /// The default binding in the keymap resolver's syntax, not necessarily what is in force:
    /// `[keys]` can override it.
    pub default_binding: String,
    /// Secondary bindings for the same command — the arrows beside `j`/`k`.
    pub alternate_bindings: Vec<String>,
    /// The surfaces it is reachable from, so the palette offers only what the focused surface can
    /// run.
    pub contexts: Vec<UiContext>,
    pub destructive: bool,
    /// How the user gets back. Never [`UiRecovery::None`] when `destructive`.
    pub recovery: UiRecovery,
    /// Which menu this belongs under, or `None` when deliberately not a menu item
    /// (`postio_core::menu::section_for`).
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
            menu: postio_core::menu::section_on(spec.id, crate::PLATFORM).map(MenuSectionFfi::from),
        }
    }
}

/// The commands the Swift frontend presents a surface for rather than dispatching. Held here and in
/// `PostioKit.Intercepted`, and checked from both sides: a list copied into Swift and checked from
/// neither is how `/` comes to open nothing.
pub const INTERCEPTED: &[postio_core::CommandId] = {
    use postio_core::CommandId as C;
    &[
        // Not `/`, `mod+k`, `g o`, the go-to keys or `?`: Focus's controller opens the command bar,
        // folders popover and key map, and the Mac draws them.
        C::Back,
        C::Settings,
        C::ScrollReaderDown,
        C::ScrollReaderUp,
        // Whether a message is drawn as its sender wrote it. Not `open_message`: Focus's controller
        // opens messages and the Mac has no handler for it.
        C::PrevView,
        C::ViewOriginal,
        C::ToggleReaderView,
        // The reader's zoom: `WKWebView.pageZoom`, and `[reader] zoom` in
        // the file GTK reads too.
        C::ZoomIn,
        C::ZoomOut,
        C::ZoomReset,
        // Find in the page: a bar this frontend draws, and WebKit's own find.
        C::FindInMessage,
        C::FindNext,
        C::FindPrevious,
        // "Show remote images" is render-once under the reader's own name:
        // this message, this view, no grant written (#1706).
        C::ShowImages,
        // AppKit's own Quit is the menu item; the registry's is the palette
        // row and whatever `[keys]` binds it to (#1706).
        C::Quit,
        // The blocked-images notice's "Always allow" and the unsubscribe banner's one deliberate
        // act (#1706).
        C::AlwaysShowImages,
        C::Unsubscribe,
        // `o`: the message window's attachments, as a menu this frontend
        // draws and hands the chosen one to the system.
        C::OpenAttachmentOrLink,
        // The unsubscribe banner's one offer (#1953): its button, from the
        // keyboard and the palette, and only while the banner offers it.
        C::ClearListMail,
        // The picker is a surface, and the four times it offers come from
        // the boundary so both frontends mean the same thing by them.
        C::ScheduleSend,
        // The settings window's account verbs act on the row its keyboard is on, a cursor no
        // session holds; adding an account, replacing a credential and opening `config.toml` need a
        // surface besides.
        C::AddAccount,
        C::EditConfig,
        C::ToggleAccountEnabled,
        C::RemoveAccount,
        C::UpdateCredential,
        C::RebuildAccountIndex,
        C::SetDefaultAccount,
        // The composer's own verbs. The draft being written is in a window
        // this frontend owns, unsaved, and the store has not seen most of
        // it -- so a session cannot answer these and the window does.
        C::Bold,
        C::Italic,
        C::Heading1,
        C::BulletList,
        C::NumberedList,
        C::QuoteBlock,
        C::InsertLink,
        C::CopyFields,
        C::Send,
        C::SaveDraft,
        C::DiscardDraft,
        C::AttachFile,
        // An open panel, and a picture that has to land at the caret of a
        // document only the window holds (#1571).
        C::InsertImage,
        // ⌘H in the composer: the draft's own reminder, which only its window holds until saved. On
        // the list and the open message `h` is the controller's picker.
        C::RemindIfNoReply,
        // The panels for a vCard: a file is read or written only through a
        // panel the user used, which this frontend presents (spec 011 S11).
        C::ContactsImport,
        C::ContactsExport,
        C::ContactsSync,
        C::ExpandAll,
        C::ToggleFold,
        C::NextInConversation,
        C::PrevInConversation,
        // Not `c`, `e`, `E` or `f`: Focus's controller opens the composer and the Mac draws it.
    ]
};

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
        // A command added in Rust reaches the macOS menu with no Swift change: the grouping
        // crosses, not just the commands (#657).
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
        // Both directions are hand-written matches: the compiler checks exhaustiveness but not that
        // they are inverses, and a mismatch reads as bindings that do not work.
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
        // Guards the `available_in` filter: an empty context set would empty every palette, an
        // all-contexts one would offer archive inside the composer.
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
