//! Keeping a rejection's own words, all of them.
//!
//! # Why this exists
//!
//! RFC 5321 §4.2.1 lets a reply be several lines, all carrying the same code,
//! with `-` after the code on every line but the last. The large providers use
//! that for the half a person can act on:
//!
//! ```text
//! 550-5.1.1 The address you entered does not exist
//! 550-5.1.1 Check for typos in the recipient
//! 550 5.1.1 https://example.invalid/help/nosuchuser
//! ```
//!
//! `io-smtp` parses every line into [`SmtpResponse::lines`], and then its own
//! error construction keeps only the first: each rejection is built from
//! `response.text()`, which is documented as *"the first (or only) line"* and
//! returns `lines[0]`. Every rejection path in the crate does it — `MAIL`,
//! `RCPT`, `DATA`, `STARTTLS`, `AUTH`, `NOOP`, `QUIT` — and no error variant
//! carries the response, so by the time an `SmtpError` reaches us the advice
//! and the help URL are gone (#921).
//!
//! # Why the fix is here and not a second parser
//!
//! Postio drives the coroutine itself: every `WantsRead` is a `read` this
//! crate performs into a buffer it owns, and the bytes go to `io-smtp` from
//! there. So the reply is in our hands before it is truncated, and recovering
//! it needs no wire parsing of our own — [`ReplyTap`] hands those same bytes
//! to `io-smtp`'s **own** public parser, `SmtpResponse::parse`, and reads the
//! `lines` field it fills.
//!
//! That is the Pimalaya-first answer CLAUDE.md asks for: the alternative is a
//! second implementation of §4.2.1 line folding in this crate, which would be
//! free to disagree with the one actually deciding what the reply meant.

use io_smtp::rfc5321::SmtpResponse;

/// The last complete reply this session read.
///
/// Fed every byte the session reads, in the order it reads them. Replies
/// arrive in whatever chunks the network gives, so this accumulates until
/// `io-smtp` says the buffer holds a whole one and then keeps it — a reply is
/// only ever *replaced*, so the one standing when a rejection surfaces is the
/// reply that carried it.
#[derive(Debug, Default)]
pub(crate) struct ReplyTap {
    /// Bytes of the reply being read now.
    pending: Vec<u8>,
    /// The last reply that was complete.
    last: Vec<u8>,
}

impl ReplyTap {
    /// Record bytes just read from the server.
    pub(crate) fn saw(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        // `is_complete` is io-smtp's own test for "the last CRLF-terminated
        // line has `ddd SP` rather than `ddd-`", which is the only reliable
        // way to know a multiline reply has ended. Asked after every read
        // because a reply can arrive in any number of chunks.
        if SmtpResponse::is_complete(&self.pending) {
            self.last = std::mem::take(&mut self.pending);
        }
    }

    /// The whole reason for a rejection carrying `code`, or `None` when this
    /// tap has nothing better than what the caller already has.
    ///
    /// **The code has to match.** A tap holds the last complete reply, and on
    /// a path that read something after the rejection — or that failed before
    /// reading anything — that reply is a different one. Attaching its text to
    /// this error would put another exchange's words in front of a person as
    /// if the server had just said them, which is worse than the truncation
    /// this fixes.
    pub(crate) fn reason(&self, code: u16) -> Option<String> {
        let response = SmtpResponse::parse(&self.last).ok()?;
        if response.code.code() != code {
            return None;
        }
        let lines: Vec<&str> = response.lines.as_ref().iter().map(AsRef::as_ref).collect();
        (lines.len() > 1).then(|| join(&lines))
    }
}

/// One reply's lines, as one sentence.
///
/// Three decisions, and each is about how it reads rather than about SMTP:
///
/// * **Joined with `; `.** A reply's lines are clauses of one refusal, not
///   three refusals. A newline would be the obvious separator and is refused
///   outright: this string is formatted into `tracing` and into the Attention
///   row, and a reason carrying a line break lets a server forge a log entry.
/// * **The enhanced status code appears once.** RFC 3463 puts `5.1.1` on
///   *every* line, so joining verbatim repeats it as many times as the server
///   was thorough. It is worth quoting once and reads as noise three times.
/// * **Empty lines are dropped**, and a line that is nothing but the status
///   code with it: neither says anything, and both leave a stray `; `.
fn join(lines: &[&str]) -> String {
    let status = lines.first().and_then(|line| enhanced_status(line));
    let mut parts: Vec<&str> = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let mut text = line.trim();
        // Only from the continuation lines: the first keeps it, so the code
        // a person can quote to their provider is still in the message.
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

/// The RFC 3463 status code a line begins with — `5.1.1`, `4.7.0` — if it
/// does.
///
/// Deliberately narrow: `class.subject.detail`, digits and dots only, and
/// only at the very start. A looser reading would strip the beginning of a
/// message that merely opens with a version number.
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
        // Once, because it is what a person quotes to their provider; not
        // three times, because that reads as three failures.
        let joined = join(&["5.7.1 refused", "5.7.1 see the policy", "5.7.1 goodbye"]);
        assert_eq!(joined.matches("5.7.1").count(), 1);
        assert!(joined.starts_with("5.7.1 "));
    }

    #[test]
    fn a_reply_with_no_status_code_is_left_exactly_as_it_came() {
        // Plenty of servers do not send RFC 3463 codes at all, and nothing
        // here may invent or remove text on one that does not.
        assert_eq!(
            join(&["mailbox unavailable", "try again from another host"]),
            "mailbox unavailable; try again from another host"
        );
    }

    #[test]
    fn a_different_status_code_on_a_later_line_is_not_stripped() {
        // Stripping by prefix, not by shape: a continuation line carrying a
        // *different* code is saying something else, and removing it would
        // change what the server said.
        let joined = join(&["5.1.1 no such user", "5.1.2 nor such domain"]);
        assert!(joined.contains("5.1.2"), "{joined}");
    }

    #[test]
    fn empty_continuations_leave_no_stray_separators() {
        // `550-5.1.1 \r\n550 5.1.1 done` is legal and pointless, and it must
        // not come out as `; ; done`.
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
        // The network gives whatever chunks it gives, and a multiline reply
        // routinely spans several.
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
        // The guard that stops another exchange's words being put in front of
        // a person as if the server had just said them.
        let mut tap = ReplyTap::default();
        tap.saw(b"550-5.1.1 no such user\r\n550 5.1.1 check the address\r\n");
        assert_eq!(tap.reason(451), None);
    }

    #[test]
    fn a_single_line_reply_is_left_to_the_caller() {
        // Nothing was lost, so there is nothing to recover, and answering
        // would only reformat what io-smtp already gave us.
        let mut tap = ReplyTap::default();
        tap.saw(b"550 5.1.1 no such user\r\n");
        assert_eq!(tap.reason(550), None);
    }

    #[test]
    fn no_reply_at_all_answers_nothing() {
        assert_eq!(ReplyTap::default().reason(550), None);
    }
}
