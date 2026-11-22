//! Keeping a rejection's own words. RFC 5321 §4.2.1 replies span several lines and providers put
//! the actionable part there, but `io-smtp` builds each rejection from `response.text()`, the first
//! line only, so the advice and help URL are gone (#921).
//! Postio drives the coroutine itself, so the bytes pass through [`ReplyTap`] first, which hands
//! them to `io-smtp`'s own `SmtpResponse::parse` rather than a second implementation of line
//! folding.

use io_smtp::rfc5321::SmtpResponse;

/// Replies arrive in whatever chunks the network gives, so this accumulates until `io-smtp` says
/// the buffer holds a whole one and keeps it; a reply is only ever replaced, so the one standing
/// when a rejection surfaces carried it.
#[derive(Debug, Default)]
pub(crate) struct ReplyTap {
    pending: Vec<u8>,
    last: Vec<u8>,
}

impl ReplyTap {
    pub(crate) fn saw(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        // `is_complete` is io-smtp's own test for a multiline reply having ended; asked after every
        // read because a reply can arrive in any number of chunks.
        if SmtpResponse::is_complete(&self.pending) {
            self.last = std::mem::take(&mut self.pending);
        }
    }

    /// The code has to match: on a path that read something after the rejection, or failed before
    /// reading, the tap holds a different reply, and attaching its words would put another exchange
    /// in front of a person as if the server had just said them.
    pub(crate) fn reason(&self, code: u16) -> Option<String> {
        let response = SmtpResponse::parse(&self.last).ok()?;
        if response.code.code() != code {
            return None;
        }
        let lines: Vec<&str> = response.lines.as_ref().iter().map(AsRef::as_ref).collect();
        (lines.len() > 1).then(|| join(&lines))
    }
}

/// Joined with `; `, never a newline: this string is formatted into `tracing` and the Attention
/// row, and a line break lets a server forge a log entry. The enhanced status code appears once
/// (RFC 3463 puts it on every line). Empty lines, and lines that are only the status code, are
/// dropped.
fn join(lines: &[&str]) -> String {
    let status = lines.first().and_then(|line| enhanced_status(line));
    let mut parts: Vec<&str> = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let mut text = line.trim();
        if index > 0
            && let Some(status) = status
            && let Some(rest) = text.strip_prefix(status)
        {
            text = rest.trim_start();
        }
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join("; ")
}

/// Deliberately narrow (`class.subject.detail`, digits and dots, at the very start): a looser
/// reading would strip the start of a message that merely opens with a version number.
fn enhanced_status(line: &str) -> Option<&str> {
    let candidate = line.split_whitespace().next()?;
    let mut parts = candidate.split('.');
    let class = parts.next()?;
    let subject = parts.next()?;
    let detail = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (matches!(class, "2" | "4" | "5") && digits(subject) && digits(detail)).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_is_itself() {
        assert_eq!(join(&["5.1.1 no such user"]), "5.1.1 no such user");
    }

    #[test]
    fn the_lines_read_as_one_refusal() {
        assert_eq!(
            join(&[
                "5.1.1 The address you entered does not exist",
                "5.1.1 Check for typos in the recipient",
                "5.1.1 https://example.invalid/help/nosuchuser",
            ]),
            "5.1.1 The address you entered does not exist; Check for typos in \
             the recipient; https://example.invalid/help/nosuchuser"
        );
    }

    #[test]
    fn the_status_code_is_quoted_once_and_kept() {
        let joined = join(&["5.7.1 refused", "5.7.1 see the policy", "5.7.1 goodbye"]);
        assert_eq!(joined.matches("5.7.1").count(), 1);
        assert!(joined.starts_with("5.7.1 "));
    }

    #[test]
    fn a_reply_with_no_status_code_is_left_exactly_as_it_came() {
        assert_eq!(
            join(&["mailbox unavailable", "try again from another host"]),
            "mailbox unavailable; try again from another host"
        );
    }

    #[test]
    fn a_different_status_code_on_a_later_line_is_not_stripped() {
        let joined = join(&["5.1.1 no such user", "5.1.2 nor such domain"]);
        assert!(joined.contains("5.1.2"), "{joined}");
    }

    #[test]
    fn empty_continuations_leave_no_stray_separators() {
        assert_eq!(
            join(&["5.1.1 refused", "5.1.1", "  ", "5.1.1 done"]),
            "5.1.1 refused; done"
        );
    }

    #[test]
    fn a_version_number_at_the_start_is_not_mistaken_for_a_status_code() {
        assert_eq!(
            enhanced_status("1.2.3 hello"),
            None,
            "1 is not a reply class"
        );
        assert_eq!(
            enhanced_status("5.1.1.1 hello"),
            None,
            "four parts is not one"
        );
        assert_eq!(enhanced_status("5.x.1 hello"), None);
        assert_eq!(enhanced_status("5.1.1 hello"), Some("5.1.1"));
    }

    #[test]
    fn the_tap_keeps_the_last_complete_reply_across_chunked_reads() {
        let mut tap = ReplyTap::default();
        tap.saw(b"250 2.1.0 sender ok\r\n");
        tap.saw(b"550-5.1.1 no such user\r\n550 5.1.1 ");
        assert_eq!(
            tap.reason(550),
            None,
            "half a reply is not a reply; the tap still holds the one before it"
        );
        tap.saw(b"check the address\r\n");
        assert_eq!(
            tap.reason(550).as_deref(),
            Some("5.1.1 no such user; check the address")
        );
    }

    #[test]
    fn the_tap_refuses_to_answer_for_a_different_code() {
        let mut tap = ReplyTap::default();
        tap.saw(b"550-5.1.1 no such user\r\n550 5.1.1 check the address\r\n");
        assert_eq!(tap.reason(451), None);
    }

    #[test]
    fn a_single_line_reply_is_left_to_the_caller() {
        let mut tap = ReplyTap::default();
        tap.saw(b"550 5.1.1 no such user\r\n");
        assert_eq!(tap.reason(550), None);
    }

    #[test]
    fn no_reply_at_all_answers_nothing() {
        assert_eq!(ReplyTap::default().reason(550), None);
    }
}
