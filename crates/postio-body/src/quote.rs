//! Quoted-text folding: `postio-1bz`.
//!
//! A reply chain a few messages deep is unreadable if every quoted ancestor
//! is fully on screen. The fix has to cost nothing to toggle and nothing to
//! render, which rules out anything JavaScript-shaped — the reader has none.
//! `<details>`/`<summary>` is the answer: closed by default with no `open`
//! attribute, expands on click, and WebKit does both without a script
//! running. Postio only has to find the quoted spans and wrap them.
//!
//! Two entry points, for the body forms `postio-model::MessageBody` carries:
//!
//! * [`fold_html_quotes`] wraps every top-level `<blockquote>` in already
//!   sanitized HTML — the shape every mail client's HTML editor emits for a
//!   quoted reply.
//! * [`text_to_html`] is what a `text/plain` body becomes on the way into the
//!   `WebView`: HTML-escaped and `<pre>`-wrapped, with contiguous runs of
//!   `>`-prefixed lines folded the same way.
//!
//! Neither ever removes a byte of content — folding only wraps it, so
//! "expand" always gets back exactly what was quoted.

/// Wrap every outermost `<blockquote>…</blockquote>` in sanitized HTML with a
/// collapsed `<details>`.
///
/// Only the outermost span of a nested quote chain is wrapped — expanding it
/// reveals every ancestor at once, which is what every mail client that nests
/// `<blockquote>` for `> >` quoting means by the nesting. Text outside a
/// `<blockquote>` is copied through untouched, byte for byte.
///
/// Input is assumed to be well-formed, already-sanitized markup (see
/// [`crate::sanitize::sanitize_body`]): tags are matched by their literal
/// spelling, not parsed, which is only safe because `ammonia`'s serializer
/// never emits the literal text `<blockquote` inside an attribute value.
pub fn fold_html_quotes(html: &str) -> String {
    const OPEN: &str = "<blockquote";
    const CLOSE: &str = "</blockquote>";

    let mut out = String::with_capacity(html.len() + 96);
    let mut depth: usize = 0;
    let mut span_start: usize = 0;
    let mut i = 0;

    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with(OPEN) && tag_name_ends_at(rest, OPEN.len()) {
            if depth == 0 {
                span_start = i;
            }
            depth += 1;
            i += OPEN.len();
            continue;
        }
        if depth > 0 && rest.starts_with(CLOSE) {
            depth -= 1;
            i += CLOSE.len();
            if depth == 0 {
                wrap_quote(&mut out, &html[span_start..i]);
            }
            continue;
        }
        let ch_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        if depth == 0 {
            out.push_str(&rest[..ch_len]);
        }
        i += ch_len;
    }

    // An unmatched `<blockquote` (malformed input slipped past sanitizing)
    // must not eat the rest of the message — emit it unwrapped rather than
    // drop it silently.
    if depth > 0 {
        out.push_str(&html[span_start..]);
    }

    out
}

fn tag_name_ends_at(rest: &str, at: usize) -> bool {
    rest.as_bytes()
        .get(at)
        .is_none_or(|b| *b == b'>' || b.is_ascii_whitespace() || *b == b'/')
}

fn wrap_quote(out: &mut String, span: &str) {
    out.push_str("<details class=\"postio-quote\"><summary>Show quoted text\u{2026}</summary>");
    out.push_str(span);
    out.push_str("</details>");
}

/// Render a `text/plain` body as HTML: escaped and `<pre>`-wrapped, with
/// contiguous runs of `>`-prefixed lines folded into a collapsed
/// [`fold_html_quotes`]-style `<details>`.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 128);
    let mut lines = text.lines().peekable();
    let mut plain_run: Vec<&str> = Vec::new();

    while let Some(line) = lines.next() {
        if !is_quote_line(line) {
            plain_run.push(line);
            continue;
        }
        flush_plain_run(&mut out, &mut plain_run);

        let mut quote_run = vec![line];
        while let Some(next) = lines.peek() {
            if !is_quote_line(next) {
                break;
            }
            quote_run.push(lines.next().expect("just peeked Some"));
        }
        out.push_str("<details class=\"postio-quote\"><summary>Show quoted text\u{2026}</summary>");
        push_pre(&mut out, &quote_run);
        out.push_str("</details>");
    }
    flush_plain_run(&mut out, &mut plain_run);

    out
}

fn is_quote_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn flush_plain_run(out: &mut String, run: &mut Vec<&str>) {
    if run.is_empty() {
        return;
    }
    push_pre(out, run);
    run.clear();
}

fn push_pre(out: &mut String, lines: &[&str]) {
    out.push_str("<pre class=\"postio-body-text\">");
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        escape_into(out, line);
    }
    out.push_str("</pre>");
}

fn escape_into(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_top_level_blockquote_is_wrapped_in_a_collapsed_details() {
        let out = fold_html_quotes("<p>hi</p><blockquote><p>quoted</p></blockquote>");
        assert_eq!(
            out,
            "<p>hi</p><details class=\"postio-quote\"><summary>Show quoted text\u{2026}</summary>\
             <blockquote><p>quoted</p></blockquote></details>"
        );
    }

    #[test]
    fn nested_blockquotes_are_wrapped_once_as_a_unit() {
        // The Gmail/Apple Mail shape: `> >` becomes nested <blockquote>s.
        let html = "<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>";
        let out = fold_html_quotes(html);
        assert_eq!(
            out,
            format!(
                "<details class=\"postio-quote\"><summary>Show quoted text\u{2026}</summary>{html}</details>"
            )
        );
        // Expanding the outer <details> reveals the whole nested chain.
        assert_eq!(out.matches("<details").count(), 1);
    }

    #[test]
    fn text_outside_a_blockquote_is_never_touched() {
        let out = fold_html_quotes("<p>before</p><blockquote>q</blockquote><p>after</p>");
        assert!(out.starts_with("<p>before</p>"));
        assert!(out.ends_with("<p>after</p>"));
    }

    #[test]
    fn a_message_with_no_quote_is_returned_unchanged() {
        let html = "<p>just a newsletter, no reply chain</p>";
        assert_eq!(fold_html_quotes(html), html);
    }

    #[test]
    fn two_sibling_quotes_are_folded_independently() {
        let out =
            fold_html_quotes("<blockquote>a</blockquote><p>mid</p><blockquote>b</blockquote>");
        assert_eq!(out.matches("<details").count(), 2);
        assert!(out.contains("<p>mid</p>"));
    }

    #[test]
    fn plain_text_quote_markers_are_folded_and_escaped() {
        // The corpus's plain-text-flowed-reply.eml shape.
        let text = "On 2026-02-10, Ada Norwood wrote:\n> Short note <b>before</b>\n> to cover\n\nThat order works.";
        let out = text_to_html(text);
        assert!(out.contains("On 2026-02-10, Ada Norwood wrote:"));
        assert!(
            out.contains("<details class=\"postio-quote\">"),
            "the `>` run should fold: {out}"
        );
        // Escaped, not interpreted as markup.
        assert!(out.contains("&lt;b&gt;before&lt;/b&gt;"), "{out}");
        assert!(out.contains("That order works."));
        // The "wrote:" line introduces the quote but is not itself quoted,
        // so it must land before the <details> it precedes.
        let details_start = out.find("<details").unwrap();
        let wrote_at = out.find("wrote:").unwrap();
        assert!(wrote_at < details_start, "{out}");
    }

    #[test]
    fn text_with_no_quote_marker_produces_a_single_pre() {
        let out = text_to_html("just two\nplain lines");
        assert_eq!(out.matches("<pre").count(), 1);
        assert!(!out.contains("<details"));
    }

    #[test]
    fn a_message_that_is_entirely_quoted_still_folds() {
        let out = text_to_html("> all quoted\n> every line");
        assert!(out.starts_with("<details class=\"postio-quote\">"));
    }
}
