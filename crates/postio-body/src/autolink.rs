//! GFM's extended autolinks (GFM 0.29 §6.9), as one pure function.
//!
//! `pulldown-cmark` parses CommonMark's `<url>` autolinks but not GFM's
//! bare ones, so `www.example.com`, `https://example.com/x` and
//! `ada@example.com` would stay words in a Markdown draft while GitHub
//! links them. [`find`] applies GFM's rules -- what may precede one, what a
//! valid domain is, the trailing punctuation and parenthesis balance -- so
//! a draft links exactly what GitHub would (spec 014 R2). Every result is
//! an [`Href`], so nothing here can produce a link the document refuses.

use std::ops::Range;

use crate::document::{Href, Inline};

/// One autolink in a run of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Autolink {
    /// Where it is, in bytes.
    pub range: Range<usize>,
    /// Where it points: `http://` added to a `www.` link, `mailto:` to an
    /// address.
    pub href: Href,
}

/// Every extended autolink in `text`, in order, never overlapping.
pub fn find(text: &str) -> Vec<Autolink> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if text.is_char_boundary(at)
            && may_start(text, at)
            && let Some(link) = link_at(text, at)
        {
            at = link.range.end;
            found.push(link);
            continue;
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

/// `inlines` with every autolink in their text made a link. Text inside a
/// link or code is left alone, as GFM leaves it.
pub fn link_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match inline {
            Inline::Text(text) => split_text(&text, &mut out),
            Inline::Strong(inner) => out.push(Inline::Strong(link_inlines(inner))),
            Inline::Emphasis(inner) => out.push(Inline::Emphasis(link_inlines(inner))),
            Inline::Strikethrough(inner) => out.push(Inline::Strikethrough(link_inlines(inner))),
            other => out.push(other),
        }
    }
    out
}

fn split_text(text: &str, out: &mut Vec<Inline>) {
    let mut last = 0;
    for link in find(text) {
        if link.range.start > last {
            out.push(Inline::Text(text[last..link.range.start].to_owned()));
        }
        out.push(Inline::Link {
            href: link.href,
            inlines: vec![Inline::Text(text[link.range.clone()].to_owned())],
        });
        last = link.range.end;
    }
    if last < text.len() {
        out.push(Inline::Text(text[last..].to_owned()));
    }
}

/// GFM: an autolink begins a line, or follows whitespace or one of
/// `*`, `_`, `~`, `(`.
fn may_start(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || matches!(c, '*' | '_' | '~' | '('))
}

fn link_at(text: &str, at: usize) -> Option<Autolink> {
    let rest = &text[at..];
    let lower = rest.get(..8).unwrap_or(rest).to_ascii_lowercase();
    let (scheme, prefix) = if lower.starts_with("https://") {
        ("", 8)
    } else if lower.starts_with("http://") {
        ("", 7)
    } else if lower.starts_with("www.") {
        ("http://", 0)
    } else {
        return email_at(text, at);
    };
    let domain_len = valid_domain(&rest[prefix..])?;
    let mut end = prefix + domain_len;
    // The path runs to whitespace or `<`.
    end += rest[end..]
        .find(|c: char| c.is_whitespace() || c == '<')
        .unwrap_or(rest.len() - end);
    let end = trim_trailing(&rest[..end]);
    // Trailing punctuation can eat into the domain, never past it.
    if end <= prefix {
        return None;
    }
    let matched = &rest[..end];
    let href = Href::parse(&format!("{scheme}{matched}"))?;
    Some(Autolink {
        range: at..at + end,
        href,
    })
}

/// The length of a valid domain at the start of `text`: segments of
/// alphanumerics, `_` and `-` joined by `.`, at least two of them, and no
/// `_` in the last two.
fn valid_domain(text: &str) -> Option<usize> {
    let len = text
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .unwrap_or(text.len());
    let domain = text[..len].trim_end_matches('.');
    let segments: Vec<&str> = domain.split('.').collect();
    if segments.len() < 2 || segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    if segments[segments.len() - 2..]
        .iter()
        .any(|segment| segment.contains('_'))
    {
        return None;
    }
    Some(domain.len())
}

/// The end of a link once GFM's trailing rules have taken off what is
/// punctuation after it rather than part of it.
fn trim_trailing(link: &str) -> usize {
    let mut end = link.len();
    loop {
        let current = &link[..end];
        let Some(last) = current.chars().next_back() else {
            return end;
        };
        if matches!(
            last,
            '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"'
        ) {
            end -= last.len_utf8();
        } else if last == ')' {
            let open = current.matches('(').count();
            let close = current.matches(')').count();
            if close > open {
                end -= 1;
            } else {
                return end;
            }
        } else if last == ';' {
            // `&hl;` at the end is an entity reference, not part of the link.
            match current.rfind('&') {
                Some(amp)
                    if current[amp + 1..end - 1]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric())
                        && end - 1 > amp + 1 =>
                {
                    end = amp
                }
                _ => return end,
            }
        } else {
            return end;
        }
    }
}

/// An address: `[A-Za-z0-9._+-]+ @ [A-Za-z0-9_-]+ (. [A-Za-z0-9_-]+)+`,
/// not ending in `-` or `_`.
fn email_at(text: &str, at: usize) -> Option<Autolink> {
    let rest = &text[at..];
    let local = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')))
        .unwrap_or(rest.len());
    if local == 0 || !rest[local..].starts_with('@') {
        return None;
    }
    let domain = &rest[local + 1..];
    let len = domain
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .unwrap_or(domain.len());
    let domain = domain[..len].trim_end_matches('.');
    if !domain.contains('.') || domain.split('.').any(str::is_empty) || domain.ends_with(['-', '_'])
    {
        return None;
    }
    let end = local + 1 + domain.len();
    let address = &rest[..end];
    Some(Autolink {
        range: at..at + end,
        href: Href::parse(&format!("mailto:{address}"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(text: &str) -> Vec<(&str, String)> {
        find(text)
            .into_iter()
            .map(|link| (&text[link.range.clone()], link.href.as_str().to_owned()))
            .collect()
    }

    #[test]
    fn www_gets_http_and_a_scheme_keeps_its_own() {
        assert_eq!(
            links("see www.example.com and https://example.com/x?y=1"),
            [
                ("www.example.com", "http://www.example.com".to_owned()),
                (
                    "https://example.com/x?y=1",
                    "https://example.com/x?y=1".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn an_address_becomes_mailto() {
        assert_eq!(
            links("write to ada@example.com."),
            [("ada@example.com", "mailto:ada@example.com".to_owned())]
        );
        assert!(links("a@b").is_empty(), "no dot in the domain");
        assert!(
            links("ada@example.com-")
                .iter()
                .all(|(t, _)| !t.ends_with('-'))
        );
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_link() {
        assert_eq!(links("go to www.example.com.")[0].0, "www.example.com");
        assert_eq!(
            links("really? https://example.com/a!")[0].0,
            "https://example.com/a"
        );
        assert_eq!(links("(see www.example.com/a)")[0].0, "www.example.com/a");
        assert_eq!(links("www.example.com/a_(b)")[0].0, "www.example.com/a_(b)");
        assert_eq!(links("www.example.com/a&hl;")[0].0, "www.example.com/a");
    }

    #[test]
    fn a_link_ends_at_whitespace_or_a_less_than() {
        assert_eq!(links("www.example.com/a<b")[0].0, "www.example.com/a");
    }

    #[test]
    fn a_link_starts_only_where_gfm_lets_one_start() {
        assert!(links("xwww.example.com").is_empty());
        assert_eq!(links("*www.example.com*")[0].0, "www.example.com");
        assert_eq!(links("~https://example.com~")[0].0, "https://example.com");
    }

    #[test]
    fn a_domain_needs_a_dot_and_no_underscore_at_its_end() {
        assert!(links("https://localhost").is_empty());
        assert!(links("www.exa_mple.com").is_empty());
        assert_eq!(links("www.sub_domain.example.com").len(), 1);
    }

    #[test]
    fn link_inlines_splits_text_and_leaves_code_and_links_alone() {
        let inlines = link_inlines(vec![
            Inline::Text("at www.example.com now".to_owned()),
            Inline::Code("www.example.com".to_owned()),
        ]);
        assert_eq!(inlines.len(), 4, "{inlines:?}");
        assert!(
            matches!(&inlines[1], Inline::Link { href, .. } if href.as_str() == "http://www.example.com")
        );
        assert_eq!(inlines[3], Inline::Code("www.example.com".to_owned()));
    }
}
