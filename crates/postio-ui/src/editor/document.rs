//! The composer's editing document, toolkit-free and built like the reader's. Every colour
//! comes from the reader's generated palette.

use std::fmt::Write as _;

/// A fixed non-`http(s)` base, so edited content is never same-origin with anything real.
pub const EDITOR_BASE_URI: &str = "postio-editor:///";

/// Not shared with the reader's: the editor needs script and `contenteditable`, the reader neither.
pub const EDITOR_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:";

/// The reader's foldable-quote class; the rules are restated in `editor.css` because `reader.css`
/// carries too much.
pub const QUOTE_CLASS: &str = "postio-quote";

pub use postio_body::document::QUOTED_MARKER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub dark: bool,
    pub text_size: u32,
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

/// `content` is already narrowed by `postio_body::parse`; nothing here sanitises.
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

/// The variable block is last so the frontend's numbers win over the tokens file.
pub fn editor_css(presentation: Presentation) -> String {
    let mut css = String::new();
    css.push_str(include_str!("../../data/reader-tokens.css"));
    css.push_str(include_str!("../../data/editor.css"));
    let _ = write!(
        css,
        "\n:root {{\n  --e-text-size: {}px;\n  --e-pad: {}px;\n}}\n",
        presentation.text_size, presentation.padding
    );
    // The palette switches on `prefers-color-scheme`, which a web view resolves itself.
    // Restating the chosen half last under `:root` makes the app's choice win in both
    // directions (#1677); a new sheet, not a reload, which would lose the caret and undo history.
    if presentation.dark {
        css.push_str(&dark_tokens_restated());
    } else {
        let _ = write!(
            css,
            "\n:root {{{}\n}}\n",
            crate::reader::document::light_tokens()
        );
    }
    css
}

/// Needed outside the document too: a web view between documents has nothing to paint, which is the
/// white flash.
pub fn editor_ground(dark: bool) -> &'static str {
    crate::reader::document::reader_ground(dark)
}

fn dark_tokens_restated() -> String {
    use crate::reader::document::{DARK_BLOCK, PALETTE};

    let (_, dark) = PALETTE
        .split_once(DARK_BLOCK)
        .expect("the generated palette always emits a dark block");
    let body = dark
        .split_once('{')
        .and_then(|(_, rest)| rest.rsplit_once('}'))
        .map(|(inside, _)| inside)
        .unwrap_or_default();
    let inner = body
        .split_once('{')
        .and_then(|(_, rest)| rest.rsplit_once('}'))
        .map(|(declarations, _)| declarations)
        .unwrap_or(body);
    format!("\n:root {{{inner}}}\n")
}

/// Wraps each reply quote in the reader's foldable `<details>`, closed by default.
/// Presentation only: `<details>` must never reach the wire, which gets a plain `<blockquote>`.
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
        let light = editor_ground(false);
        let dark = editor_ground(true);
        assert!(light.starts_with('#'), "not a colour: {light:?}");
        assert!(dark.starts_with('#'), "not a colour: {dark:?}");
        assert_ne!(light, dark, "one ground for both schemes is one too few");
    }

    #[test]
    fn the_editor_ground_is_the_readers_ground() {
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
        let light = editor_css(Presentation::default());
        let dark = editor_css(Presentation {
            dark: true,
            ..Presentation::default()
        });
        assert_ne!(light, dark, "the scheme made no difference to the sheet");
    }

    #[test]
    fn each_scheme_states_its_palette_last_so_the_engine_cannot_overrule_it() {
        // #1677: the last `--r-ground` each sheet declares is the one that applies.
        let last_ground = |css: &str| {
            css.rsplit_once("--r-ground:")
                .and_then(|(_, rest)| rest.split_once(';'))
                .map(|(value, _)| value.trim().to_owned())
                .expect("the sheet declares a ground")
        };
        for dark in [false, true] {
            let css = editor_css(Presentation {
                dark,
                ..Presentation::default()
            });
            assert_eq!(last_ground(&css), editor_ground(dark), "dark = {dark}");
        }
    }

    #[test]
    fn text_size_and_density_reach_the_sheet() {
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
        // Only the reply's quote folds; a blockquote the user typed is their own prose.
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
