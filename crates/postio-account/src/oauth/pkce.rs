//! PKCE (RFC 7636) and the `state` parameter (RFC 6749 §10.12), on
//! io-oauth's own types (#537).
//!
//! ADR 0006 Q3: **PKCE (S256) always**, including where a client secret
//! exists — it is what makes intercepting a loopback redirect useless. And
//! `state` is a fresh random value per attempt, checked by
//! [`super::redirect`] so a callback that does not match is dropped before
//! it ever reaches a token exchange.
//!
//! The generation and encoding used to be hand-rolled here; they are now
//! `io-oauth`'s `rfc7636::pkce` and `rfc6749::state` — the same maintained
//! wire core [`super::exchange`] uses for the token grants. These wrappers
//! keep the two values as distinct Postio types, because the pair guard
//! against different things: PKCE binds the code to the client that
//! requested it, `state` binds the callback to the browser tab this attempt
//! opened, and confusing the two in a signature is exactly the mistake a
//! distinct type rules out.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use io_oauth::rfc6749::state::Oauth20State;
use io_oauth::rfc7636::pkce::{Oauth20PkceCodeChallenge, Oauth20PkceCodeVerifier};

/// How many random VSCHAR characters feed a `state` value.
///
/// Base64url-encoded below, 32 stays comfortably past the entropy where
/// guessing one is a threat model worth naming.
const STATE_CHARS: u8 = 32;

/// The PKCE code verifier and its S256 challenge, generated together so one
/// can never be sent without the other.
#[derive(Clone, Debug)]
pub struct Pkce {
    challenge: Oauth20PkceCodeChallenge,
    /// The verifier's characters, cached as a string once: the token
    /// exchange sends it as form text, and `io-oauth` keeps the bytes in a
    /// `SecretBox` it only exposes by slice.
    verifier: String,
}

impl Pkce {
    /// Generates a fresh verifier and its derived challenge.
    pub fn generate() -> Self {
        // 64 unreserved characters: inside RFC 7636's 43–128 with room to
        // spare, same as the hand-rolled version's entropy.
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

    /// The secret sent only to the token endpoint, over the connection the
    /// authorization code itself arrives on.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// `BASE64URL-ENCODE(SHA256(verifier))`, sent in the authorization
    /// request. Never a secret — it is derived, one-way, and public by
    /// design.
    pub fn challenge(&self) -> String {
        self.challenge.encode().into_owned()
    }
}

/// A fresh `state` value for one authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State(String);

impl State {
    /// A fresh random state.
    ///
    /// io-oauth's [`Oauth20State`] is the entropy source; the transmitted
    /// value is its base64url encoding rather than its raw characters.
    /// RFC 6749 allows any VSCHAR in `state` — spaces, `&`, `#` included —
    /// and every one of those has to survive a browser redirect and a
    /// query-string parse to be compared against. An unreserved-only value
    /// removes that whole class of round-trip disagreement, which is not
    /// hypothetical: the raw alphabet hung [`super::redirect`]'s own tests
    /// on the first value that drew a `&`.
    pub fn generate() -> Self {
        let state = Oauth20State::new(STATE_CHARS);
        Self(URL_SAFE_NO_PAD.encode(state.expose()))
    }

    /// The value to send in the authorization request and compare the
    /// callback against.
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
        // RFC 7636 Appendix B's worked example, held against io-oauth's
        // encoding so a swapped dependency still computes the same bytes.
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
