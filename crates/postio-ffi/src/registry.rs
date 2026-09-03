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
    /// A thread drilled into from the list.
    Thread,
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
            Context::Thread => UiContext::Thread,
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
