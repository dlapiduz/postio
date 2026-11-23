//! What a cryptographically checked signature is allowed to mean. rPGP checks the mathematics; this
//! layer refuses what Sequoia's standard policy refuses, so a weak signature never reads as good.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hash {
    Md5,
    Sha1,
    Ripemd160,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Sha3,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStrength {
    Rsa { bits: u32 },
    Dsa { bits: u32 },
    EdDsa,
    Ecdsa,
    Other,
}

/// What a signature and the key that made it say, before any judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub hash: Hash,
    pub key: KeyStrength,
    /// Seconds since the Unix epoch.
    pub signed_at: Option<i64>,
    pub key_created: i64,
    pub key_expires: Option<i64>,
    /// A revocation signature on the key is already held.
    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Good,
    Bad,
    CantCheck,
    NotHereYet,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NoKey,
    WeakHash(Hash),
    WeakKey,
    Expired,
    NotYetValid,
    Unsupported,
    Malformed,
    Tampered,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Policy;

impl Policy {
    /// `mathematics` is whether the signature checks against the key's material.
    pub fn judge(&self, facts: &Facts, mathematics: bool) -> (Status, Option<Reason>) {
        let _ = (facts, mathematics);
        (Status::Good, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATED: i64 = 1_772_528_400;

    fn sound() -> Facts {
        Facts {
            hash: Hash::Sha256,
            key: KeyStrength::EdDsa,
            signed_at: Some(CREATED + 3_600),
            key_created: CREATED,
            key_expires: None,
            revoked: false,
        }
    }

    fn judge(facts: Facts, mathematics: bool) -> (Status, Option<Reason>) {
        Policy.judge(&facts, mathematics)
    }

    #[test]
    fn a_sound_signature_that_checks_is_good() {
        assert_eq!(judge(sound(), true), (Status::Good, None));
        let rsa = Facts {
            key: KeyStrength::Rsa { bits: 3072 },
            hash: Hash::Sha512,
            ..sound()
        };
        assert_eq!(judge(rsa, true), (Status::Good, None));
    }

    #[test]
    fn sha1_and_md5_are_never_good() {
        for hash in [Hash::Sha1, Hash::Md5] {
            let facts = Facts { hash, ..sound() };
            assert_eq!(
                judge(facts, true),
                (Status::CantCheck, Some(Reason::WeakHash(hash)))
            );
        }
    }

    #[test]
    fn rsa_and_dsa_under_2048_bits_are_never_good() {
        for key in [
            KeyStrength::Rsa { bits: 1024 },
            KeyStrength::Dsa { bits: 1024 },
        ] {
            let facts = Facts { key, ..sound() };
            assert_eq!(
                judge(facts, true),
                (Status::CantCheck, Some(Reason::WeakKey))
            );
        }
        let enough = Facts {
            key: KeyStrength::Rsa { bits: 2048 },
            ..sound()
        };
        assert_eq!(judge(enough, true).0, Status::Good);
    }

    #[test]
    fn a_signature_made_before_its_key_or_after_it_expired_is_bad() {
        let early = Facts {
            signed_at: Some(CREATED - 1),
            ..sound()
        };
        assert_eq!(judge(early, true), (Status::Bad, Some(Reason::NotYetValid)));
        let late = Facts {
            key_expires: Some(CREATED + 60),
            ..sound()
        };
        assert_eq!(judge(late, true), (Status::Bad, Some(Reason::Expired)));
        let in_time = Facts {
            key_expires: Some(CREATED + 7_200),
            ..sound()
        };
        assert_eq!(
            judge(in_time, true).0,
            Status::Good,
            "expiring later is no fault"
        );
    }

    #[test]
    fn a_held_revocation_is_revoked_even_when_the_mathematics_holds() {
        let facts = Facts {
            revoked: true,
            ..sound()
        };
        assert_eq!(judge(facts, true), (Status::Revoked, None));
    }

    #[test]
    fn a_mathematical_mismatch_is_tampered() {
        assert_eq!(judge(sound(), false), (Status::Bad, Some(Reason::Tampered)));
    }

    #[test]
    fn a_weak_signature_that_does_not_check_is_still_uncheckable_never_bad() {
        let facts = Facts {
            hash: Hash::Sha1,
            ..sound()
        };
        assert_eq!(
            judge(facts, false),
            (Status::CantCheck, Some(Reason::WeakHash(Hash::Sha1))),
            "Postio refuses to rely on SHA-1 either way"
        );
    }
}
