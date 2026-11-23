//! Where a message's signature is, read from the MIME structure before any key is needed.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    PgpMime,
    InlinePgp,
    Smime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedScope {
    /// The message itself is `multipart/signed`: `part` is the signed entity's exact bytes, from
    /// after its boundary line to the CRLF before the next (RFC 3156 §5).
    WholeMessage {
        format: Format,
        part: Range<usize>,
        signature: Range<usize>,
    },
    /// A `multipart/signed` inside another part, a list footer's wrapper or a forward's: `path`
    /// is its IMAP part number.
    Part {
        format: Format,
        path: Vec<usize>,
        part: Range<usize>,
        signature: Range<usize>,
    },
    /// A cleartext-signed text part; `whole` when that text is the message's whole body.
    Inline {
        text: Range<usize>,
        whole: bool,
    },
    Encrypted,
}

impl SignedScope {
    pub fn format(&self) -> Option<Format> {
        match self {
            SignedScope::WholeMessage { format, .. } | SignedScope::Part { format, .. } => {
                Some(*format)
            }
            SignedScope::Inline { .. } => Some(Format::InlinePgp),
            SignedScope::Encrypted => None,
        }
    }
}

pub fn signed_scope(raw: &[u8]) -> Option<SignedScope> {
    let _ = raw;
    None
}

#[cfg(test)]
mod tests {
    use postio_model::test_corpus;

    use super::*;

    fn scope(fixture: &str) -> Option<SignedScope> {
        signed_scope(test_corpus::load(fixture).bytes())
    }

    #[test]
    fn a_signed_message_is_signed_whole_and_its_part_is_the_wire_bytes() {
        let raw = test_corpus::load("pgp-signed-good").bytes();
        let Some(SignedScope::WholeMessage {
            format,
            part,
            signature,
        }) = signed_scope(raw)
        else {
            panic!("{:?}", signed_scope(raw));
        };
        assert_eq!(format, Format::PgpMime);
        assert!(raw[part.clone()].starts_with(b"Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(
            raw[part].ends_with(b"\r\nAda"),
            "the CRLF before the boundary is not signed"
        );
        assert!(raw[signature].starts_with(b"-----BEGIN PGP SIGNATURE-----"));
    }

    #[test]
    fn a_list_footer_leaves_only_a_part_signed() {
        let Some(SignedScope::Part { path, format, .. }) = scope("pgp-signed-list-footer") else {
            panic!("{:?}", scope("pgp-signed-list-footer"));
        };
        assert_eq!((path, format), (vec![1], Format::PgpMime));
    }

    #[test]
    fn an_inline_signature_over_the_whole_body_is_inline_and_whole() {
        let raw = test_corpus::load("pgp-inline-signed").bytes();
        let Some(SignedScope::Inline { text, whole }) = signed_scope(raw) else {
            panic!("{:?}", signed_scope(raw));
        };
        assert!(whole);
        assert!(raw[text].starts_with(b"-----BEGIN PGP SIGNED MESSAGE-----"));
    }

    #[test]
    fn an_encrypted_message_is_encrypted_and_unsigned_mail_has_no_scope() {
        assert_eq!(scope("pgp-encrypted"), Some(SignedScope::Encrypted));
        for fixture in [
            "plain-text-simple",
            "autocrypt-header",
            "pgp-key-attached",
            "html-newsletter",
        ] {
            assert_eq!(scope(fixture), None, "{fixture}");
        }
    }

    #[test]
    fn nothing_parseable_is_no_scope_and_never_a_panic() {
        for raw in [
            &b""[..],
            b"\r\n\r\n",
            b"Content-Type: multipart/signed; boundary=x\r\n\r\n--x\r\n",
            b"Content-Type: multipart/signed\r\n\r\nno boundary",
        ] {
            let _ = signed_scope(raw);
        }
        for fixture in test_corpus::all() {
            let _ = signed_scope(fixture.bytes());
        }
    }
}
