//! The Source surface's behaviour: which tab is showing, and which of its verbs a tab has.

use postio_core::CommandId;
use postio_ui::source::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    List,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceVerb {
    Tab(Tab),
    Copy,
    Save,
    /// The fold under the frontend's cursor; only on Raw.
    Unfold,
    /// Only where there is text to find: Headers and Raw.
    Find,
}

#[derive(Debug)]
pub(crate) struct Source {
    tab: Tab,
}

impl Source {
    pub(crate) fn new() -> Source {
        Source {
            tab: Tab::Verification,
        }
    }

    pub(crate) fn owns(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::SourceTabVerification
                | CommandId::SourceTabHeaders
                | CommandId::SourceTabRaw
                | CommandId::CopySource
                | CommandId::SaveSource
                | CommandId::UnfoldPart
                | CommandId::FindInMessage
                | CommandId::Back
        )
    }

    /// `None` for a key this tab does nothing with: it reaches nothing behind the surface either.
    pub(crate) fn verb(&mut self, id: CommandId) -> Option<SourceVerb> {
        let tab = match id {
            CommandId::SourceTabVerification => Tab::Verification,
            CommandId::SourceTabHeaders => Tab::Headers,
            CommandId::SourceTabRaw => Tab::Raw,
            CommandId::CopySource => return Some(SourceVerb::Copy),
            CommandId::SaveSource => return Some(SourceVerb::Save),
            CommandId::UnfoldPart => return (self.tab == Tab::Raw).then_some(SourceVerb::Unfold),
            CommandId::FindInMessage => {
                return (self.tab != Tab::Verification).then_some(SourceVerb::Find);
            }
            _ => return None,
        };
        self.tab = tab;
        Some(SourceVerb::Tab(tab))
    }
}
