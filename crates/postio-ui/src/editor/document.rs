//! The document the composer edits in.
//!
//! The editing surface's answer to [`crate::reader::document`], and built the
//! same way for the same reason: toolkit-free, so both frontends inherit one
//! answer instead of each writing their own and drifting.
//!
//! Everything visual comes from the reader's generated palette. A colour has
//! one source (#296), and the requirement here is not "some colours" but
//! *the same ones the reader uses for a message body* (FR-073) — a composer
//! that is nearly the reader is worse than one that is obviously not.

use std::fmt::Write as _;

/// A fixed, non-`http(s)` base, so edited content is never same-origin with
/// anything real — the reader's reasoning, applied to the other document.
pub const EDITOR_BASE_URI: &str = "postio-editor:///";

/// The content policy the editing shell carries.
///
/// Deliberately **not** shared with the reader's. The reader permits neither
/// script nor `contenteditable`; the editor requires both, and a shared
/// function with a flag would hide the one difference that matters inside a
/// parameter. Two documents, two policies, one set of tokens.
pub const EDITOR_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:";

/// The class the reader gives a foldable quote, and therefore the class the
/// editor gives one. Shared vocabulary is the requirement (FR-077); the rules
/// are restated in `editor.css` because `reader.css` carries a great deal this
/// surface must not have.
pub const QUOTE_CLASS: &str = "postio-quote";

/// The attribute `postio_body` marks a reply quote with.
///
/// Named here rather than imported so this crate does not depend on the body
/// crate for one string; the two are asserted equal in `postio-gtk`, which
/// sees both.
pub const QUOTED_MARKER: &str = "data-postio-quoted";

/// How the surface is drawn: the numbers a frontend owns and this module does
/// not guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    /// Whether the dark palette applies.
    pub dark: bool,
    /// Body text size in CSS pixels, from the application's own setting.
    pub text_size: u32,
    /// Vertical padding, which is what density means on a surface with one
    /// column and no rows.
    pub padding: u32,
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            dark: false,
            text_size: 15,
            padding: 12,
        }
    }
}

/// The whole document the editing web view loads.
///
/// `content` is the draft's body markup, already narrowed by
/// `postio_body::parse` — nothing here sanitises, and nothing here may be the
/// first thing that would have.
pub fn wrap_document(content: &str, presentation: Presentation) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{EDITOR_CSP}\">\n\
         <style>{}</style>\n\
         </head><body contenteditable=\"true\">{}</body></html>",
        editor_css(presentation),
        fold_quotes(content),
    )
}

/// The sheet, tokens and all.
///
/// The variable block is last so it wins: the tokens file defines the palette
/// and `editor.css` reads it, and the frontend's own numbers are applied over
/// both.
pub fn editor_css(presentation: Presentation) -> String {
    let mut css = String::new();
    css.push_str(include_str!("../../data/reader-tokens.css"));
    css.push_str(include_str!("../../data/editor.css"));
    let _ = write!(
        css,
        "\n:root {{\n  --e-text-size: {}px;\n  --e-pad: {}px;\n}}\n",
        presentation.text_size, presentation.padding
    );
    if presentation.dark {
        // The palette switches on `prefers-color-scheme`, which a web view
        // resolves from its own settings rather than from ours. Restating the
        // dark block under an explicit selector is what makes the application's
        // choice win -- and is why a scheme change is a new sheet rather than a
        // new document (FR-075): reloading would lose the caret and the undo
        // history.
        css.push_str(&dark_tokens_restated());
    }
    css
}

/// The ground colour for `presentation`, as the generated palette spells it.
///
/// A frontend needs this outside the document as well as inside it: the
/// document paints `--r-ground` on `body`, but only once it has *parsed*, and
/// a web view between one document and the next has nothing to paint from.
/// That interval is the white flash. The reader learned this the same way.
pub fn editor_ground(dark: bool) -> &'static str {
    crate::reader::document::reader_ground(dark)
}

/// The dark half of the palette, restated so it applies without the engine
/// agreeing about the scheme.
fn dark_tokens_restated() -> String {
    const PALETTE: &str = include_str!("../../data/reader-tokens.css");
    const DARK_BLOCK: &str = "@media (prefers-color-scheme: dark)";

    let (_, dark) = PALETTE
        .split_once(DARK_BLOCK)
        .expect("the generated palette always emits a dark block");
    let body = dark
        .split_once('{')
        .and_then(|(_, rest)| rest.rsplit_once('}'))
        .map(|(inside, _)| inside)
        .unwrap_or_default();
    // The media block wraps one `:root { ... }`; lifting its body out and
    // restating it under `:root` is the whole trick.
    let inner = body
        .split_once('{')
        .and_then(|(_, rest)| rest.rsplit_once('}'))
        .map(|(declarations, _)| declarations)
        .unwrap_or(body);
    format!("\n:root {{{inner}}}\n")
}

/// Wraps each reply quote in the reader's foldable shape.
///
/// A presentation concern and nothing else, which is why it happens here and
/// not in `postio_body`'s rendering: `<details>` must never reach the wire.
/// Mail clients disagree about it wildly, and a recipient whose client ignores
/// it would get the summary line as stray text above the quote. What is sent
/// is a plain `<blockquote>`; what is *edited* is that blockquote inside a
/// `<details>` the parser walks straight through on the way back.
///
/// Closed by default — no `open` attribute — so a reply opens with the caret
/// above a folded quote rather than below a screenful of someone else's mail
/// (FR-041). Opening it is a click or a keypress on the summary, with no
/// script involved, exactly as in the reader.
pub fn fold_quotes(content: &str) -> String {
    let needle = format!("<blockquote {QUOTED_MARKER}");
    let mut out = String::with_capacity(content.len() + 96);
    let mut rest = content;
    while let Some(at) = rest.find(&needle) {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find("</blockquote>") else {
            break;
        };
        let end = at + end + "</blockquote>".len();
        let _ = write!(
            out,
            "<details class=\"{QUOTE_CLASS}\"><summary contenteditable=\"false\">\
             Quoted message</summary>{}</details>",
            &rest[at..end]
        );
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_carries_a_stylesheet_and_the_editing_surface() {
        // FR-073. The surface used to be `<body contenteditable="true">` with
        // a CSP and nothing else, so it drew in the engine's defaults: the
        // wrong typeface, the wrong size, and a white page in dark mode.
        let document = wrap_document("<p>Morning.</p>", Presentation::default());

        assert!(document.contains("<style>"), "no stylesheet at all");
        assert!(
            document.contains("Barlow"),
            "the sheet does not name the application's typeface"
        );
        assert!(
            document.contains("--r-ground"),
            "the sheet does not use the reader's palette, so it can drift \
             from the message body it sits beside"
        );
        assert!(
            document.contains("contenteditable=\"true\""),
            "the surface stopped being editable"
        );
        assert!(document.contains("Morning."), "the content was dropped");
    }

    #[test]
    fn the_ground_resolves_in_both_schemes_and_they_differ() {
        // FR-074. The value a frontend paints on the view itself, for the
        // interval before the document has parsed -- the white flash.
        let light = editor_ground(false);
        let dark = editor_ground(true);
        assert!(light.starts_with('#'), "not a colour: {light:?}");
        assert!(dark.starts_with('#'), "not a colour: {dark:?}");
        assert_ne!(light, dark, "one ground for both schemes is one too few");
    }

    #[test]
    fn the_editor_ground_is_the_readers_ground() {
        // The contract that cannot be tested by comparing screenshots, so it
        // is tested by construction instead: both come from one palette.
        for dark in [false, true] {
            assert_eq!(
                editor_ground(dark),
                crate::reader::document::reader_ground(dark),
                "the composer and the reader disagree about the page colour"
            );
        }
    }

    #[test]
    fn a_scheme_change_produces_a_different_sheet_from_the_same_input() {
        // FR-075. Different *sheet*, not a different document: a reload would
        // take the caret and the undo history with it.
        let light = editor_css(Presentation::default());
        let dark = editor_css(Presentation {
            dark: true,
            ..Presentation::default()
        });
        assert_ne!(light, dark, "the scheme made no difference to the sheet");
        assert!(
            dark.matches("--r-ground").count() > light.matches("--r-ground").count(),
            "the dark sheet does not restate the palette, so it depends on \
             the engine agreeing about the scheme rather than on our choice"
        );
    }

    #[test]
    fn text_size_and_density_reach_the_sheet() {
        // FR-076. One number scales the whole sheet, rather than a rule per
        // element that can be half-applied.
        let css = editor_css(Presentation {
            text_size: 19,
            padding: 4,
            ..Presentation::default()
        });
        assert!(css.contains("--e-text-size: 19px"), "{css}");
        assert!(css.contains("--e-pad: 4px"), "{css}");
    }

    #[test]
    fn a_reply_quote_is_folded_in_the_readers_own_vocabulary() {
        // FR-041 and FR-077. `details.postio-quote` is what the reader uses,
        // so it is what the composer uses -- reuse beats inventing a second
        // way to say "this is quoted".
        let folded = fold_quotes(
            "<p>Acknowledged.</p><blockquote data-postio-quoted=\"1\">\
             <p>Do not reset.</p></blockquote>",
        );

        assert!(
            folded.contains(&format!("<details class=\"{QUOTE_CLASS}\"")),
            "the quote is not foldable: {folded}"
        );
        assert!(
            !folded.contains("<details class=\"postio-quote\" open"),
            "a reply must open with the quote closed, or the caret starts \
             below a screenful of someone else's mail: {folded}"
        );
        assert!(
            folded.contains("<summary"),
            "nothing says what the fold contains: {folded}"
        );
        assert!(
            folded.contains("<blockquote data-postio-quoted"),
            "the quote itself must survive inside the fold -- what is sent is \
             the blockquote, and `<details>` must never reach the wire: {folded}"
        );
    }

    #[test]
    fn a_quote_the_user_made_is_not_folded() {
        // Only the reply's quote folds. A `<blockquote>` the user typed is
        // their own writing, and hiding it behind a summary would be the
        // editor deciding their prose is someone else's.
        let plain = "<p>Hi</p><blockquote><p>as I said</p></blockquote>";
        assert_eq!(fold_quotes(plain), plain);
    }

    #[test]
    fn folding_leaves_everything_around_the_quote_alone() {
        let folded = fold_quotes(
            "<p>One</p><blockquote data-postio-quoted=\"1\"><p>Q</p></blockquote><p>Two</p>",
        );
        assert!(folded.starts_with("<p>One</p><details"), "{folded}");
        assert!(folded.ends_with("</details><p>Two</p>"), "{folded}");
    }
}
