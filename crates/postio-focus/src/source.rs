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
    Unfold,
    Find,
    Import,
}

#[derive(Debug)]
pub(crate) struct Source {
    pub(crate) message: postio_model::MessageId,
    tab: Tab,
}

impl Source {
    pub(crate) fn new(message: postio_model::MessageId) -> Source {
        Source {
            message,
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
                | CommandId::AcceptKey
                | CommandId::ForgetKey
                | CommandId::ImportKey
                | CommandId::Back
        )
    }

    pub(crate) fn verb(&mut self, id: CommandId) -> Option<SourceVerb> {
        let tab = match id {
            CommandId::SourceTabVerification => Tab::Verification,
            CommandId::SourceTabHeaders => Tab::Headers,
            CommandId::SourceTabRaw => Tab::Raw,
            CommandId::CopySource => return Some(SourceVerb::Copy),
            CommandId::SaveSource => return Some(SourceVerb::Save),
            CommandId::ImportKey => return Some(SourceVerb::Import),
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
