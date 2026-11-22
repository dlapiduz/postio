/// Longer forms come first so `fwd` wins over `fw`.
const PREFIXES: &[&str] = &[
    "fwd", "fw", "re", "aw", "sv", "vs", "antw", "odp", "ref", "rif", "enc", "tr", "res",
];

/// How far apart, in days, two rootless conversations can be and still be the same to the unified
/// list's subject fallback
/// (#184, ADR 0005 Q2): across accounts a bare subject match would fold every "Weekly digest" into
/// one conversation.
pub const COALESCING_WINDOW_DAYS: i64 = 7;

/// Groups messages whose `References` were broken in transit. Conservative: a leading `[list]` tag
/// is kept, since two lists are two conversations, but prefixes after it are stripped.
pub fn normalize_subject(subject: &str) -> String {
    let collapsed = subject
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let (tag, rest) = split_leading_tag(&collapsed);

    let mut rest = rest;
    while let Some(stripped) = strip_one_prefix(rest) {
        rest = stripped;
    }
    let rest = rest.trim();

    match (tag.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_owned(),
        (false, true) => tag.to_owned(),
        (false, false) => format!("{tag} {rest}"),
    }
}

fn split_leading_tag(subject: &str) -> (&str, &str) {
    if !subject.starts_with('[') {
        return ("", subject);
    }
    match subject.find(']') {
        Some(end) => (&subject[..=end], subject[end + 1..].trim_start()),
        None => ("", subject),
    }
}

fn strip_one_prefix(subject: &str) -> Option<&str> {
    let subject = subject.trim_start();
    for prefix in PREFIXES {
        let Some(after_word) = subject.strip_prefix(prefix) else {
            continue;
        };
        let after_counter = strip_counter(after_word);
        if let Some(rest) = after_counter.trim_start().strip_prefix(':') {
            return Some(rest.trim_start());
        }
    }
    None
}

/// The safeguard on subject-based threading: JWZ groups by subject only where one side looks like a
/// reply, or every "Hello" collapses into one conversation.
pub fn is_reply(subject: &str) -> bool {
    let collapsed = subject.to_lowercase();
    let (_, rest) = split_leading_tag(collapsed.trim());
    strip_one_prefix(rest).is_some()
}

/// Checks only the `Re:` spelling, not `is_reply`'s multi-language recognition: this is Postio's
/// own outgoing subject, so replying to `Fwd: X` reads `Re: Fwd: X`.
pub fn reply_subject(original: &str) -> String {
    prefixed_subject(original, "Re:", &["re"])
}

/// Checks only its own prefix (`Fwd:`, `Fw:`), for the reason given on `reply_subject`.
pub fn forward_subject(original: &str) -> String {
    prefixed_subject(original, "Fwd:", &["fwd", "fw"])
}

fn prefixed_subject(original: &str, label: &str, prefixes: &[&str]) -> String {
    let trimmed = original.trim();
    if starts_with_one_of(trimmed, prefixes) {
        return trimmed.to_owned();
    }
    if trimmed.is_empty() {
        return label.to_owned();
    }
    format!("{label} {trimmed}")
}

fn starts_with_one_of(subject: &str, prefixes: &[&str]) -> bool {
    let lowered = subject.to_lowercase();
    for prefix in prefixes {
        let Some(after_word) = lowered.strip_prefix(prefix) else {
            continue;
        };
        let after_counter = strip_counter(after_word);
        if after_counter.trim_start().starts_with(':') {
            return true;
        }
    }
    false
}

fn strip_counter(subject: &str) -> &str {
    for (open, close) in [('[', ']'), ('(', ')')] {
        let Some(inner) = subject.strip_prefix(open) else {
            continue;
        };
        let Some(end) = inner.find(close) else {
            continue;
        };
        let digits = &inner[..end];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            return &inner[end + 1..];
        }
    }
    subject
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_subject_that_has_no_decoration() {
        assert_eq!(normalize_subject("Quarterly report"), "quarterly report");
    }

    #[test]
    fn collapses_runs_of_whitespace() {
        assert_eq!(normalize_subject("a   b\tc"), "a b c");
    }

    #[test]
    fn handles_a_subject_that_is_only_a_prefix() {
        assert_eq!(normalize_subject("Re:"), "");
        assert_eq!(normalize_subject("[list] Re:"), "[list]");
    }

    #[test]
    fn ignores_a_word_that_merely_starts_like_a_prefix() {
        assert_eq!(normalize_subject("Report ready"), "report ready");
        assert_eq!(normalize_subject("Reference check"), "reference check");
    }

    #[test]
    fn strips_non_english_prefixes() {
        assert_eq!(normalize_subject("AW: Vertrag"), "vertrag");
        assert_eq!(normalize_subject("SV: Avtal"), "avtal");
    }

    #[test]
    fn a_reply_is_recognized_wherever_its_prefix_sits() {
        assert!(is_reply("Re: Contract"));
        assert!(is_reply("re: contract"));
        assert!(is_reply("RE[2]: Contract"));
        assert!(is_reply("[postio-dev] Re: Contract"));
        assert!(is_reply("AW: Vertrag"));
    }

    #[test]
    fn an_ordinary_subject_is_not_a_reply() {
        assert!(!is_reply("Contract"));
        assert!(!is_reply("Reference check"), "not every word starting `re`");
        assert!(!is_reply("[postio-dev] Contract"));
        assert!(!is_reply(""));
    }

    #[test]
    fn leaves_an_unterminated_bracket_alone() {
        assert_eq!(normalize_subject("[list Re: x"), "[list re: x");
    }

    #[test]
    fn replying_prepends_re_exactly_once() {
        assert_eq!(reply_subject("Quarterly report"), "Re: Quarterly report");
        assert_eq!(
            reply_subject("Re: Quarterly report"),
            "Re: Quarterly report",
            "no Re: Re: stacking"
        );
        assert_eq!(
            reply_subject("RE: Quarterly report"),
            "RE: Quarterly report",
            "case-insensitive detection, but the original casing is kept"
        );
        assert_eq!(
            reply_subject("re[2]: Quarterly report"),
            "re[2]: Quarterly report",
            "an existing reply counter is not a fresh Re:"
        );
        assert_eq!(reply_subject(""), "Re:");
        assert_eq!(reply_subject("  "), "Re:");
    }

    #[test]
    fn replying_to_a_forward_adds_re_rather_than_folding_it_away() {
        assert_eq!(
            reply_subject("Fwd: Quarterly report"),
            "Re: Fwd: Quarterly report",
            "a reply to a forward is still a reply"
        );
    }

    #[test]
    fn forwarding_prepends_fwd_exactly_once() {
        assert_eq!(forward_subject("Quarterly report"), "Fwd: Quarterly report");
        assert_eq!(
            forward_subject("Fwd: Quarterly report"),
            "Fwd: Quarterly report"
        );
        assert_eq!(
            forward_subject("FW: Quarterly report"),
            "FW: Quarterly report",
            "the short form counts too"
        );
        assert_eq!(forward_subject(""), "Fwd:");
    }

    #[test]
    fn forwarding_a_reply_adds_fwd_rather_than_folding_it_away() {
        assert_eq!(
            forward_subject("Re: Quarterly report"),
            "Fwd: Re: Quarterly report",
            "a forward of a reply is still a forward"
        );
    }
}
