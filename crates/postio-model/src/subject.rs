//! Subject normalization, shared by threading and by the UI.

/// Reply and forward prefixes, lowercased, across the languages Postio is
/// likely to meet. Longer forms come first so `fwd` wins over `fw`.
const PREFIXES: &[&str] = &[
    "fwd", "fw", "re", "aw", "sv", "vs", "antw", "odp", "ref", "rif", "enc", "tr", "res",
];

/// Strips reply and forward decoration from a subject and folds case.
///
/// Used to group messages whose `References` chains were broken in transit — a
/// mailing list that rewrites headers, or a client that replies without
/// `In-Reply-To`. Deliberately conservative: a leading bracketed list tag such
/// as `[postio-dev]` is *kept*, because two different lists are two different
/// conversations, but prefixes after it are still stripped.
///
/// ```
/// use postio_model::normalize_subject;
/// assert_eq!(normalize_subject("RE: Re: FWD: Contract"), "contract");
/// assert_eq!(normalize_subject("[list] Re: Contract"), "[list] contract");
/// ```
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

/// Splits off a leading `[tag]`, returning the tag and everything after it.
fn split_leading_tag(subject: &str) -> (&str, &str) {
    if !subject.starts_with('[') {
        return ("", subject);
    }
    match subject.find(']') {
        Some(end) => (&subject[..=end], subject[end + 1..].trim_start()),
        None => ("", subject),
    }
}

/// Strips one `re:` / `fwd:` style prefix, including `re[2]:` and `re(2):`.
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

/// Whether a subject carries reply or forward decoration.
///
/// The safeguard on subject-based threading. JWZ groups by subject only as a
/// fallback for a broken `References` chain, and only where one side *looks*
/// like a reply — otherwise every message anyone ever titled "Hello" collapses
/// into one conversation, which is worse than the broken chain it was meant to
/// repair.
///
/// ```
/// use postio_model::subject::is_reply;
/// assert!(is_reply("Re: Contract"));
/// assert!(is_reply("[list] FWD: Contract"));
/// assert!(!is_reply("Contract"));
/// assert!(!is_reply("Reference check"));
/// ```
pub fn is_reply(subject: &str) -> bool {
    let collapsed = subject.to_lowercase();
    let (_, rest) = split_leading_tag(collapsed.trim());
    strip_one_prefix(rest).is_some()
}

/// Strips a `[2]` or `(2)` repetition counter, if one is there.
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
}
