//! What the window says when the keyring will not give up the key that opens
//! the mail.
//!
//! The store is encrypted under a key the keyring holds (ADR 0014 Q3), so a
//! keyring that will not answer means the mail cannot be opened at all. The
//! error type's own `Display` is written for a log; this is the sentence for
//! a person, in sentence case and in the app's words, so the page that stands
//! between someone and their mail never shows a pasted error.

/// Where a person unlocks the keyring on this platform.
#[cfg(not(target_os = "macos"))]
const UNLOCK: &str = "log in again, or open Passwords and Keys and unlock the login keyring";
#[cfg(target_os = "macos")]
const UNLOCK: &str = "log in again, or open Keychain Access and unlock the login keychain";

/// The keyring is locked: say what that costs and how to get past it.
pub fn locked() -> String {
    format!(
        "Your mail is encrypted, and your system keyring holds the key. The \
         keyring is locked: unlock it ({UNLOCK}), then try again."
    )
}

/// The keyring could not be read for some other reason, which `reason`
/// gives in the keyring's own words.
pub fn unreadable(reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    format!(
        "Your mail is encrypted, and Postio could not read the key to it \
         from your system keyring ({reason}). Check that the keyring is \
         running and unlocked, then try again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_locked_sentence_is_for_a_person() {
        let said = locked();
        assert!(said.starts_with(char::is_uppercase), "{said}");
        assert!(said.ends_with('.'), "{said}");
        for jargon in ["encryption key", "local store", "cannot read the password"] {
            assert!(!said.contains(jargon), "{jargon}: {said}");
        }
        assert!(
            said.contains("locked") && said.contains("unlock it"),
            "{said}"
        );
        // One spelling of the keyring, not "login keyring" and "Login keyring".
        assert!(!said.contains("Login"), "{said}");
    }

    #[test]
    fn an_unreadable_keyring_names_its_reason_without_a_stray_full_stop() {
        let said = unreadable("the Secret Service is not running.");
        assert!(
            said.contains("(the Secret Service is not running)"),
            "{said}"
        );
        assert!(said.starts_with(char::is_uppercase) && said.ends_with('.'));
    }
}
