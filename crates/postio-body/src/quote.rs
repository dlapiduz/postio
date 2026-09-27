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
//!
//! Both fold the [`Stretch`]es [`html_stretches`] and [`text_stretches`] find.
//! They are the one quote detector: own-text extraction
//! ([`crate::own_text()`]) leaves out exactly the stretches the reader folds.

/// A stretch of a message body: the sender's own words, or history they
/// quoted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stretch<'a> {
    /// The sender's own words, and whatever markup carries them.
    Own(&'a str),
    /// Quoted history: an outermost `<blockquote>…</blockquote>`, or a run
    /// of `>`-prefixed lines.
    Quoted(&'a str),
}

/// Sanitized HTML, split at its outermost `<blockquote>` elements.
///
/// Only the outermost span of a nested quote chain is one stretch, which is
/// what every mail client that nests `<blockquote>` for `> >` quoting means
/// by the nesting. Every byte of `html` is in exactly one stretch, in order.
///
/// Input is assumed to be well-formed, already-sanitized markup (see
/// [`crate::sanitize::sanitize_body`]): tags are matched by their literal
/// spelling, not parsed, which is only safe because `ammonia`'s serializer
/// never emits the literal text `<blockquote` inside an attribute value.
pub fn html_stretches(html: &str) -> Vec<Stretch<'_>> {
    const OPEN: &str = "<blockquote";
    const CLOSE: &str = "</blockquote>";

    let mut stretches = Vec::new();
    let mut depth: usize = 0;
    let mut own_start: usize = 0;
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
                if own_start < span_start {
                    stretches.push(Stretch::Own(&html[own_start..span_start]));
                }
                stretches.push(Stretch::Quoted(&html[span_start..i]));
                own_start = i;
            }
            continue;
        }
        i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
    }

    // An unmatched `<blockquote` (malformed input slipped past sanitizing)
    // must not eat the rest of the message: it stays the sender's own,
    // rather than being folded or dropped.
    if own_start < html.len() {
        stretches.push(Stretch::Own(&html[own_start..]));
    }
    stretches
}

/// A `text/plain` body, split at its contiguous runs of `>`-prefixed lines.
///
/// Every byte of `text` is in exactly one stretch, in order, and each
/// stretch is whole lines, line endings included.
pub fn text_stretches(text: &str) -> Vec<Stretch<'_>> {
    let mut stretches = Vec::new();
    let mut start = 0;
    let mut at = 0;
    let mut quoted: Option<bool> = None;
    for line in text.split_inclusive('\n') {
        let quote = is_quote_line(line);
        if let Some(run) = quoted
            && run != quote
        {
            stretches.push(stretch(&text[start..at], run));
            start = at;
        }
        quoted = Some(quote);
        at += line.len();
    }
    if let Some(run) = quoted {
        stretches.push(stretch(&text[start..], run));
    }
    stretches
}

fn stretch(text: &str, quoted: bool) -> Stretch<'_> {
    if quoted {
        Stretch::Quoted(text)
    } else {
        Stretch::Own(text)
    }
}

/// Wrap every outermost `<blockquote>…</blockquote>` in sanitized HTML with a
/// collapsed `<details>`.
///
/// Only the outermost span of a nested quote chain is wrapped — expanding it
/// reveals every ancestor at once. Text outside a `<blockquote>` is copied
/// through untouched, byte for byte. The spans are [`html_stretches`]'s.
pub fn fold_html_quotes(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 96);
    for stretch in html_stretches(html) {
        match stretch {
            Stretch::Own(own) => out.push_str(own),
            Stretch::Quoted(quoted) => wrap_quote(&mut out, quoted),
        }
    }
    out
}

fn tag_name_ends_at(rest: &str, at: usize) -> bool {
    rest.as_bytes()
        .get(at)
        .is_none_or(|b| *b == b'>' || b.is_ascii_whitespace() || *b == b'/')
}

fn wrap_quote(out: &mut String, span: &str) {
    open_fold(out, html_quote_lines(span));
    out.push_str(span);
    out.push_str("</details>");
}

/// A quote fold's opening, and the summary that says how much it hides.
fn open_fold(out: &mut String, lines: usize) {
    out.push_str("<details class=\"postio-quote\"><summary>");
    out.push_str(&fold_label(lines));
    out.push_str("</summary>");
}

/// What a quote's fold says, open or closed: how many lines of words it
/// holds (spec 007 FR-034, "31 quoted lines"). One label for a single
/// message and a conversation, in every app that draws the document.
fn fold_label(lines: usize) -> String {
    if lines == 1 {
        "1 quoted line".to_owned()
    } else {
        format!("{lines} quoted lines")
    }
}

/// The lines of words in a `<blockquote>` span, as its plain rendering
/// breaks them: at blocks and at `<br>`.
fn html_quote_lines(span: &str) -> usize {
    crate::parse(span)
        .to_text()
        .lines()
        .filter(|line| has_words(line))
        .count()
}

/// The lines of words in a run of `>` lines.
fn text_quote_lines(run: &str) -> usize {
    run.lines().filter(|line| has_words(line)).count()
}

/// Whether a quoted line says anything once its `>` markers are off: a
/// marker alone is the blank between two quoted paragraphs.
fn has_words(line: &str) -> bool {
    !line
        .trim_start_matches(|c: char| c == '>' || c.is_whitespace())
        .is_empty()
}

/// Render a `text/plain` body as HTML: escaped and `<pre>`-wrapped, with
/// contiguous runs of `>`-prefixed lines ([`text_stretches`]) folded into a
/// collapsed [`fold_html_quotes`]-style `<details>`.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 128);
    for stretch in text_stretches(text) {
        match stretch {
            Stretch::Own(own) => push_pre(&mut out, &own.lines().collect::<Vec<_>>()),
            Stretch::Quoted(quoted) => {
                open_fold(&mut out, text_quote_lines(quoted));
                push_pre(&mut out, &quoted.lines().collect::<Vec<_>>());
                out.push_str("</details>");
            }
        }
    }
    out
}

fn is_quote_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn push_pre(out: &mut String, lines: &[&str]) {
    out.push_str("<pre class=\"postio-body-text\">");
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        push_linkified(out, line);
    }
    out.push_str("</pre>");
}

/// Escape `line` into `out`, turning the URLs and bare email addresses in it
/// into anchors as it goes (#752).
///
/// **Order matters, and it is the whole security argument.** The anchor is
/// built here, out of a span this function matched, and every piece of it —
/// the `href` and the visible text alike — goes through [`escape_into`]. A
/// sender therefore cannot close the attribute or the tag: the only markup in
/// the output is markup this function wrote. Linkifying the *escaped* string
/// instead would be the tempting shape and the wrong one, because it would
/// mean matching against text in which `&amp;` has already become five
/// characters.
///
/// Nothing else linkifies: a `text/plain` body is not run through ammonia
/// (see `document::body_html`), so what this emits is what the reader shows.
fn push_linkified(out: &mut String, line: &str) {
    let mut cursor = 0;
    let mut at = 0;
    while at < line.len() {
        if !line.is_char_boundary(at) {
            at += 1;
            continue;
        }
        if starts_a_token(line, at)
            && let Some(link) = match_link(&line[at..])
        {
            escape_into(out, &line[cursor..at]);
            out.push_str("<a href=\"");
            escape_into(out, &link.href);
            out.push_str("\">");
            escape_into(out, link.text);
            out.push_str("</a>");
            at += link.text.len();
            cursor = at;
            continue;
        }
        at += 1;
    }
    escape_into(out, &line[cursor..]);
}

/// A span worth linking: what to show, and where it points.
struct Link<'a> {
    text: &'a str,
    href: String,
}

/// Whether `at` begins a word — so `example.com` inside `notexample.com` is
/// not matched, and neither is the `ada@` of a URL's userinfo.
fn starts_a_token(line: &str, at: usize) -> bool {
    let Some(before) = line[..at].chars().next_back() else {
        return true;
    };
    before.is_whitespace() || matches!(before, '(' | '[' | '{' | '<' | '"' | '\'' | ',' | ';')
}

/// Match a URL or a bare email address at the head of `rest`.
fn match_link(rest: &str) -> Option<Link<'_>> {
    let token = trim_trailing_punctuation(token_at(rest));
    if token.is_empty() {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        // A scheme and nothing after it is not a link.
        let after = token.find("//").map(|slash| slash + 2)?;
        if token[after..].is_empty() {
            return None;
        }
        return Some(Link {
            text: token,
            href: token.to_owned(),
        });
    }
    if let Some(address) = lower.strip_prefix("mailto:") {
        if address.is_empty() {
            return None;
        }
        return Some(Link {
            text: token,
            href: token.to_owned(),
        });
    }
    if lower.starts_with("www.") && lower.len() > 4 && looks_like_host(&lower[4..]) {
        // Written without a scheme, so one has to be chosen. `https` rather
        // than `http`: the reader never fetches either, and handing a
        // downgrade to the browser would be this application's choice, not
        // the sender's.
        return Some(Link {
            text: token,
            href: format!("https://{token}"),
        });
    }
    if let Some(address) = match_email(token) {
        return Some(Link {
            text: address,
            href: format!("mailto:{address}"),
        });
    }
    None
}

/// Everything up to the first character that cannot be inside a URL.
fn token_at(rest: &str) -> &str {
    let end = rest
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '"'))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Drop the punctuation a sentence put after a URL rather than inside it.
///
/// `(see https://example.com/x).` ends in `).`, and neither belongs to the
/// link — but `https://example.com/a_(b)` does end in a bracket that does, so
/// a closing one is only dropped when nothing opened it.
fn trim_trailing_punctuation(mut token: &str) -> &str {
    loop {
        let Some(last) = token.chars().next_back() else {
            return token;
        };
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => !token.contains('('),
            ']' => !token.contains('['),
            '}' => !token.contains('{'),
            _ => false,
        };
        if !drop {
            return token;
        }
        token = &token[..token.len() - last.len_utf8()];
    }
}

/// Whether `host` reads as a domain name: labels, dots, and a final alphabetic
/// label of at least two characters.
fn looks_like_host(host: &str) -> bool {
    let host = host.split('/').next().unwrap_or(host);
    let Some((_, tld)) = host.rsplit_once('.') else {
        return false;
    };
    tld.len() >= 2
        && tld.chars().all(|ch| ch.is_ascii_alphabetic())
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
}

/// A bare `local@domain`, or `None` if `token` is not one.
fn match_email(token: &str) -> Option<&str> {
    let (local, domain) = token.split_once('@')?;
    if local.is_empty() || domain.contains('@') {
        return None;
    }
    let local_ok = local
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-'));
    if !local_ok || !looks_like_host(&domain.to_ascii_lowercase()) || domain.contains('/') {
        return None;
    }
    Some(token)
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
            "<p>hi</p><details class=\"postio-quote\"><summary>1 quoted line</summary>\
             <blockquote><p>quoted</p></blockquote></details>"
        );
    }

    /// Spec 007 FR-034: a fold says how much it hides, "31 quoted lines",
    /// counting the lines that hold words. A `>` alone is not a line anyone
    /// quoted, and neither is the blank between two paragraphs.
    #[test]
    fn a_fold_says_how_many_quoted_lines_it_hides() {
        let text = text_to_html(
            "Sounds good.\n\n> On Monday you wrote:\n> the old words\n>\n> > and older ones\n\nThanks",
        );
        assert!(text.contains("<summary>3 quoted lines</summary>"), "{text}");
        let html = fold_html_quotes(
            "<p>hi</p><blockquote><p>one</p><p>two<br>three</p>\
             <blockquote><p>older</p></blockquote></blockquote>",
        );
        assert!(html.contains("<summary>4 quoted lines</summary>"), "{html}");
        let one = fold_html_quotes("<blockquote><p>just this</p></blockquote>");
        assert!(one.contains("<summary>1 quoted line</summary>"), "{one}");
    }

    #[test]
    fn nested_blockquotes_are_wrapped_once_as_a_unit() {
        // The Gmail/Apple Mail shape: `> >` becomes nested <blockquote>s.
        let html = "<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>";
        let out = fold_html_quotes(html);
        assert_eq!(
            out,
            format!(
                "<details class=\"postio-quote\"><summary>2 quoted lines</summary>{html}</details>"
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

    // ── #752: a URL in a plain-text body is a link ──────────────────────

    #[test]
    fn a_plain_text_url_becomes_an_anchor() {
        let out = text_to_html("See https://example.com/report for the figures.");
        assert!(
            out.contains("<a href=\"https://example.com/report\">https://example.com/report</a>"),
            "a bare URL should be clickable: {out}"
        );
        assert!(out.contains("See "), "the surrounding words survive: {out}");
    }

    #[test]
    fn a_scheme_less_host_gets_one_and_a_bare_address_gets_mailto() {
        let out = text_to_html("www.example.com and ada@example.com");
        assert!(
            out.contains("<a href=\"https://www.example.com\">www.example.com</a>"),
            "{out}"
        );
        assert!(
            out.contains("<a href=\"mailto:ada@example.com\">ada@example.com</a>"),
            "{out}"
        );
    }

    /// A sentence's punctuation is not part of the address it follows, but a
    /// bracket the URL opened for itself is.
    #[test]
    fn trailing_punctuation_stays_outside_the_link() {
        let out = text_to_html("(see https://example.com/a).");
        assert!(
            out.contains("<a href=\"https://example.com/a\">https://example.com/a</a>"),
            "{out}"
        );
        assert!(out.contains("</a>)."), "the `).` is text, not href: {out}");

        let balanced = text_to_html("https://example.com/a_(b)");
        assert!(
            balanced.contains("<a href=\"https://example.com/a_(b)\">"),
            "a bracket the URL opened belongs to it: {balanced}"
        );
    }

    /// The security property, stated as a test: the anchor is built out of a
    /// span this module matched and every part of it is escaped, so a sender
    /// cannot close the attribute or the tag. `text/plain` bodies are not run
    /// through ammonia, so nothing downstream would catch it if they could.
    #[test]
    fn a_url_cannot_inject_markup_through_the_anchor_it_becomes() {
        let out = text_to_html("https://example.com/\"><script>alert(1)</script>");
        assert!(
            !out.contains("<script>"),
            "markup in the source must not survive as markup: {out}"
        );
        assert!(out.contains("&lt;script&gt;"), "{out}");
        assert!(
            !out.contains("\"><script"),
            "the href attribute must not be closable: {out}"
        );
    }

    #[test]
    fn linkifying_leaves_the_quote_fold_alone() {
        let out = text_to_html("> https://example.com/quoted\n\nreply");
        assert!(out.starts_with("<details class=\"postio-quote\">"), "{out}");
        assert!(
            out.contains("<a href=\"https://example.com/quoted\">"),
            "a link inside a quoted run is still a link: {out}"
        );
    }

    /// Words that merely contain a dot, an `@` inside a longer token, or a
    /// scheme with nothing after it are not links — a false positive here is
    /// a clickable thing in the middle of somebody's prose.
    #[test]
    fn ordinary_prose_is_not_linkified() {
        for text in [
            "Sentences end. Like this one.",
            "the ratio was 3.5 or so",
            "read notexample.com/x",
            "https://",
        ] {
            let out = text_to_html(text);
            assert!(!out.contains("<a "), "{text:?} should not linkify: {out}");
        }
    }
}
