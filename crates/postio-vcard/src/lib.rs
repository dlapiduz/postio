//! vCard import and export.
//!
//! This crate maps between vCard files and Postio's contacts: which card
//! properties become a person's name, addresses, organisation and note, which
//! cards are groups, and how a card Postio has edited is written back out. It
//! does no storage and no I/O of its own — the app reads the file the user
//! picked, hands the bytes here, and writes what comes back into the store.
//!
//! Parsing is Pimalaya's `vcard-rs`, chosen because it round-trips every
//! property it does not understand byte-for-byte, which is the property an
//! honest import-then-export needs (`specs/005-contacts/research.md` R9). It
//! is a leaf: no database engine, no toolkit, no async runtime —
//! `scripts/checks/check-crate-boundaries.py` holds it to that.
