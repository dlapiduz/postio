//! Which signature a new draft starts on: mailbox override, then account default, then the
//! identity's own (#394). The identity's own
//! is the picker's resting state, so `None` from `resolve` means exactly that. A mailbox override
//! must outrank an account-wide choice.

use crate::ids::SignatureId;

/// `None` means no opinion here: fall through to the identity's own signature.
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
