//! ADR 0006 Q3: PKCE (S256) always, and a fresh `state` per attempt checked by `redirect` before
//! any exchange. `Pkce` and `State` stay distinct types: PKCE binds the code to the client, `state`
//! binds the callback to the browser tab, and mixing them in a signature is the mistake to rule
//! out.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use io_oauth::rfc6749::state::Oauth20State;
use io_oauth::rfc7636::pkce::{Oauth20PkceCodeChallenge, Oauth20PkceCodeVerifier};

const STATE_CHARS: u8 = 32;

#[derive(Clone, Debug)]
pub struct Pkce {
    challenge: Oauth20PkceCodeChallenge,
    /// Cached as a string once: io-oauth keeps the bytes in a `SecretBox` exposed only by slice.
    verifier: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = Oauth20PkceCodeVerifier::new(64);
        let text = String::from_utf8_lossy(verifier.expose()).into_owned();
        Self {
            challenge: Oauth20PkceCodeChallenge {
                method: Default::default(), // S256 — io-oauth's own default
                verifier,
            },
            verifier: text,
        }
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    pub fn challenge(&self) -> String {
        self.challenge.encode().into_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State(String);

impl State {
    /// The transmitted value is base64url of io-oauth's entropy: RFC 6749 allows any VSCHAR, but a
    /// `&` or `#` must survive a redirect and query parse, and a raw value hung `redirect`'s tests.
    pub fn generate() -> Self {
        let state = Oauth20State::new(STATE_CHARS);
        Self(URL_SAFE_NO_PAD.encode(state.expose()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for State {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verifier_is_within_rfc_7636s_length_bounds() {
        let pkce = Pkce::generate();
        assert!(pkce.verifier().len() >= 43, "{}", pkce.verifier());
        assert!(pkce.verifier().len() <= 128, "{}", pkce.verifier());
    }

    #[test]
    fn a_verifier_uses_only_the_unreserved_characters_rfc_7636_allows() {
        let pkce = Pkce::generate();
        assert!(
            pkce.verifier()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')),
            "{}",
            pkce.verifier()
        );
    }

    #[test]
    fn the_challenge_is_the_base64url_sha256_of_the_verifier() {
        use std::str::FromStr;
        let challenge = Oauth20PkceCodeChallenge {
            method: Default::default(),
            verifier: Oauth20PkceCodeVerifier::from_str(
                "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            )
            .expect("the RFC's own example verifier"),
        };
        assert_eq!(
            challenge.encode(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn two_generated_pkce_pairs_never_collide() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_ne!(a.verifier(), b.verifier());
        assert_ne!(a.challenge(), b.challenge());
    }

    #[test]
    fn state_is_fresh_per_attempt() {
        let a = State::generate();
        let b = State::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn state_compares_to_a_callback_value() {
        let state = State::generate();
        let same = state.as_str().to_string();
        assert_eq!(state, *same);
        assert_ne!(state, *"something-else");
    }
}
