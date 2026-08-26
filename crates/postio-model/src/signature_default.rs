//! Which signature a new draft starts on, before anyone touches the picker.
//!
//! A signature has always been per-identity, and #12's picker lets a draft
//! point at any of the account's named signatures instead — but nothing
//! decided what the picker *starts on* besides the identity's own. #394 adds
//! two more opinions, an account-wide default and a per-mailbox override, and
//! this is where they combine into one answer.
//!
//! # The precedence
//!
//! Mailbox override, then account default, then the identity's own signature.
//! The identity's own needs no representation here — it is the composer
//! picker's resting state, so `None` from [`resolve`] already means exactly
//! that, and the caller does nothing further.
//!
//! A weaker order (account default only when the identity has none, say)
//! would make the account-wide setting redundant with what an identity
//! already does; the mailbox override existing at all only matters if it
//! can outrank an account-wide choice, since two accounts sharing one inbox
//! for two roles is the case #394 was filed for.

use crate::ids::SignatureId;

/// Combines a mailbox's own override with its account's default, in that
/// order. `None` means "no opinion here" — fall through to the identity's
/// own signature, which the composer already does when nothing else has
/// selected a signature in its picker.
pub fn resolve(
    mailbox_signature_id: Option<SignatureId>,
    account_default_signature_id: Option<SignatureId>,
) -> Option<SignatureId> {
    mailbox_signature_id.or(account_default_signature_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mailbox_override_wins_over_the_account_default() {
        let mailbox = Some(SignatureId::new(1));
        let account = Some(SignatureId::new(2));
        assert_eq!(resolve(mailbox, account), mailbox);
    }

    #[test]
    fn the_account_default_applies_when_the_mailbox_has_no_opinion() {
        let account = Some(SignatureId::new(2));
        assert_eq!(resolve(None, account), account);
    }

    #[test]
    fn neither_set_falls_through_to_the_identity_s_own() {
        assert_eq!(resolve(None, None), None);
    }
}
