//! Quoted-text folding: quoted spans become closed `<details>`, which needs no script.
//! `fold_html_quotes` wraps top-level `<blockquote>`s in sanitized HTML; `text_to_html` renders
//! `text/plain` with `>` runs folded. Neither removes a byte of content.
//! `html_stretches` and `text_stretches` are the one quote detector; `own_text` leaves out exactly
//! what the reader folds.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stretch<'a> {
    Own(&'a str),
    Quoted(&'a str),
}

/// Only the outermost span of a nested chain is a stretch; every byte is in exactly one stretch, in
/// order.
/// Tags are matched by literal spelling, safe only because `ammonia`'s serializer never emits
/// `<blockquote` inside an attribute value.
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

    // An unmatched `<blockquote` must not eat the rest of the message: it stays the sender's own.
    if own_start < html.len() {
        stretches.push(Stretch::Own(&html[own_start..]));
    }
    stretches
}

/// Each stretch is whole lines, line endings included.
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

/// Only the outermost span is wrapped, so expanding reveals every ancestor; other text is copied
/// byte for byte.
/// The block introducing the quote gets `ATTRIBUTION_CLASS`, the one change to the sender's markup:
/// a class, never text.
pub fn fold_html_quotes(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 96);
    let stretches = html_stretches(html);
    for (n, stretch) in stretches.iter().enumerate() {
        match *stretch {
            Stretch::Own(own) => {
                let introduces = matches!(stretches.get(n + 1), Some(Stretch::Quoted(_)));
                match introduces.then(|| mark_html_attribution(own)).flatten() {
                    Some(marked) => out.push_str(&marked),
                    None => out.push_str(own),
                }
            }
            Stretch::Quoted(quoted) => wrap_quote(&mut out, quoted),
        }
    }
    out
}

pub const ATTRIBUTION_CLASS: &str = "postio-attribution";

/// `own` without its last line when that line introduces the quote; whole when it introduces
/// nothing.
pub fn without_attribution(own: &str) -> &str {
    let words = own.trim_end();
    let start = words.rfind('\n').map_or(0, |at| at + 1);
    if introduces(&words[start..]) {
        &own[..start]
    } else {
        own
    }
}

/// Ends in a colon ("wrote:", "a écrit :", "schrieb:") and has words before it.
fn introduces(text: &str) -> bool {
    let text = text.trim();
    text.ends_with(':') && text.chars().any(char::is_alphanumeric)
}

/// `None` when no block holds the introducing line, or words follow the last block's close
/// (`<div>Thanks,</div>On Mon, Ana wrote:`).
fn mark_html_attribution(own: &str) -> Option<String> {
    let (start, tag_end) = introducing_block(own)?;
    let block = &own[start..];
    let tag = &block[..tag_end];
    let marked = match tag.find(" class=\"") {
        Some(at) => {
            let at = at + " class=\"".len();
            format!("{}{ATTRIBUTION_CLASS} {}", &tag[..at], &tag[at..])
        }
        None => {
            let name = if tag.starts_with("<div") { 4 } else { 2 };
            format!(
                "{} class=\"{ATTRIBUTION_CLASS}\"{}",
                &tag[..name],
                &tag[name..]
            )
        }
    };
    Some(format!("{}{marked}{}", &own[..start], &block[tag_end..]))
}

fn introducing_block(own: &str) -> Option<(usize, usize)> {
    let start = ["<div", "<p"]
        .iter()
        .filter_map(|open| {
            own.match_indices(open)
                .map(|(at, _)| at)
                .filter(|at| tag_name_ends_at(&own[*at..], open.len()))
                .last()
        })
        .max()?;
    let block = &own[start..];
    let tag_end = block.find('>')?;
    let close = ["</div>", "</p>"]
        .iter()
        .filter_map(|close| block.find(close))
        .min()?;
    if close < tag_end || !only_markup(&block[close..]) {
        return None;
    }
    if !introduces(&strip_tags(&block[tag_end + 1..close])) {
        return None;
    }
    Some((start, tag_end))
}

/// The HTML twin of `without_attribution`.
pub fn html_without_attribution(own: &str) -> &str {
    match introducing_block(own) {
        Some((start, _)) => &own[..start],
        None => own,
    }
}

fn only_markup(html: &str) -> bool {
    let mut rest = html;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if !rest.starts_with('<') {
            return false;
        }
        match rest.find('>') {
            Some(end) => rest = &rest[end + 1..],
            None => return false,
        }
    }
}

fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => text.push(ch),
            _ => {}
        }
    }
    text
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

fn open_fold(out: &mut String, lines: usize) {
    out.push_str("<details class=\"postio-quote\"><summary>");
    out.push_str(&fold_label(lines));
    out.push_str("</summary>");
}

/// One label for a single message and a conversation, in every app that draws a fold.
pub fn fold_label(lines: usize) -> String {
    if lines == 1 {
        "1 quoted line".to_owned()
    } else {
        format!("{lines} quoted lines")
    }
}

fn html_quote_lines(span: &str) -> usize {
    crate::parse(span)
        .to_text()
        .lines()
        .filter(|line| has_words(line))
        .count()
}

fn text_quote_lines(run: &str) -> usize {
    run.lines().filter(|line| has_words(line)).count()
}

fn has_words(line: &str) -> bool {
    !line
        .trim_start_matches(|c: char| c == '>' || c.is_whitespace())
        .is_empty()
}

/// Every word stays; a run of two or more bulleted or numbered lines is drawn as a list, and the
/// line introducing a fold is its own block marked `ATTRIBUTION_CLASS`.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 128);
    let stretches = text_stretches(text);
    for (n, stretch) in stretches.iter().enumerate() {
        match *stretch {
            Stretch::Own(own) => {
                let mut paragraphs = paragraphs(own);
                let attribution = match stretches.get(n + 1) {
                    Some(Stretch::Quoted(_)) => take_attribution(&mut paragraphs),
                    _ => None,
                };
                for paragraph in paragraphs {
                    push_blocks(&mut out, &paragraph);
                }
                if let Some(line) = attribution {
                    out.push_str("<p class=\"");
                    out.push_str(ATTRIBUTION_CLASS);
                    out.push_str("\">");
                    push_linkified(&mut out, line.trim());
                    out.push_str("</p>");
                }
            }
            Stretch::Quoted(quoted) => {
                open_fold(&mut out, text_quote_lines(quoted));
                push_pre(&mut out, &quoted.lines().collect::<Vec<_>>());
                out.push_str("</details>");
            }
        }
    }
    out
}

/// A paragraph break is the stylesheet's gap, not an empty line, which read as a line and a half.
fn paragraphs(text: &str) -> Vec<Vec<&str>> {
    let mut paragraphs: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

fn take_attribution<'a>(paragraphs: &mut Vec<Vec<&'a str>>) -> Option<&'a str> {
    let last = paragraphs.last_mut()?;
    if !introduces(last.last()?) {
        return None;
    }
    let line = last.pop();
    if last.is_empty() {
        paragraphs.pop();
    }
    line
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    Bullet(char),
    Number(u32),
}

/// A dash bullets only when a space follows, so `-- `, `-5` and hyphenated words are not items.
fn list_item(line: &str) -> Option<(List, usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    let (kind, after) = if matches!(first, '-' | '*' | '•') {
        (List::Bullet(first), first.len_utf8())
    } else {
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        if digits == 0 || digits > 3 || !matches!(rest[digits..].chars().next(), Some('.' | ')')) {
            return None;
        }
        (List::Number(rest[..digits].parse().ok()?), digits + 1)
    };
    let words = &rest[after..];
    if !words.starts_with([' ', '\t']) {
        return None;
    }
    let words = words.trim();
    (!words.is_empty()).then_some((kind, indent, words))
}

fn continues(kind: List, next: List) -> bool {
    match (kind, next) {
        (List::Bullet(a), List::Bullet(b)) => a == b,
        (List::Number(a), List::Number(b)) => b == a + 1,
        _ => false,
    }
}

fn list_at<'a>(lines: &[&'a str]) -> Option<(List, Vec<Vec<&'a str>>, usize)> {
    let (first, mut indent, words) = list_item(lines.first()?)?;
    let mut items = vec![vec![words]];
    let mut last = first;
    let mut taken = 1;
    for line in &lines[1..] {
        if let Some((kind, at, words)) = list_item(line) {
            if !continues(last, kind) {
                break;
            }
            items.push(vec![words]);
            (last, indent) = (kind, at);
        } else if line.len() - line.trim_start().len() > indent {
            items.last_mut()?.push(line.trim());
        } else {
            break;
        }
        taken += 1;
    }
    (items.len() >= 2).then_some((first, items, taken))
}

fn push_blocks(out: &mut String, lines: &[&str]) {
    let mut text_from = 0;
    let mut at = 0;
    while at < lines.len() {
        let Some((kind, items, taken)) = list_at(&lines[at..]) else {
            at += 1;
            continue;
        };
        if text_from < at {
            push_pre(out, &lines[text_from..at]);
        }
        push_list(out, kind, &items);
        at += taken;
        text_from = at;
    }
    if text_from < lines.len() {
        push_pre(out, &lines[text_from..]);
    }
}

fn push_list(out: &mut String, kind: List, items: &[Vec<&str>]) {
    let tag = match kind {
        List::Bullet(_) => "ul",
        List::Number(_) => "ol",
    };
    out.push('<');
    out.push_str(tag);
    out.push_str(" class=\"postio-body-list\"");
    if let List::Number(start) = kind
        && start != 1
    {
        out.push_str(&format!(" start=\"{start}\""));
    }
    out.push('>');
    for item in items {
        out.push_str("<li>");
        for (n, line) in item.iter().enumerate() {
            if n > 0 {
                out.push(' ');
            }
            push_linkified(out, line);
        }
        out.push_str("</li>");
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
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

/// The anchor is built from a span this function matched, and its `href` and text both go through
/// `escape_into`, so a sender cannot close the attribute or tag. Linkifying the escaped string
/// would match against expanded `&amp;` (#752).
/// Nothing else linkifies: `text/plain` bodies skip ammonia.
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

struct Link<'a> {
    text: &'a str,
    href: String,
}

fn starts_a_token(line: &str, at: usize) -> bool {
    let Some(before) = line[..at].chars().next_back() else {
        return true;
    };
    before.is_whitespace() || matches!(before, '(' | '[' | '{' | '<' | '"' | '\'' | ',' | ';')
}

fn match_link(rest: &str) -> Option<Link<'_>> {
    let token = trim_trailing_punctuation(token_at(rest));
    if token.is_empty() {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
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
        // `https`, not `http`: a downgrade handed to the browser would be this app's choice, not
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

fn token_at(rest: &str) -> &str {
    let end = rest
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '"'))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// A closing bracket is dropped only when nothing opened it: `(see https://example.com/x).` loses
/// `).`, `…/a_(b)` keeps its `)`.
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
        let html = "<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>";
        let out = fold_html_quotes(html);
        assert_eq!(
            out,
            format!(
                "<details class=\"postio-quote\"><summary>2 quoted lines</summary>{html}</details>"
            )
        );
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
        let text = "On 2026-02-10, Ada Norwood wrote:\n> Short note <b>before</b>\n> to cover\n\nThat order works.";
        let out = text_to_html(text);
        assert!(out.contains("On 2026-02-10, Ada Norwood wrote:"));
        assert!(
            out.contains("<details class=\"postio-quote\">"),
            "the `>` run should fold: {out}"
        );
        assert!(out.contains("&lt;b&gt;before&lt;/b&gt;"), "{out}");
        assert!(out.contains("That order works."));
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
    fn a_blank_line_parts_paragraphs_rather_than_drawing_an_empty_line() {
        let out = text_to_html("Hi all,\nline two\n\n\n  \nSecond paragraph.\n");
        assert_eq!(out.matches("<pre").count(), 2, "{out}");
        assert!(
            out.contains("<pre class=\"postio-body-text\">Hi all,\nline two</pre>"),
            "{out}"
        );
        assert!(
            out.contains("<pre class=\"postio-body-text\">Second paragraph.</pre>"),
            "{out}"
        );
    }

    #[test]
    fn a_message_that_is_entirely_quoted_still_folds() {
        let out = text_to_html("> all quoted\n> every line");
        assert!(out.starts_with("<details class=\"postio-quote\">"));
    }

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

    /// The anchor is built from a matched span with every part escaped; `text/plain` skips ammonia,
    /// so nothing downstream would catch an injection.
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

    /// A false positive is a clickable thing in the middle of somebody's prose.
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

    #[test]
    fn the_line_before_a_plain_quote_is_marked_as_its_attribution() {
        for text in [
            "Thanks,\nLena\n\nOn Mon, 21 Sep 2026, Ben <ben@example.org> wrote:\n\n> Looks good.\n",
            "Thanks,\nLena\nOn Mon, 21 Sep 2026, Ben <ben@example.org> wrote:\n> Looks good.\n",
        ] {
            let out = text_to_html(text);
            assert!(
                out.contains("<pre class=\"postio-body-text\">Thanks,\nLena</pre>"),
                "the sign-off keeps its own block: {out}"
            );
            let marked = out
                .find("<p class=\"postio-attribution\">On Mon, 21 Sep 2026, Ben &lt;")
                .unwrap_or_else(|| panic!("no attribution: {out}"));
            let fold = out.find("<details").expect("a fold");
            assert!(marked < fold, "{out}");
            assert!(
                out[marked..fold].ends_with("wrote:</p>"),
                "the attribution is the last block before the fold: {out}"
            );
            assert!(
                out[marked..fold].contains("href=\"mailto:ben@example.org\""),
                "linkified like any line: {out}"
            );
        }
    }

    #[test]
    fn the_own_words_above_a_quote_lose_only_its_attribution() {
        assert_eq!(
            without_attribution("Done.\n\nOn Fri 9 Oct 2026 at 10:59, You wrote:\n"),
            "Done.\n\n"
        );
        assert_eq!(
            without_attribution("On 2026-02-10, Ada Norwood wrote:\n"),
            "",
            "an attribution alone is all of it"
        );
        assert_eq!(
            without_attribution("I agree with this part.\n"),
            "I agree with this part.\n",
            "a sentence that introduces nothing stays"
        );
    }

    #[test]
    fn a_line_that_does_not_introduce_the_quote_is_not_marked() {
        let out = text_to_html("I agree with this part.\n> The quoted part.\n");
        assert!(!out.contains("postio-attribution"), "{out}");
        let out = text_to_html("Hi\n\nNothing quoted here, wrote:\n");
        assert!(!out.contains("postio-attribution"), "{out}");
    }

    #[test]
    fn the_block_before_an_html_quote_is_marked_when_it_introduces_it() {
        let gmail = "<div>Thanks,</div><div>Lena</div><div class=\"x-gmail_quote\">\
            <div dir=\"ltr\" class=\"x-gmail_attr\">On Mon, Ben &lt;<a href=\"mailto:ben@example.org\">\
            ben@example.org</a>&gt; wrote:<br></div><blockquote><div>Looks good.</div></blockquote></div>";
        let out = fold_html_quotes(gmail);
        assert!(
            out.contains("<div dir=\"ltr\" class=\"postio-attribution x-gmail_attr\">On Mon"),
            "{out}"
        );
        let plain =
            fold_html_quotes("<p>Thanks</p><p>On Mon, Ben wrote:</p><blockquote>q</blockquote>");
        assert!(
            plain.contains("<p class=\"postio-attribution\">On Mon, Ben wrote:</p><details"),
            "{plain}"
        );
        for html in [
            "<p>I agree.</p><blockquote>q</blockquote>",
            "<div>Thanks,</div>On Mon, Ben wrote:<blockquote>q</blockquote>",
        ] {
            let out = fold_html_quotes(html);
            assert!(!out.contains("postio-attribution"), "{out}");
        }
    }

    #[test]
    fn a_run_of_bulleted_lines_is_a_list() {
        let out = text_to_html(
            "The differences:\n\n- Cursor pagination.\n- Rate-limit headers.\n   - See https://example.com/x\n\nThanks",
        );
        assert!(
            out.contains(
                "<ul class=\"postio-body-list\"><li>Cursor pagination.</li>\
                 <li>Rate-limit headers.</li><li>See <a href=\"https://example.com/x\">"
            ),
            "{out}"
        );
        assert!(!out.contains("- Cursor"), "{out}");
        let together = text_to_html("Two things:\n* one\n* two\n");
        assert!(
            together.contains(
                "<pre class=\"postio-body-text\">Two things:</pre><ul class=\"postio-body-list\">\
                 <li>one</li><li>two</li></ul>"
            ),
            "{together}"
        );
        let numbered = text_to_html("1. first\n2. second\n");
        assert!(
            numbered.contains("<ol class=\"postio-body-list\"><li>first</li><li>second</li></ol>"),
            "{numbered}"
        );
    }

    #[test]
    fn a_list_item_keeps_its_wrapped_lines_and_escapes_its_words() {
        let out = text_to_html("- a long item that\n  wraps onto a second line\n- <b>bold</b>\n");
        assert!(
            out.contains(
                "<li>a long item that wraps onto a second line</li><li>&lt;b&gt;bold&lt;/b&gt;</li>"
            ),
            "{out}"
        );
    }

    #[test]
    fn what_only_looks_like_a_list_stays_text() {
        for text in [
            "- just one\n",
            "Lena\n-- \nsent from a phone\n",
            "-5 degrees\n-3 tomorrow\n",
            "3. third\n1. first\n",
        ] {
            let out = text_to_html(text);
            assert!(
                !out.contains("<ul") && !out.contains("<ol"),
                "{text:?} became a list: {out}"
            );
        }
    }
}
