//! The `Bcc` field never leaves in the bytes `DATA` sends (RFC 5322 §3.6.3): a `Bcc` header there
//! would hand everyone on To and Cc the bcc list.
//! io-smtp strips it inside `SmtpMessageSend`, which Postio does not use (ADR 0021's payload
//! boundary), so the rule is repeated here: header section only, folded lines included, and the
//! obsolete `Bcc :` spelling.

use std::borrow::Cow;

/// Only the header section is read: a body line starting with `Bcc:` is the body's business.
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
            kept.extend_from_slice(line);
            kept.extend_from_slice(rest);
            return Cow::Owned(kept);
        }
        if !starts_with_whitespace(line) {
            in_bcc = is_bcc(line);
        }
        if !in_bcc {
            kept.extend_from_slice(line);
        }
    }
    Cow::Owned(kept)
}

fn header_lines(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = raw;
    std::iter::from_fn(move || {
        let line = next_line(rest)?;
        rest = &rest[line.len()..];
        (!is_blank(line)).then_some(line)
    })
}

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

fn is_blank(line: &[u8]) -> bool {
    matches!(line, b"\r\n" | b"\n")
}

fn starts_with_whitespace(line: &[u8]) -> bool {
    matches!(line.first(), Some(b' ' | b'\t'))
}

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
