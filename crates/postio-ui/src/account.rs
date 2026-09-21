//! What an account is, said the same way in both settings panes.
//!
//! The accounts pane is a list of rows and a form, and everything on the row
//! that is not a number out of the store is a *wording* decision: which
//! backend this is, how it signs in, whether it is the default, and whether
//! the credential behind it still works. Those read the same on both
//! platforms or the two panes describe different software.

use std::time::SystemTime;

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
        // Not "Maildir": the badge says what kind of account this is to
        // somebody who has one, and what is true of it is that the mail is
        // already here.
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

/// How an OAuth account's access token stands right now, from the expiry
/// `postio_account::oauth::token_source` persisted beside it (#870, #878).
///
/// A *state*, not a string, because two surfaces want different halves of
/// it: the fact line wants the wording, and the row wants to know whether
/// this is a problem — a warning mark beside the line, and the Reconnect
/// button that fixes it. GTK derived the second from the first by asking
/// whether the sentence it had just built began with "token expired", which
/// works and is the kind of thing that stops working the day somebody
/// rewords the sentence. Worse, on macOS there was nothing to ask at all:
/// `AccountFfi` carried only the badge, so the row's `needsAttention` gate
/// could never fire and the Reconnect button was unreachable code (#1584).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStanding {
    /// Nothing to say, and that is a real answer rather than a failure: a
    /// password account has no token, an account fed by an external broker
    /// never had Postio's own token source write an expiry for it, a
    /// provider that does not send `expires_in` leaves none behind, and a
    /// keyring that would not open answers the same way. None of those is
    /// something a person can act on, so the row stays quiet.
    Unknown,
    /// Good for a while yet.
    Valid {
        /// Whole days remaining — `0` meaning less than one, which is said
        /// in words rather than as a number nobody would trust.
        days: u64,
    },
    /// Past its stated expiry. The one state on an account row that is a
    /// problem rather than a fact.
    Expired,
}

impl TokenStanding {
    /// Where `expiry` leaves this account as of `now`.
    ///
    /// `now` is handed in rather than read here so the arithmetic can be
    /// asserted against a fixed clock; every caller passes
    /// [`SystemTime::now`].
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

    /// The line this standing puts on the row's `·`-joined fact line, or
    /// `None` when there is nothing to say.
    ///
    /// The expired wording names the repair rather than only the fault:
    /// "expired" alone is a dead end, and the row draws a Reconnect button
    /// beside it precisely because the sentence promises one.
    pub fn line(self) -> Option<String> {
        match self {
            Self::Unknown => None,
            Self::Valid { days: 0 } => Some("token valid less than a day".to_owned()),
            Self::Valid { days } => Some(format!("token valid {days}d")),
            Self::Expired => Some("token expired — re-authorization needed".to_owned()),
        }
    }

    /// Whether this is the state that calls for a warning mark and a repair.
    pub fn is_expired(self) -> bool {
        matches!(self, Self::Expired)
    }
}

/// What putting an account's credential right would actually take.
///
/// The two routes are not interchangeable and offering the wrong one is
/// worse than offering none: an expired OAuth grant is re-consented in a
/// browser and cannot be typed, and a rotated app password is typed and
/// cannot be re-consented. So the boundary says which, from the account's
/// own auth method, rather than each frontend reading `AuthMethod` and
/// drawing its own conclusion (#1584).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// There is nothing this pane can repair. A local mail store signs in to
    /// nothing, and an account whose token an external broker mints is one
    /// Postio holds no client for and no credential of — both would be rows
    /// offering to fix something that is not theirs to fix.
    Nothing,
    /// A new password for the keyring — an app-specific password the
    /// provider rotated, most of all.
    Password,
    /// Sign in again through the system browser. The grant is re-consented,
    /// which is a thing that happens to the person rather than a string they
    /// hold.
    Browser,
}

/// Which repair route `account` takes.
pub fn repair(account: &Account) -> Repair {
    if matches!(
        account.backend,
        postio_model::account::Backend::Maildir { .. }
    ) {
        return Repair::Nothing;
    }
    match account.auth {
        postio_model::account::AuthMethod::OAuth2 | postio_model::account::AuthMethod::XOAuth2 => {
            // A token account is one a typed password would not repair —
            // but only an account that signed in through Postio's own flow
            // carries the OAuth client a browser round trip would need. One
            // fed by a broker carries none, and offering to reconnect it
            // would open a browser with nothing to ask for.
            //
            // The client id is read rather than merely counted, and trimmed,
            // because this answer and the one `Session::browser_sign_in_for`
            // gives have to be the same answer. That one refuses a blank
            // client id — it has to: `sign_in_with_browser` beneath it
            // refuses one as a fact about the product, since Postio ships no
            // client id of its own (ADR 0006 Q1). A row that offered
            // Reconnect where the resolution refuses would draw a button
            // whose only outcome is a sentence about a credential the person
            // never registered. `oauth.is_some()` is not that question:
            // `AccountRepository` builds an `OAuthConfig` from two non-NULL
            // columns without asking whether either says anything.
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

    /// An account signed in through Postio's own OAuth flow, which is the
    /// only kind that carries the client a reconnect would sign in with.
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

    /// A fixed clock, so the day arithmetic below is not a race against the
    /// second boundary it is measured across.
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
        // The word on the row is for somebody looking at their own account,
        // not for somebody who knows the format's name.
        assert_eq!(
            badge(&account(
                Backend::Maildir {
                    root: "/home/ada/mail".to_owned()
                },
                AuthMethod::Password
            )),
            "Local mail · password"
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
        // "token valid 0d" reads as broken. It is not: it is a token that
        // will want renewing today, which is a different sentence.
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
        // A password account, a broker-fed account, a provider that never
        // said `expires_in`: all of them land here, and none of them is a
        // problem to report.
        let standing = TokenStanding::of(None, noon());
        assert_eq!(standing, TokenStanding::Unknown);
        assert_eq!(standing.line(), None);
        assert!(!standing.is_expired());
    }

    #[test]
    fn an_account_that_signs_in_with_a_token_is_repaired_in_the_browser() {
        // Both spellings, for the same reason the badge collapses them: the
        // difference is SASL's, and the repair is identical.
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
        // `oama`, `mutt_oauth2.py` and their kin hold the grant, do the
        // refreshing, and own the relationship with the provider. Postio has
        // no client id to sign in with and no token to replace, so a
        // Reconnect button here would open a browser to nowhere — the repair
        // is in the broker's own configuration, not in this pane.
        assert_eq!(
            repair(&account(Backend::Imap, AuthMethod::XOAuth2)),
            Repair::Nothing
        );
    }

    #[test]
    fn a_token_account_whose_client_id_is_blank_offers_no_repair_either() {
        // The row's offer and the boundary's resolution have to agree about
        // which accounts Reconnect appears on, and the resolution refuses a
        // blank client id — `Session::browser_sign_in_for` trims it, because
        // `sign_in_with_browser` further down refuses one too and says so as
        // a fact about the product ("Postio ships no client id"). An offer
        // the resolution would turn away is a button that is drawn, pressed,
        // and answered with a sentence about something the person cannot do
        // anything about.
        //
        // A row can hold one: the store builds `OAuthConfig` from two
        // non-NULL columns without asking whether either says anything, so
        // `oauth.is_some()` is not the same question as "there is a client
        // here".
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
        // `provision_local` writes no keyring entry at all, so a Reconnect
        // button here would be offering to replace a secret that was never
        // stored — and the auth method on the row is whatever `Account::new`
        // defaulted to, which is why the backend is asked first.
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
}
