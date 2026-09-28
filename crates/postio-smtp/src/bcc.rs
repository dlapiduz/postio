//! The `Bcc` field never leaves in the bytes `DATA` sends.
//!
//! `DATA` goes to every envelope recipient as it is, so a `Bcc` header there
//! hands everyone on `To` and `Cc` the list of people who were bcc'd -- the
//! disclosure `Bcc` exists to prevent (RFC 5322 §3.6.3). `outgoing::build`
//! never writes one for the sent copy, and this is the transport holding the
//! same line on its own, whatever bytes reach it.
//!
//! io-smtp 0.4 strips `Bcc` inside its `SmtpMessageSend`, which Postio does
//! not use: the session writes the `DATA` exchange out itself, for ADR
//! 0021's payload boundary. So the rule is repeated here, on the same terms
//! as io-smtp's: the header section only, every `Bcc` field with its folded
//! lines, and the obsolete `Bcc :` spelling (RFC 5322 §4.5.3).

use std::borrow::Cow;

/// `raw` without its `Bcc` fields, borrowed when it has none.
///
/// Only the header section is read: it ends at the first empty line, and a
/// body line that starts with `Bcc:` is the body's business.
pub(crate) fn strip_bcc(raw: &[u8]) -> Cow<'_, [u8]> {
    if !header_lines(raw).any(is_bcc) {
        return Cow::Borrowed(raw);
    }
    let mut kept = Vec::with_capacity(raw.len());
    let mut in_bcc = false;
    let mut rest = raw;
    while let Some(line) = next_line(rest) {
        rest = &rest[line.len()..];
        if is_blank(line) {
            // The end of the header section: the rest goes as it is.
            kept.extend_from_slice(line);
            kept.extend_from_slice(rest);
            return Cow::Owned(kept);
        }
        // A folded line continues whatever field it follows, so it goes or
        // stays with that field.
        if !starts_with_whitespace(line) {
            in_bcc = is_bcc(line);
        }
        if !in_bcc {
            kept.extend_from_slice(line);
        }
    }
    Cow::Owned(kept)
}

/// The header section's lines, each with its line ending.
fn header_lines(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = raw;
    std::iter::from_fn(move || {
        let line = next_line(rest)?;
        rest = &rest[line.len()..];
        (!is_blank(line)).then_some(line)
    })
}

/// The next line of `rest`, up to and including its `\n`, or what remains.
fn next_line(rest: &[u8]) -> Option<&[u8]> {
    if rest.is_empty() {
        return None;
    }
    let end = rest
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(rest.len(), |at| at + 1);
    Some(&rest[..end])
}

/// A line with nothing on it but its line ending.
fn is_blank(line: &[u8]) -> bool {
    matches!(line, b"\r\n" | b"\n")
}

fn starts_with_whitespace(line: &[u8]) -> bool {
    matches!(line.first(), Some(b' ' | b'\t'))
}

/// Whether a header line opens a `Bcc` field, allowing whitespace before
/// the colon (RFC 5322 §4.5.3's obsolete syntax).
fn is_bcc(line: &[u8]) -> bool {
    !starts_with_whitespace(line)
        && line
            .iter()
            .position(|&byte| byte == b':')
            .is_some_and(|colon| line[..colon].trim_ascii_end().eq_ignore_ascii_case(b"bcc"))
}

#[cfg(test)]
mod tests {
    use super::strip_bcc;

    #[test]
    fn the_bcc_field_goes_and_everything_else_stays() {
        let raw = b"From: a@example.com\r\nTo: b@example.com\r\nBcc: c@example.com\r\n\
                    Subject: s\r\n\r\nBcc: in the body stays\r\n";
        assert_eq!(
            strip_bcc(raw).as_ref(),
            b"From: a@example.com\r\nTo: b@example.com\r\nSubject: s\r\n\r\n\
              Bcc: in the body stays\r\n"
        );
    }

    #[test]
    fn a_folded_bcc_field_goes_with_every_line_it_folds_onto() {
        let raw = b"From: a@example.com\r\nBCC: c@example.com,\r\n d@example.com,\r\n\
                    \te@example.com\r\nTo: b@example.com\r\n\r\nbody\r\n";
        assert_eq!(
            strip_bcc(raw).as_ref(),
            b"From: a@example.com\r\nTo: b@example.com\r\n\r\nbody\r\n"
        );
    }

    #[test]
    fn the_obsolete_spelling_with_space_before_the_colon_is_a_bcc_field_too() {
        let raw =
            b"From: a@example.com\r\nBcc : c@example.com\r\nTo: b@example.com\r\n\r\nbody\r\n";
        assert_eq!(
            strip_bcc(raw).as_ref(),
            b"From: a@example.com\r\nTo: b@example.com\r\n\r\nbody\r\n"
        );
    }

    #[test]
    fn a_field_that_only_starts_like_bcc_is_left_alone() {
        let raw = b"From: a@example.com\r\nBcc-Note: kept\r\nX-Bcc: kept\r\n\r\nbody\r\n";
        assert_eq!(strip_bcc(raw).as_ref(), raw.as_slice());
    }

    #[test]
    fn a_message_with_no_bcc_is_handed_back_untouched() {
        let raw = b"From: a@example.com\r\nTo: b@example.com\r\n\r\nbody\r\n";
        assert!(matches!(strip_bcc(raw), std::borrow::Cow::Borrowed(_)));
    }
}
