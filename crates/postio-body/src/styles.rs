//! A sender's `<style>` block, rewritten so it reaches only their own message.
//!
//! Inline `style` attributes (#1325) needed no scoping: a declaration applies
//! to the element it sits on, which is already inside the container drawn
//! around that message. A `<style>` block has *selectors*, and a selector
//! matches whatever is in the document — which under ADR 0032 is every other
//! sender's message and Postio's own chrome. Admitting one unscoped would let
//! message A restyle message B.
//!
//! So every rule is rewritten to sit under
//! [`crate::sanitize::message_selector`], and the declarations inside it go
//! through the same [`crate::sanitize::REFUSED`] table an inline attribute
//! does. Two rules, one table.
//!
//! **Rewriting, not CSS `@scope`.** Not because `@scope` is unavailable — it
//! was never measured — but because rewriting is one implementation in a
//! toolkit-free crate that both frontends inherit, provable in milliseconds
//! with no display. `@scope` would have to be verified separately against
//! WebKitGTK and WKWebView, and could not be tested without a display on
//! either.

use std::sync::atomic::AtomicU32;

use cssparser::{Delimiter, ParseError, Parser, ParserInput, Token};

use crate::sanitize::{RemoteImages, contain_declarations};

/// A sender's stylesheet after scoping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scoped {
    /// The rewritten CSS, ready to be emitted in Postio's own `<style>`.
    pub css: String,
    /// Remote references dropped, counted with the message's blocked images.
    pub remote_blocked: u32,
}

/// Rewrite `css` so nothing in it can match outside `prefix`.
pub fn scope(css: &str, prefix: &str, remote: RemoteImages) -> Scoped {
    let counter = AtomicU32::new(0);
    let css = scope_into(css, prefix, remote, &counter);
    Scoped {
        css,
        remote_blocked: counter.load(std::sync::atomic::Ordering::Relaxed),
    }
}

pub(crate) fn scope_into(
    css: &str,
    prefix: &str,
    remote: RemoteImages,
    blocked: &AtomicU32,
) -> String {
    let mut out = String::new();
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    write_rules(&mut parser, prefix, remote, blocked, &mut out);
    out
}

/// Every rule at one nesting level: the top of the sheet, or the inside of a
/// `@media`.
fn write_rules(
    parser: &mut Parser<'_, '_>,
    prefix: &str,
    remote: RemoteImages,
    blocked: &AtomicU32,
    out: &mut String,
) {
    loop {
        parser.skip_whitespace();
        if parser.is_exhausted() {
            return;
        }
        let state = parser.state();
        let at = match parser.next() {
            Ok(Token::AtKeyword(name)) => Some(name.as_ref().to_ascii_lowercase()),
            Ok(_) => None,
            // Exhausted, or a stray `}` closing a block we are not in. Either
            // way there is no next rule; stopping is what a browser does too.
            Err(_) => return,
        };
        match at {
            Some(name) => write_at_rule(parser, &name, prefix, remote, blocked, out),
            None => {
                parser.reset(&state);
                if !write_qualified_rule(parser, Some(prefix), remote, blocked, out) {
                    return;
                }
            }
        }
    }
}

/// One `selector { declarations }`.
///
/// `prefix` is `None` inside `@keyframes`, where the prelude is a percentage
/// rather than a selector and has nothing to reach with.
///
/// Returns whether the sheet can continue: a rule with no block at all is the
/// end of the input, and looping on it would not terminate.
fn write_qualified_rule(
    parser: &mut Parser<'_, '_>,
    prefix: Option<&str>,
    remote: RemoteImages,
    blocked: &AtomicU32,
    out: &mut String,
) -> bool {
    let start = parser.position();
    let _ = parser.parse_until_before(Delimiter::CurlyBracketBlock, consume_all);
    let prelude = parser.slice_from(start).trim().to_owned();
    if !matches!(parser.next(), Ok(Token::CurlyBracketBlock)) {
        // Unterminated. The prelude is dropped rather than guessed at: a
        // selector with no declarations styles nothing anyway.
        return false;
    }
    let body = parser
        .parse_nested_block(slice_of_block)
        .unwrap_or_default();

    let declarations = contain_declarations(&body, remote, blocked);
    if declarations.is_empty() {
        return true;
    }
    let selectors = match prefix {
        Some(prefix) => scope_selectors(&prelude, prefix),
        None => prelude,
    };
    if selectors.is_empty() {
        return true;
    }
    out.push_str(&selectors);
    out.push_str(" { ");
    out.push_str(&declarations);
    out.push_str(" }\n");
    true
}

fn write_at_rule(
    parser: &mut Parser<'_, '_>,
    name: &str,
    prefix: &str,
    remote: RemoteImages,
    blocked: &AtomicU32,
    out: &mut String,
) {
    let start = parser.position();
    let _ = parser.parse_until_before(
        Delimiter::Semicolon | Delimiter::CurlyBracketBlock,
        consume_all,
    );
    let prelude = parser.slice_from(start).trim().to_owned();
    let has_block = matches!(parser.next(), Ok(Token::CurlyBracketBlock));

    // A refused at-rule is still *consumed* — its block included — so the
    // rules written after it are read as rules rather than as its contents.
    // Anything not named here is refused by omission, which is the safe
    // direction: a CSS feature Postio has never heard of is not one it can
    // reason about the reach of.
    let kept = match name {
        "media" | "supports" | "container" | "layer" => Nested::Rules,
        "keyframes" | "-webkit-keyframes" => Nested::Keyframes,
        _ => Nested::Refused,
    };
    if kept == Nested::Refused {
        if has_block {
            let _ = parser.parse_nested_block(slice_of_block);
        }
        return;
    }
    if !has_block {
        // `@media screen;` is not a thing. Nothing to keep.
        return;
    }

    let mut inner = String::new();
    let _ = parser.parse_nested_block(|nested| {
        match kept {
            Nested::Rules => write_rules(nested, prefix, remote, blocked, &mut inner),
            Nested::Keyframes => {
                while write_qualified_rule(nested, None, remote, blocked, &mut inner) {
                    nested.skip_whitespace();
                    if nested.is_exhausted() {
                        break;
                    }
                }
            }
            Nested::Refused => unreachable!("refused above"),
        }
        Ok::<(), ParseError<'_, ()>>(())
    });
    if inner.trim().is_empty() {
        return;
    }
    out.push('@');
    out.push_str(name);
    if !prelude.is_empty() {
        out.push(' ');
        out.push_str(&prelude);
    }
    out.push_str(" {\n");
    out.push_str(&inner);
    out.push_str("}\n");
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Nested {
    /// Ordinary rules, each scoped as if it were at the top of the sheet.
    Rules,
    /// Keyframe selectors: `from`, `to`, `50%`. Not element selectors, so
    /// prefixing them would destroy them and scoping them buys nothing.
    Keyframes,
    Refused,
}

/// Rewrite a selector list so every member sits under `prefix`.
///
/// Split on **top-level** commas only. `:is(h1, h2)` is one selector, and
/// cutting it at its comma emits `<prefix> :is(h1` — well-formed enough to
/// parse as nothing at all, so the rule would vanish with no sign of it.
/// `parse_until_before` knows where the function ends; a `split(',')` does
/// not.
fn scope_selectors(prelude: &str, prefix: &str) -> String {
    let mut input = ParserInput::new(prelude);
    let mut parser = Parser::new(&mut input);
    let mut parts: Vec<String> = Vec::new();
    loop {
        parser.skip_whitespace();
        let start = parser.position();
        let _ = parser.parse_until_before(Delimiter::Comma, consume_all);
        let part = parser.slice_from(start).trim();
        if !part.is_empty() {
            parts.push(scope_one_selector(part, prefix));
        }
        if parser.next().is_err() {
            break;
        }
    }
    parts.join(", ")
}

/// The document a sender means is their own message.
///
/// `body { font-family: … }` is how a great deal of real mail sets its type,
/// and it means "this email", because when it was written the email *was* the
/// document. Prefixing it into `<prefix> body` would be literal and useless:
/// a well-formed selector matching nothing, which reads to the reader as
/// Postio dropping the sender's styling rather than containing it. So the
/// document heads resolve to the message's own container instead.
fn scope_one_selector(selector: &str, prefix: &str) -> String {
    let mut rest = selector;
    let mut anchored = false;
    while let Some(stripped) = strip_document_head(rest) {
        anchored = true;
        rest = stripped;
    }
    match (anchored, rest.is_empty()) {
        (true, true) => prefix.to_owned(),
        (true, false) => format!("{prefix} {rest}"),
        (false, _) => format!("{prefix} {selector}"),
    }
}

/// `selector` with a leading `html`, `body` or `:root` removed, if it has one.
///
/// Only at a token boundary, so `bodycopy` and `body-text` are ordinary
/// selectors and keep their meaning.
fn strip_document_head(selector: &str) -> Option<&str> {
    for head in ["html", "body", ":root"] {
        let Some(rest) = selector
            .get(..head.len())
            .filter(|start| start.eq_ignore_ascii_case(head))
            .map(|_| &selector[head.len()..])
        else {
            continue;
        };
        let boundary = rest
            .chars()
            .next()
            .is_none_or(|next| next.is_whitespace() || next == '>' || next == '+' || next == '~');
        if boundary {
            return Some(rest.trim_start());
        }
    }
    None
}

/// Read a delimited parser to its end and answer with the source text of it.
fn slice_of_block<'i>(parser: &mut Parser<'i, '_>) -> Result<String, ParseError<'i, ()>> {
    let start = parser.position();
    while parser.next().is_ok() {}
    Ok(parser.slice_from(start).to_owned())
}

/// Read a delimited parser to its end, for the source text `slice_from` will
/// then be asked for.
fn consume_all<'i>(parser: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, ()>> {
    while parser.next().is_ok() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = r#".postio-body[data-postio-message="2"]"#;

    fn scoped(css: &str) -> String {
        scope(css, PREFIX, RemoteImages::Blocked).css
    }

    #[test]
    fn a_rule_reaches_only_inside_its_own_message() {
        let css = scoped("p { color: red }");
        assert!(
            css.contains(&format!("{PREFIX} p")),
            "the rule must be rewritten under the message's own container, got {css:?}"
        );
        assert!(
            css.contains("color: red"),
            "the declaration survives: {css:?}"
        );
    }

    #[test]
    fn every_member_of_a_selector_list_is_scoped() {
        let css = scoped("h1, h2 { margin: 0 }");
        assert!(css.contains(&format!("{PREFIX} h1")), "{css:?}");
        assert!(
            css.contains(&format!("{PREFIX} h2")),
            "an unscoped second member reaches the whole conversation: {css:?}"
        );
    }

    #[test]
    fn a_comma_inside_a_functional_selector_is_not_a_list_boundary() {
        // `:is(h1, h2)` is one selector. Splitting the prelude on every comma
        // would cut it in half and emit `<prefix> :is(h1` -- which parses as
        // nothing and silently loses the rule.
        let css = scoped(":is(h1, h2) span { color: red }");
        assert_eq!(
            css.matches(PREFIX).count(),
            1,
            "the prelude is one selector, not two: {css:?}"
        );
        assert!(css.contains(":is(h1, h2) span"), "{css:?}");
    }

    #[test]
    fn styling_the_document_styles_the_message() {
        // Senders write `body { ... }` meaning "my email". Prefixing it into
        // `<prefix> body` would match nothing at all, which reads to the user
        // as Postio failing to render what they were sent.
        for selector in ["body", "html", ":root"] {
            let css = scoped(&format!("{selector} {{ background: #fff }}"));
            assert!(
                css.contains(&format!("{PREFIX} {{")),
                "{selector} must become the message's own container, got {css:?}"
            );
        }
    }

    #[test]
    fn an_import_loads_nothing_and_the_rest_of_the_sheet_survives() {
        // `@import` fetches when the stylesheet parses, carrying the referer
        // and the reader's IP. It needs no `<img>`, so neither the declaration
        // table nor `img-src` touches it.
        let css = scoped("@import url(https://tracker.example/x.css);\np { color: red }");
        assert!(!css.contains("@import"), "{css:?}");
        assert!(!css.contains("tracker.example"), "{css:?}");
        assert!(
            css.contains("color: red"),
            "one refusal must not discard the sheet: {css:?}"
        );
    }

    #[test]
    fn a_media_query_survives_and_what_is_inside_it_is_scoped() {
        let css = scoped("@media (min-width: 600px) { p { color: red } }");
        assert!(css.contains("@media (min-width: 600px)"), "{css:?}");
        assert!(
            css.contains(&format!("{PREFIX} p")),
            "a rule nested in an at-rule is still a rule: {css:?}"
        );
    }

    #[test]
    fn keyframes_survive_intact() {
        // A keyframe's "selectors" are percentages, not element selectors:
        // there is nothing to scope, and prefixing them would destroy them.
        let css = scoped("@keyframes fade { from { opacity: 0 } to { opacity: 1 } }");
        assert!(css.contains("@keyframes fade"), "{css:?}");
        assert!(css.contains("opacity: 0"), "{css:?}");
        assert!(!css.contains(&format!("{PREFIX} from")), "{css:?}");
    }

    #[test]
    fn a_font_face_is_refused() {
        let css = scoped("@font-face { font-family: X; src: url(https://f.example/x.woff2) }");
        assert!(!css.contains("@font-face"), "{css:?}");
        assert!(!css.contains("f.example"), "{css:?}");
    }

    #[test]
    fn a_refused_property_does_not_take_its_neighbours_with_it() {
        let css = scoped("p { position: fixed; color: red }");
        assert!(!css.contains("position"), "{css:?}");
        assert!(
            css.contains("color: red"),
            "refusing `position` is not licence to discard the colour: {css:?}"
        );
    }

    #[test]
    fn a_remote_background_in_a_rule_is_blocked_and_counted() {
        let scoped = scope(
            "p { background-image: url(https://tracker.example/p.gif) }",
            PREFIX,
            RemoteImages::Blocked,
        );
        assert!(!scoped.css.contains("tracker.example"), "{:?}", scoped.css);
        assert_eq!(
            scoped.remote_blocked, 1,
            "a background is a remote image and the panel counts it"
        );
    }

    #[test]
    fn a_remote_background_stays_for_an_allowed_sender() {
        let scoped = scope(
            "p { background-image: url(https://known.example/p.gif) }",
            PREFIX,
            RemoteImages::Allowed,
        );
        assert!(scoped.css.contains("known.example"), "{:?}", scoped.css);
    }

    #[test]
    fn postio_s_own_chrome_cannot_be_named() {
        // The message head, the blocked-images notice and the per-message
        // actions all sit outside `.postio-body`. A sender naming them gets a
        // selector that is well-formed and matches nothing, which is the
        // point: a message must not be able to hide the notice that says its
        // images were blocked.
        let css = scoped(".postio-blocked { display: none }");
        assert!(
            css.contains(&format!("{PREFIX} .postio-blocked")),
            "{css:?}"
        );
        assert!(
            !css.contains("}.postio-blocked") && !css.starts_with(".postio-blocked"),
            "an unprefixed rule would reach the notice: {css:?}"
        );
    }

    #[test]
    fn an_unknown_at_rule_is_refused_rather_than_passed_through() {
        let css = scoped("@totally-new { p { color: red } }\nh1 { color: blue }");
        assert!(!css.contains("@totally-new"), "{css:?}");
        assert!(
            css.contains("color: blue"),
            "an unknown at-rule must not swallow the rest of the sheet: {css:?}"
        );
    }

    #[test]
    fn an_unterminated_rule_does_not_swallow_the_sheet() {
        let css = scoped("p { color: red");
        // Either the rule is recovered or it is dropped; what must not happen
        // is a panic or an unscoped fragment reaching the document.
        assert!(!css.contains("<"), "{css:?}");
    }

    #[test]
    fn every_refused_at_rule_is_actually_refused() {
        // The table is the specification of the refusal, so it has to be the
        // thing that fails when the two disagree -- otherwise it is a comment
        // that happens to compile.
        assert!(!crate::sanitize::REFUSED_AT_RULES.is_empty());
        for (name, _reason) in crate::sanitize::REFUSED_AT_RULES {
            let with_block = scoped(&format!("@{name} x {{ p {{ color: red }} }}"));
            assert!(
                !with_block.contains(&format!("@{name}")),
                "@{name} is in the refusal table and survived: {with_block:?}"
            );
            let with_semicolon = scoped(&format!("@{name} x;\nh1 {{ color: blue }}"));
            assert!(
                !with_semicolon.contains(&format!("@{name}")),
                "@{name} survived in its statement form: {with_semicolon:?}"
            );
            assert!(
                with_semicolon.contains("color: blue"),
                "refusing @{name} must not swallow the rule after it: {with_semicolon:?}"
            );
        }
    }

    #[test]
    fn a_sheet_with_nothing_left_emits_nothing() {
        assert_eq!(scoped("@import url(https://x.example/a.css);"), "");
    }
}
