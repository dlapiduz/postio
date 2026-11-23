//! SMTP sending for Postio over `io-smtp`: a sibling of `postio-account`, not a dependent, so
//! sending mail never needs the IMAP stack to compile. Shared shapes (the password, a cancellation
//! token) are duplicated in miniature.
//! No pool, unlike IMAP: a send opens a connection, runs one mail transaction and closes it.

mod bcc;
pub mod cancel;
pub mod error;
mod reply;
pub mod session;
pub mod settings;
pub mod transport;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
