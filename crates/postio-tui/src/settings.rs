//! The settings: `config.toml`'s sections, and the accounts (US7).
//!
//! Canvas 3f's contract holds here as on the desktop: the file *is* the
//! settings. The sections are `postio_ui::settings::Section::ALL`, grouped and
//! described as the desktop's navigation has them; a section of the file is
//! shown as it stands, and edited in the person's own editor at that section.
//! Two sections are not text: the accounts, where the registry's account
//! commands act on the account under the cursor, and privacy, which reads
//! back what left this machine and lets an allowed sender be asked again.

use postio_model::{AccountId, SignatureId};
use postio_ui::settings::Section;

/// The settings screen, while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    section: usize,
    in_list: bool,
    /// The row in the section's list: an account, or an allowed sender.
    row: usize,
    /// The account just removed, for `undo`.
    removed: Option<AccountId>,
    /// The account whose signatures are listed, while they are.
    signatures: Option<AccountId>,
    /// The signature the cursor is on.
    signature: usize,
    /// The signature a first delete asked about.
    deleting: Option<SignatureId>,
}

impl Settings {
    /// Every section, in the desktop's order.
    pub fn sections(&self) -> &'static [Section] {
        &Section::ALL
    }

    /// The section the cursor is on.
    pub fn current(&self) -> Section {
        Section::ALL[self.section]
    }

    /// Move the section cursor by `step`.
    pub fn step(&mut self, step: isize) {
        let last = Section::ALL.len() - 1;
        self.section = self.section.saturating_add_signed(step).min(last);
        self.in_list = false;
    }

    /// Whether the keyboard is in the section's list rather than the
    /// sections.
    pub fn in_list(&self) -> bool {
        self.in_list
    }

    /// Put the keyboard in the section's list, or back in the sections. Only
    /// the accounts and privacy have one.
    pub fn set_in_list(&mut self, inside: bool) {
        self.in_list = inside && matches!(self.current(), Section::Accounts | Section::Privacy);
    }

    /// The row the cursor is on, of `count`.
    pub fn row(&self, count: usize) -> usize {
        self.row.min(count.saturating_sub(1))
    }

    /// Move the row cursor by `step`, of `count`.
    pub fn step_row(&mut self, step: isize, count: usize) {
        self.row = self
            .row
            .saturating_add_signed(step)
            .min(count.saturating_sub(1));
    }

    /// Remember a removal, for `undo`.
    pub fn removed(&mut self, account: AccountId) {
        self.removed = Some(account);
    }

    /// The account whose signatures are listed, while they are.
    pub fn signatures_of(&self) -> Option<AccountId> {
        self.signatures
    }

    /// List `account`'s signatures, or go back to the accounts with `None`.
    pub fn show_signatures(&mut self, account: Option<AccountId>) {
        self.signatures = account;
        self.signature = 0;
        self.deleting = None;
    }

    /// The signature the cursor is on, of `count`.
    pub fn signature(&self, count: usize) -> usize {
        self.signature.min(count.saturating_sub(1))
    }

    /// Move the signature cursor by `step`, of `count`.
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

    /// Forget a delete asked about: anything else was the answer.
    pub fn keep(&mut self) {
        self.deleting = None;
    }

    /// The removal `undo` takes back, once.
    pub fn take_removed(&mut self) -> Option<AccountId> {
        self.removed.take()
    }
}
