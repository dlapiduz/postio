//! The sentence for a person when the keyring will not give up the key that opens the mail
//! (ADR 0014 Q3); the error's own `Display` is for a log.

#[cfg(not(target_os = "macos"))]
const UNLOCK: &str = "log in again, or open Passwords and Keys and unlock the login keyring";
#[cfg(target_os = "macos")]
const UNLOCK: &str = "log in again, or open Keychain Access and unlock the login keychain";

pub fn locked() -> String {
    format!(
        "Your mail is encrypted, and your system keyring holds the key. The \
         keyring is locked: unlock it ({UNLOCK}), then try again."
    )
}

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
