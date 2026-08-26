//! PKCE (RFC 7636) and the `state` parameter (RFC 6749 §10.12).
//!
//! ADR 0006 Q3: **PKCE (S256) always**, including where a client secret
//! exists — it is what makes intercepting a loopback redirect useless. And
//! `state` is a fresh random value per attempt, checked by
//! [`super::redirect`] so a callback that does not match is dropped before
//! it ever reaches a token exchange.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Bytes of randomness behind a verifier or `state` value.
///
/// 32 bytes base64url-encodes to 43 characters, inside RFC 7636's required
/// 43–128 range for a code verifier with room to spare, and is the size
/// `state` generators converge on elsewhere for the same reason: enough
/// entropy that guessing one is not a threat model worth naming.
const RANDOM_BYTES: usize = 32;

/// A random base64url (no padding) string of [`RANDOM_BYTES`] bytes.
fn random_token() -> String {
    let mut bytes = [0u8; RANDOM_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The PKCE code verifier and its S256 challenge, generated together so one
/// can never be sent without the other.
#[derive(Clone, Debug)]
pub struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    /// Generates a fresh verifier and its derived challenge.
    pub fn generate() -> Self {
        let verifier = random_token();
        let challenge = challenge_for(&verifier);
        Self {
            verifier,
            challenge,
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
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

/// `BASE64URL-ENCODE(SHA256(ASCII(verifier)))`, per RFC 7636 §4.2.
fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// A fresh `state` value for one authorization attempt.
///
/// A distinct type from the verifier/challenge pair, even though both are
/// "a random token", because the two guard against different things: PKCE
/// binds the code to the client that requested it, `state` binds the
/// callback to the browser tab this attempt opened. Confusing the two in a
/// signature is exactly the kind of mistake a distinct type rules out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State(String);

impl State {
    /// A fresh random state.
    pub fn generate() -> Self {
        Self(random_token())
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
        // RFC 7636 Appendix B's worked example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        assert_eq!(challenge_for(verifier), expected);
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
