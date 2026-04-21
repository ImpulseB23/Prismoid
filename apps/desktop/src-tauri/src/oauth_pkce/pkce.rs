//! PKCE (RFC 7636) verifier + S256 challenge, and CSRF `state`.
//!
//! Verifier: 43–128 chars from the unreserved set `[A-Z][a-z][0-9]-._~`.
//! We generate 32 random bytes and base64url-encode them, yielding a
//! 43-char verifier — comfortably within spec and indistinguishable
//! from server-side practice.
//!
//! Challenge: `BASE64URL(SHA256(verifier))`. S256 only — `plain` is
//! permitted by RFC 7636 §4.2 but is downgrade-attack-vulnerable and
//! disallowed by every OAuth provider we target.
//!
//! State: 32 random bytes, base64url-encoded. Carried through the
//! authorization request and verified on the redirect (RFC 6749
//! §10.12). Distinct from the verifier — the verifier is a secret
//! never sent until the token exchange; the state is sent in the clear
//! and is just an unguessable opaque token.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::TryRngCore;
use sha2::{Digest, Sha256};

/// Number of random bytes for both verifier and state. 32 → 256 bits
/// of entropy → 43 characters base64url-no-pad. Above the RFC 7636
/// minimum (32 chars) and well below the maximum (128 chars).
const RANDOM_BYTES: usize = 32;

/// PKCE verifier + S256 challenge pair. The verifier is what gets sent
/// at the token-exchange step; the challenge is what's published in
/// the authorization URL.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh verifier+challenge pair using the OS RNG.
    ///
    /// The OS RNG (`rand::rngs::OsRng`, a thin wrapper over `getrandom`)
    /// is the only acceptable source: PKCE security relies on the
    /// verifier being unguessable. Userspace PRNGs would not survive
    /// a timing analysis of the SHA-256 challenge round-tripping the
    /// network during the authorization phase.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; RANDOM_BYTES];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .expect("OS RNG unavailable");
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = challenge_for(&verifier);
        Self {
            verifier,
            challenge,
        }
    }
}

/// Opaque CSRF token sent as `state` in the authorization request and
/// re-checked on the redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State(pub String);

impl State {
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; RANDOM_BYTES];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .expect("OS RNG unavailable");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_is_43_chars_base64url_no_pad() {
        let p = Pkce::generate();
        assert_eq!(p.verifier.len(), 43);
        // base64url-no-pad alphabet: A-Z a-z 0-9 - _
        assert!(
            p.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier had non-base64url chars: {}",
            p.verifier
        );
    }

    #[test]
    fn challenge_matches_s256_of_verifier() {
        let p = Pkce::generate();
        assert_eq!(p.challenge, challenge_for(&p.verifier));
    }

    #[test]
    fn challenge_known_test_vector() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(challenge_for(verifier), expected);
    }

    #[test]
    fn two_generates_produce_distinct_pairs() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.challenge, b.challenge);
    }

    #[test]
    fn state_is_43_chars_base64url_no_pad() {
        let s = State::generate();
        assert_eq!(s.0.len(), 43);
        assert!(s
            .0
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn two_states_are_distinct() {
        assert_ne!(State::generate(), State::generate());
    }
}
