//! Wording for an account row, shared by both settings panes.

use std::time::SystemTime;

use postio_model::account::Account;

/// An account row's connection-type and auth-method badge, e.g. "IMAP · password".
pub fn badge(account: &Account) -> String {
    if matches!(
        account.backend,
        postio_model::account::Backend::Maildir { .. }
    ) {
        return "Local mail".to_owned();
    }
    let backend = match &account.backend {
        postio_model::account::Backend::Imap => "IMAP",
        postio_model::account::Backend::Jmap { .. } => "JMAP",
        postio_model::account::Backend::Gmail => "Gmail",
        postio_model::account::Backend::Maildir { .. } => "Local mail",
    };
    let auth = match account.auth {
        postio_model::account::AuthMethod::Password => "password",
        postio_model::account::AuthMethod::AppPassword => "app password",
        postio_model::account::AuthMethod::OAuth2 => "OAuth 2",
        postio_model::account::AuthMethod::XOAuth2 => "OAuth 2",
    };
    format!("{backend} · {auth}")
}

/// Token standing from the persisted expiry. A state, not a string: the row needs the
/// problem flag (warning mark, Reconnect) as well as the wording, and deriving one from
/// the other by string match breaks on rewording (#1584).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStanding {
    /// Nothing to say, and not a failure: password account, broker-fed token, no
    /// `expires_in`, or an unopenable keyring. Nothing a person can act on.
    Unknown,
    Valid {
        /// Whole days remaining; `0` means less than one.
        days: u64,
    },
    Expired,
}

impl TokenStanding {
    /// `now` is passed in so tests can fix the clock.
    pub fn of(expiry: Option<SystemTime>, now: SystemTime) -> Self {
        let Some(at) = expiry else {
            return Self::Unknown;
        };
        match at.duration_since(now) {
            Ok(remaining) => Self::Valid {
                days: remaining.as_secs() / (24 * 60 * 60),
            },
            Err(_) => Self::Expired,
        }
    }

    /// The fact-line wording, or `None`. Expired names the repair, since the row draws a
    /// Reconnect button beside it.
    pub fn line(self) -> Option<String> {
        match self {
            Self::Unknown => None,
            Self::Valid { days: 0 } => Some("token valid less than a day".to_owned()),
            Self::Valid { days } => Some(format!("token valid {days}d")),
            Self::Expired => Some("token expired — re-authorization needed".to_owned()),
        }
    }

    pub fn is_expired(self) -> bool {
        matches!(self, Self::Expired)
    }
}

/// The repair route for an account's credential. Not interchangeable: an OAuth grant is
/// re-consented in a browser, a password is typed (#1584).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// Nothing this pane can repair: a local store signs in to nothing, and a broker-minted
    /// token is not Postio's to fix.
    Nothing,
    Password,
    Browser,
}

pub fn repair(account: &Account) -> Repair {
    if matches!(
        account.backend,
        postio_model::account::Backend::Maildir { .. }
    ) {
        return Repair::Nothing;
    }
    match account.auth {
        postio_model::account::AuthMethod::OAuth2 | postio_model::account::AuthMethod::XOAuth2 => {
            // Only an account signed in through Postio's own flow has a client for a browser
            // round trip. Must agree with `Session::browser_sign_in_for`, which refuses a
            // blank client id (ADR 0006 Q1); `oauth.is_some()` does not say that.
            let client = account
                .oauth
                .as_ref()
                .map(|oauth| oauth.client_id.trim())
                .unwrap_or_default();
            if client.is_empty() {
                Repair::Nothing
            } else {
                Repair::Browser
            }
        }
        postio_model::account::AuthMethod::Password
        | postio_model::account::AuthMethod::AppPassword => Repair::Password,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    Expired,
    Rejected,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub reason: String,
    /// `None` when the pane can put nothing right; the row still says what is wrong.
    pub button: Option<&'static str>,
}

/// `None` when the credential is fine. `expired` is read fresh on every settings open, so
/// it outranks `found`, the last connection test. The button is named for its route (#1584).
pub fn offer(
    repair: Repair,
    expired: bool,
    found: Option<Trouble>,
    running: bool,
) -> Option<Offer> {
    let trouble = if expired { Trouble::Expired } else { found? };
    let (held, button) = match repair {
        Repair::Nothing => ("credential", None),
        Repair::Password => (
            "password",
            Some(if running {
                "Saving…"
            } else {
                "New password…"
            }),
        ),
        Repair::Browser => (
            "sign-in",
            Some(if running {
                "Signing in…"
            } else {
                "Reconnect"
            }),
        ),
    };
    let reason = match trouble {
        Trouble::Expired => format!("This account's {held} has expired."),
        Trouble::Rejected => format!("The server refused this account's {held}."),
        Trouble::Missing => format!("This account has no {held} in your keyring yet."),
    };
    Some(Offer { reason, button })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use postio_model::EmailAddress;
    use postio_model::account::{Account, AuthMethod, Backend, OAuthConfig};

    fn account(backend: Backend, auth: AuthMethod) -> Account {
        let mut account = Account::new(
            "Ada Lovelace",
            EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        );
        account.backend = backend;
        account.auth = auth;
        account
    }

    fn with_own_client(mut account: Account) -> Account {
        account.oauth = Some(OAuthConfig {
            client_id: "a-client-the-user-registered".to_owned(),
            token_url: "https://oauth.example.com/token".to_owned(),
            authorize_url: "https://oauth.example.com/authorize".to_owned(),
            scopes: "https://oauth.example.com/mail".to_owned(),
            refresh_token_lifetime_days: None,
        });
        account
    }

    fn noon() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
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
    fn a_local_maildir_reads_as_mail_that_is_already_here() {
        assert_eq!(
            badge(&account(
                Backend::Maildir {
                    root: "/home/ada/mail".to_owned()
                },
                AuthMethod::Password
            )),
            "Local mail"
        );
    }

    #[test]
    fn both_oauth_spellings_read_the_same_to_a_person() {
        assert_eq!(
            badge(&account(Backend::Imap, AuthMethod::XOAuth2)),
            badge(&account(Backend::Imap, AuthMethod::OAuth2))
        );
    }

    #[test]
    fn a_token_whose_moment_has_passed_is_expired_and_says_what_to_do() {
        let standing = TokenStanding::of(Some(noon() - Duration::from_secs(60)), noon());
        assert_eq!(standing, TokenStanding::Expired);
        assert!(standing.is_expired());
        assert_eq!(
            standing.line().as_deref(),
            Some("token expired — re-authorization needed"),
            "the line has to name the repair: 'expired' alone is a dead end"
        );
    }

    #[test]
    fn a_token_with_days_left_counts_the_whole_ones() {
        let standing = TokenStanding::of(
            Some(noon() + Duration::from_secs(41 * 24 * 60 * 60 + 3600)),
            noon(),
        );
        assert_eq!(standing, TokenStanding::Valid { days: 41 });
        assert_eq!(standing.line().as_deref(), Some("token valid 41d"));
        assert!(!standing.is_expired());
    }

    #[test]
    fn a_token_good_for_hours_says_so_in_words_rather_than_zero_days() {
        let standing = TokenStanding::of(Some(noon() + Duration::from_secs(3 * 3600)), noon());
        assert_eq!(standing, TokenStanding::Valid { days: 0 });
        assert_eq!(
            standing.line().as_deref(),
            Some("token valid less than a day")
        );
        assert!(!standing.is_expired());
    }

    #[test]
    fn nothing_on_file_puts_nothing_on_the_row() {
        let standing = TokenStanding::of(None, noon());
        assert_eq!(standing, TokenStanding::Unknown);
        assert_eq!(standing.line(), None);
        assert!(!standing.is_expired());
    }

    #[test]
    fn an_account_that_signs_in_with_a_token_is_repaired_in_the_browser() {
        assert_eq!(
            repair(&with_own_client(account(
                Backend::Imap,
                AuthMethod::XOAuth2
            ))),
            Repair::Browser
        );
        assert_eq!(
            repair(&with_own_client(account(
                Backend::Gmail,
                AuthMethod::OAuth2
            ))),
            Repair::Browser
        );
    }

    #[test]
    fn a_token_account_whose_credential_a_broker_owns_offers_no_repair_here() {
        // Broker-held grant: no client id to sign in with, so Reconnect would open a browser to
        // nowhere.
        assert_eq!(
            repair(&account(Backend::Imap, AuthMethod::XOAuth2)),
            Repair::Nothing
        );
    }

    #[test]
    fn a_token_account_whose_client_id_is_blank_offers_no_repair_either() {
        // The offer must agree with `browser_sign_in_for`, which trims and refuses a blank
        // client id; `oauth.is_some()` is not enough.
        let mut blank = with_own_client(account(Backend::Imap, AuthMethod::XOAuth2));
        blank.oauth.as_mut().expect("an oauth row").client_id = "   ".to_owned();
        assert_eq!(repair(&blank), Repair::Nothing);
    }

    #[test]
    fn an_account_that_signs_in_with_a_password_is_repaired_by_typing_one() {
        assert_eq!(
            repair(&account(Backend::Imap, AuthMethod::Password)),
            Repair::Password
        );
        assert_eq!(
            repair(&account(Backend::Imap, AuthMethod::AppPassword)),
            Repair::Password
        );
    }

    #[test]
    fn a_local_mail_store_has_no_credential_to_repair() {
        assert_eq!(
            repair(&account(
                Backend::Maildir {
                    root: "/home/ada/mail".to_owned()
                },
                AuthMethod::Password
            )),
            Repair::Nothing
        );
    }

    #[test]
    fn every_broken_credential_gets_a_reason_and_a_button_by_its_route() {
        let refused = offer(Repair::Password, false, Some(Trouble::Rejected), false)
            .expect("a refused password is something to repair");
        assert_eq!(
            refused.reason,
            "The server refused this account's password."
        );
        assert_eq!(refused.button, Some("New password…"));

        let missing = offer(Repair::Browser, false, Some(Trouble::Missing), false)
            .expect("a missing credential is something to repair");
        assert_eq!(
            missing.reason,
            "This account has no sign-in in your keyring yet."
        );
        assert_eq!(missing.button, Some("Reconnect"));

        let expired = offer(Repair::Browser, true, None, false).expect("an expiry is trouble");
        assert_eq!(expired.reason, "This account's sign-in has expired.");
    }

    #[test]
    fn an_expiry_outranks_what_a_test_found_and_nothing_found_offers_nothing() {
        assert_eq!(
            offer(Repair::Browser, true, Some(Trouble::Rejected), false).map(|offer| offer.reason),
            Some("This account's sign-in has expired.".to_owned())
        );
        assert_eq!(offer(Repair::Password, false, None, false), None);
    }

    #[test]
    fn a_credential_nothing_here_can_mend_is_said_but_offers_no_button() {
        let broker = offer(Repair::Nothing, false, Some(Trouble::Rejected), false)
            .expect("the trouble is still said");
        assert_eq!(
            broker.reason,
            "The server refused this account's credential."
        );
        assert_eq!(broker.button, None);
    }

    #[test]
    fn the_button_says_what_it_is_doing_while_it_runs() {
        assert_eq!(
            offer(Repair::Password, false, Some(Trouble::Rejected), true)
                .and_then(|offer| offer.button),
            Some("Saving…")
        );
        assert_eq!(
            offer(Repair::Browser, true, None, true).and_then(|offer| offer.button),
            Some("Signing in…")
        );
    }
}
