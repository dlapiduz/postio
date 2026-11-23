//! The settings: `config.toml`'s sections and the accounts. A section is shown as the file has it
//! and edited in the person's own editor; accounts and privacy are the two that are not text.

use postio_model::{AccountId, SignatureId};
use postio_ui::settings::Section;

/// The desktop's sections less Contacts, which has no commands in a terminal.
const SECTIONS: [Section; 9] = [
    Section::Accounts,
    Section::Filtering,
    Section::Filters,
    Section::Composing,
    Section::Appearance,
    Section::Keyboard,
    Section::Sync,
    Section::Privacy,
    Section::ConfigFile,
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    section: usize,
    in_list: bool,
    row: usize,
    removed: Option<AccountId>,
    signatures: Option<AccountId>,
    signature: usize,
    deleting: Option<SignatureId>,
}

impl Settings {
    pub fn sections(&self) -> &'static [Section] {
        &SECTIONS
    }

    pub fn current(&self) -> Section {
        SECTIONS[self.section]
    }

    pub fn step(&mut self, step: isize) {
        let last = SECTIONS.len() - 1;
        self.section = self.section.saturating_add_signed(step).min(last);
        self.in_list = false;
    }

    pub fn in_list(&self) -> bool {
        self.in_list
    }

    /// Only the accounts and privacy sections have a list.
    pub fn set_in_list(&mut self, inside: bool) {
        self.in_list = inside && matches!(self.current(), Section::Accounts | Section::Privacy);
    }

    pub fn row(&self, count: usize) -> usize {
        self.row.min(count.saturating_sub(1))
    }

    pub fn step_row(&mut self, step: isize, count: usize) {
        self.row = self
            .row
            .saturating_add_signed(step)
            .min(count.saturating_sub(1));
    }

    pub fn removed(&mut self, account: AccountId) {
        self.removed = Some(account);
    }

    pub fn signatures_of(&self) -> Option<AccountId> {
        self.signatures
    }

    pub fn show_signatures(&mut self, account: Option<AccountId>) {
        self.signatures = account;
        self.signature = 0;
        self.deleting = None;
    }

    pub fn signature(&self, count: usize) -> usize {
        self.signature.min(count.saturating_sub(1))
    }

    pub fn step_signature(&mut self, step: isize, count: usize) {
        self.signature = self
            .signature
            .saturating_add_signed(step)
            .min(count.saturating_sub(1));
    }

    /// Whether deleting `signature` was already asked about: the first ask
    /// only remembers it, the second is the answer.
    pub fn confirm_delete(&mut self, signature: SignatureId) -> bool {
        if self.deleting == Some(signature) {
            self.deleting = None;
            return true;
        }
        self.deleting = Some(signature);
        false
    }

    pub fn keep(&mut self) {
        self.deleting = None;
    }

    pub fn take_removed(&mut self) -> Option<AccountId> {
        self.removed.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_shows_every_section_that_is_shown_to_it() {
        let shown: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|section| section.shown_in(postio_core::Frontend::Terminal))
            .collect();
        assert_eq!(shown, SECTIONS);
    }
}
