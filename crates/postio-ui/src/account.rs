//! What an account is, said the same way in both settings panes.
//!
//! The accounts pane is a list of rows and a form, and everything on the row
//! that is not a number out of the store is a *wording* decision: which
//! backend this is, how it signs in, whether it is the default. Those read
//! the same on both platforms or the two panes describe different software.

use postio_model::account::Account;

/// An account row's connection-type and auth-method badge — "IMAP ·
/// password", "Gmail · OAuth 2" — both already on the account itself, so
/// unlike the mail weight and the token validity this needs nothing handed
/// in from the composition root (#878).
pub fn badge(account: &Account) -> String {
    let backend = match &account.backend {
        postio_model::account::Backend::Imap => "IMAP",
        postio_model::account::Backend::Jmap { .. } => "JMAP",
        postio_model::account::Backend::Gmail => "Gmail",
    };
    let auth = match account.auth {
        postio_model::account::AuthMethod::Password => "password",
        postio_model::account::AuthMethod::AppPassword => "app password",
        postio_model::account::AuthMethod::OAuth2 => "OAuth 2",
        postio_model::account::AuthMethod::XOAuth2 => "OAuth 2",
    };
    format!("{backend} · {auth}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::EmailAddress;
    use postio_model::account::{Account, AuthMethod, Backend};

    fn account(backend: Backend, auth: AuthMethod) -> Account {
        let mut account = Account::new(
            "Ada Lovelace",
            EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        );
        account.backend = backend;
        account.auth = auth;
        account
    }

    #[test]
    fn a_badge_names_the_backend_and_how_it_signs_in() {
        assert_eq!(
            badge(&account(Backend::Imap, AuthMethod::Password)),
            "IMAP · password"
        );
        assert_eq!(
            badge(&account(Backend::Gmail, AuthMethod::OAuth2)),
            "Gmail · OAuth 2"
        );
        assert_eq!(
            badge(&account(Backend::Imap, AuthMethod::AppPassword)),
            "IMAP · app password"
        );
    }

    #[test]
    fn both_oauth_spellings_read_the_same_to_a_person() {
        // `XOAuth2` is a wire detail. A row that said "XOAUTH2" would be
        // telling the user about SASL rather than about their account.
        assert_eq!(
            badge(&account(Backend::Imap, AuthMethod::XOAuth2)),
            badge(&account(Backend::Imap, AuthMethod::OAuth2))
        );
    }
}
