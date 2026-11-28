//! The merge sheet: its words and keys are Rust's. The frontend draws `MergeSheetViewFfi`, reports
//! a click on a card, and presses Return, `n` and Escape as commands.

use postio_ui::contact_merge::MergeSheetView;

use crate::Session;
use crate::focus_contact_edit::EditButtonFfi;
use crate::focus_contacts::AvatarFfi;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MergeCardFfi {
    pub avatar: AvatarFfi,
    pub name: String,
    /// "saved" or "not saved".
    pub source: String,
    pub address: String,
    /// "412 messages, last today".
    pub stats: String,
    /// "Keep this name and photo" or "Address kept".
    pub keep: String,
    /// Whether it is the kept record: the accent ring.
    pub kept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MergeResultFfi {
    pub avatar: AvatarFfi,
    pub name: String,
    pub role: String,
    /// The addresses, the primary starred.
    pub addresses: String,
    /// "Result, 431 messages, ...".
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MergeSheetViewFfi {
    /// "These 3 look like the same person".
    pub title: String,
    pub subtitle: String,
    /// "2 sets left", none for records chosen by hand.
    pub position: Option<String>,
    pub cards: Vec<MergeCardFfi>,
    /// "Why we think so".
    pub why_title: String,
    pub evidence: Vec<String>,
    pub result: MergeResultFfi,
    /// "Undo with Cmd-Z. ..."
    pub foot: String,
    pub not_same: EditButtonFfi,
    pub later: EditButtonFfi,
    pub merge: EditButtonFfi,
}

impl From<MergeSheetView> for MergeSheetViewFfi {
    fn from(view: MergeSheetView) -> Self {
        let button = |button: postio_ui::contact_edit::EditButton| EditButtonFfi {
            label: button.label,
            key: button.key,
        };
        MergeSheetViewFfi {
            title: view.title,
            subtitle: view.subtitle,
            position: view.position,
            cards: view
                .cards
                .into_iter()
                .map(|card| MergeCardFfi {
                    avatar: card.avatar.into(),
                    name: card.name,
                    source: card.source,
                    address: card.address,
                    stats: card.stats,
                    keep: card.keep,
                    kept: card.kept,
                })
                .collect(),
            why_title: view.why_title,
            evidence: view.evidence,
            result: MergeResultFfi {
                avatar: view.result.avatar.into(),
                name: view.result.name,
                role: view.result.role,
                addresses: view.result.addresses,
                line: view.result.line,
            },
            foot: view.foot,
            not_same: button(view.not_same),
            later: button(view.later),
            merge: button(view.merge),
        }
    }
}

#[uniffi::export]
impl Session {
    /// A click on the merge sheet's card at `index`: its name and photo are kept.
    pub fn focus_contact_merge_keep(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactMergeKeep(index));
    }

    /// "Split" on the card's address: it becomes a person of its own.
    pub fn focus_contact_split(&self, address: String) {
        self.focus_driver()
            .input(postio_focus::Input::ContactSplit(address));
    }
}
