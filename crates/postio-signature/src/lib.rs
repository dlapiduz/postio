//! Message signatures checked on this computer: bytes and keys in, a verdict out. A pure leaf with
//! no store, toolkit, runtime or network, because everything it parses was chosen by a sender.
#![forbid(unsafe_code)]
pub mod autocrypt;
pub mod gnupg;
pub mod keys;
pub mod policy;
pub mod scope;
pub mod verify;

pub use autocrypt::autocrypt;
pub use gnupg::{KeyRead, keyblocks_from_keybox, keyblocks_from_pubring_gpg};
pub use keys::{KeyInfo, keys_in};
pub use policy::{Facts, Hash, KeyStrength, Policy, Reason, Status};
pub use scope::{Format, SignedScope, signed_scope};
pub use verify::{KeySet, Verdict, verify};
