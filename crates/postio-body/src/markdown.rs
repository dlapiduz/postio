//! Markdown, in both directions, for a frontend that draws text.
//!
//! [`from_html`] is the terminal reader's: the reader sanitiser's output, as
//! CommonMark a terminal can style. Only ever sanitised HTML -- converting
//! *after* the sanitiser is what keeps every guarantee it makes
//! (`specs/005-tui-frontend` research R5, `contracts/markdown.md`).
//!
//! Two things are Postio's own on the way out, because Markdown has no word
//! for them:
//!
//! * an image is `![alt](postio-image:N)`, a placeholder the terminal draws
//!   and never follows -- including an image whose remote `src` the sanitiser
//!   removed, which would otherwise vanish without a trace;
//! * a folded quote (`<details>`, from [`crate::fold_html_quotes`]) is its
//!   content between [`FOLD`] and [`END_FOLD`] lines, which the terminal
//!   turns into a block that expands.

/// The line that opens a folded quote in [`from_html`]'s output.
///
/// Private-use characters, so ordinary text does not look like one. A sender
/// who writes them can make text fold, which is all they can do with it.
pub const FOLD: &str = "\u{E000}fold\u{E000}";

/// The line that closes a folded quote.
pub const END_FOLD: &str = "\u{E000}end\u{E000}";

/// The scheme an image placeholder carries in [`from_html`]'s output.
pub const IMAGE_SCHEME: &str = "postio-image";

/// Sanitised HTML as Markdown for a terminal.
pub fn from_html(sanitized: &str) -> String {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use htmd::element_handler::Handlers;
    use htmd::{Element, HtmlToMarkdown};

    let images = Arc::new(AtomicUsize::new(0));
    let converter = HtmlToMarkdown::builder()
        .add_handler(vec!["img"], move |_: &dyn Handlers, element: Element| {
            let alt = element
                .attrs
                .iter()
                .find(|attribute| &*attribute.name.local == "alt")
                .map(|attribute| attribute.value.trim().to_owned())
                .filter(|alt| !alt.is_empty())
                .unwrap_or_else(|| "image".to_owned());
            let number = images.fetch_add(1, Ordering::Relaxed);
            // Brackets would end the alt text early; the placeholder's words
            // are the sender's, and only ever drawn, never followed.
            let alt = alt.replace(['[', ']'], "");
            Some(format!("![{alt}]({IMAGE_SCHEME}:{number})").into())
        })
        .add_handler(
            vec!["details"],
            |handlers: &dyn Handlers, element: Element| {
                let inner = handlers.walk_children(element.node);
                Some(format!("\n\n{FOLD}\n\n{}\n\n{END_FOLD}\n\n", inner.content.trim()).into())
            },
        )
        // The terminal draws its own line for a fold; the sanitiser's summary
        // words would only repeat it.
        .add_handler(vec!["summary"], |_: &dyn Handlers, _: Element| {
            Some("".into())
        })
        .build();
    // A converter that cannot read what the sanitiser wrote has nothing to
    // show; the plain text is what the reader falls back to then.
    converter.convert(sanitized).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RemoteImages, fold_html_quotes, sanitize_body};

    fn read(html: &str) -> String {
        from_html(&fold_html_quotes(
            &sanitize_body(html, RemoteImages::Blocked).html,
        ))
    }

    #[test]
    fn structure_survives() {
        let markdown = read(
            "<h2>Plan</h2><p>Some <b>bold</b> and <a href=\"https://example.com/x\">a link</a>.</p><ul><li>one</li><li>two</li></ul>",
        );
        assert!(markdown.contains("Plan"), "{markdown}");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(
            markdown.contains("[a link](https://example.com/x)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("one") && markdown.contains("two"),
            "{markdown}"
        );
    }

    #[test]
    fn a_blocked_remote_image_leaves_a_placeholder_not_nothing() {
        let markdown = read(
            "<p>Before</p><img src=\"https://tracker.example.org/p.gif\" alt=\"Logo\"><p>After</p>",
        );
        assert!(markdown.contains("![Logo](postio-image:"), "{markdown}");
        assert!(!markdown.contains("tracker.example.org"), "{markdown}");
    }

    #[test]
    fn an_image_with_no_alt_is_still_named() {
        let markdown = read("<img src=\"https://example.org/a.png\">");
        assert!(markdown.contains("![image](postio-image:"), "{markdown}");
    }

    #[test]
    fn a_folded_quote_is_marked_so_it_can_expand() {
        let markdown = read("<p>Thanks!</p><blockquote><p>Earlier words</p></blockquote>");
        let fold = markdown
            .find(FOLD)
            .unwrap_or_else(|| panic!("no fold: {markdown:?}"));
        let end = markdown
            .find(END_FOLD)
            .unwrap_or_else(|| panic!("no end: {markdown:?}"));
        let inside = &markdown[fold..end];
        assert!(inside.contains("Earlier words"), "{markdown:?}");
        assert!(markdown[..fold].contains("Thanks!"), "{markdown:?}");
    }

    #[test]
    fn a_table_is_still_a_table() {
        let markdown = read(
            "<table><tr><th>Item</th><th>Qty</th></tr><tr><td>Widget</td><td>3</td></tr></table>",
        );
        assert!(
            markdown.contains("| Item") && markdown.contains("| Widget"),
            "{markdown}"
        );
    }
}
