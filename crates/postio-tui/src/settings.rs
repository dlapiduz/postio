//! The settings: `config.toml`'s sections, and the accounts (US7).
//!
//! Canvas 3f's contract holds here as on the desktop: the file *is* the
//! settings. The sections are `postio_ui::settings::Section::ALL`, grouped and
//! described as the desktop's navigation has them; a section of the file is
//! shown as it stands, and edited in the person's own editor at that section.
//! The one section that is not text is the accounts, where the registry's
//! account commands act on the account under the cursor.

use postio_model::AccountId;
use postio_ui::settings::Section;

/// The settings screen, while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    section: usize,
    in_accounts: bool,
    account: usize,
    /// The account just removed, for `undo`.
    removed: Option<AccountId>,
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
        self.in_accounts = false;
    }

    /// Whether the keyboard is in the account list rather than the sections.
    pub fn in_accounts(&self) -> bool {
        self.in_accounts
    }

    /// Put the keyboard in the account list, or back in the sections.
    pub fn set_in_accounts(&mut self, inside: bool) {
        self.in_accounts = inside && self.current() == Section::Accounts;
    }

    /// The account the cursor is on, of `count`.
    pub fn account(&self, count: usize) -> usize {
        self.account.min(count.saturating_sub(1))
    }

    /// Move the account cursor by `step`, of `count`.
    pub fn step_account(&mut self, step: isize, count: usize) {
        self.account = self
            .account
            .saturating_add_signed(step)
            .min(count.saturating_sub(1));
    }

    /// Remember a removal, for `undo`.
    pub fn removed(&mut self, account: AccountId) {
        self.removed = Some(account);
    }

    /// The removal `undo` takes back, once.
    pub fn take_removed(&mut self) -> Option<AccountId> {
        self.removed.take()
    }
}
