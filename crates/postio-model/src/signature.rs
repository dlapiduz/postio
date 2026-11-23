//! The RFC 3676 §4.3 `-- ` separator means everything after it is signature, so it must be last.
//! `apply` replaces rather than appends: idempotent, and an identity switch swaps one signature for
//! the other.

pub const SEPARATOR: &str = "-- ";

/// The last separator wins, so a quoted earlier signature does not shadow this draft's own. A
/// separator with quoted lines after it is somebody else's signature inside the text.
pub fn split(body: &str) -> (&str, Option<&str>) {
    let mut separator: Option<(usize, usize)> = None;
    let mut offset = 0;

    for line in body.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.trim_end() == "--" && !content.starts_with(' ') {
            separator = Some((offset, offset + line.len()));
        } else if separator.is_some() && content.starts_with('>') {
            separator = None;
        }
        offset += line.len();
    }

    match separator {
        Some((start, end)) => (&body[..start], Some(body[end..].trim_end())),
        None => (body, None),
    }
}

/// Idempotent: the body is split first, so applying twice leaves one signature and switching
/// identities swaps one for the other. `None` or an empty signature removes the block.
pub fn apply(body: &str, signature: Option<&str>) -> String {
    let (written, _) = split(body);
    let written = written.trim_end_matches(['\n', '\r']);

    match signature.map(str::trim_end).filter(|text| !text.is_empty()) {
        None => {
            if written.is_empty() {
                String::new()
            } else {
                format!("{written}\n")
            }
        }
        Some(signature) => format!("{written}\n\n{SEPARATOR}\n{signature}\n"),
    }
}

/// The composer inserts a signature before anything is typed; counting it as content would make
/// every abandoned composer permanent.
pub fn is_only_signature(body: &str) -> bool {
    split(body).0.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_without_a_separator_is_all_message() {
        assert_eq!(split("Looking now."), ("Looking now.", None));
        assert_eq!(split(""), ("", None));
    }

    #[test]
    fn the_last_separator_is_the_one_that_counts() {
        let body = "Looking now.\n\n-- \nquoted sig\n\nand more\n\n-- \nAda\n";
        let (written, signature) = split(body);
        assert_eq!(signature, Some("Ada"));
        assert!(written.ends_with("and more\n\n"));
    }

    #[test]
    fn a_separator_with_quoted_text_under_it_belongs_to_the_quote() {
        let body = "Looking now.\n\n> -- \n> Diogo\n";
        assert_eq!(split(body).1, None);

        let body = "Looking now.\n\n-- \nDiogo\n> still quoting\n";
        assert_eq!(split(body).1, None, "a quote below it is not a signature");
    }

    #[test]
    fn applying_the_same_signature_twice_leaves_one() {
        let once = apply("Looking now.", Some("Ada"));
        assert_eq!(once, "Looking now.\n\n-- \nAda\n");
        assert_eq!(apply(&once, Some("Ada")), once);
        assert_eq!(apply(&apply(&once, Some("Ada")), Some("Ada")), once);
    }

    #[test]
    fn switching_identity_replaces_the_signature_rather_than_stacking_it() {
        let ada = apply("Looking now.", Some("Ada\nlovelace.example.com"));
        let grace = apply(&ada, Some("Grace"));
        assert_eq!(grace, "Looking now.\n\n-- \nGrace\n");
        assert!(
            !grace.contains("Ada"),
            "the old signature is gone: {grace:?}"
        );
    }

    #[test]
    fn an_identity_without_a_signature_takes_the_block_away() {
        let with = apply("Looking now.", Some("Ada"));
        assert_eq!(apply(&with, None), "Looking now.\n");
        assert_eq!(apply(&with, Some("   ")), "Looking now.\n");
        assert_eq!(apply("", None), "");
    }

    #[test]
    fn a_signature_lands_below_quoted_text_because_the_separator_says_so() {
        let reply = "\n\n> Small diff, mostly the folder walker.\n";
        let applied = apply(reply, Some("Ada"));
        assert!(
            applied.ends_with("> Small diff, mostly the folder walker.\n\n-- \nAda\n"),
            "{applied:?}"
        );
    }

    #[test]
    fn a_body_that_is_only_a_signature_counts_as_nothing_written() {
        assert!(is_only_signature(&apply("", Some("Ada"))));
        assert!(is_only_signature(""));
        assert!(!is_only_signature(&apply("Looking now.", Some("Ada"))));
    }
}
