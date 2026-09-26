//! `markdown::from_html` over the whole corpus: what the reader's sanitiser
//! emits, as Markdown a terminal can show safely (`specs/005-tui-frontend`
//! T005, then T040; SC-005).
//!
//! The promises are `contracts/markdown.md`'s `from_html` list. The input is
//! always sanitised HTML, never raw mail -- converting *after* the sanitiser is
//! what keeps its guarantees (research R5).

use postio_body::markdown::from_html;
use postio_body::{RemoteImages, fold_html_quotes, sanitize_body};

/// Every corpus message with an HTML body, sanitised as the reader does it
/// and converted, with its fixture name.
fn converted() -> Vec<(String, String)> {
    postio_model::test_corpus::all()
        .iter()
        .filter_map(|fixture| {
            let message = fixture.parse();
            let html = message.body.html.as_deref()?;
            let sanitized = sanitize_body(html, RemoteImages::Blocked);
            let folded = fold_html_quotes(&sanitized.html);
            let markdown = from_html(&folded);
            Some((fixture.name().to_string(), markdown))
        })
        .collect()
}

#[test]
fn no_corpus_message_converts_to_markup_script_or_a_remote_image() {
    let all = converted();
    assert!(
        all.len() >= 5,
        "only {} HTML messages were converted; the corpus loader is not \
         finding them and this test is passing over nothing",
        all.len()
    );
    let mut failures = Vec::new();
    for (name, markdown) in &all {
        let lower = markdown.to_ascii_lowercase();
        for forbidden in [
            "<script",
            "<iframe",
            "<object",
            "<embed",
            "<img",
            "<style",
            "<div",
            "<span",
            "<table",
            "<p>",
            "<a ",
            "javascript:",
            "](http://",
            "](https://",
        ] {
            // A link is fine; an *image* pointing at the network is not.
            let hit = match forbidden {
                "](http://" | "](https://" => {
                    lower.contains(&format!("!{}", "["))
                        && markdown.match_indices("![").any(|(i, _)| {
                            markdown[i..]
                                .find("](")
                                .map(|j| {
                                    let rest = &markdown[i + j + 2..];
                                    rest.starts_with("http://") || rest.starts_with("https://")
                                })
                                .unwrap_or(false)
                        })
                }
                _ => lower.contains(forbidden),
            };
            if hit {
                failures.push(format!("{name}: {forbidden:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "from_html broke a promise:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_table_survives_as_a_markdown_table() {
    let html = "<table><thead><tr><th>Item</th><th>Qty</th></tr></thead>\
                <tbody><tr><td>Widget</td><td>3</td></tr></tbody></table>";
    let sanitized = sanitize_body(html, RemoteImages::Blocked);
    let markdown = from_html(&sanitized.html);
    assert!(
        markdown.contains("| Item") && markdown.contains("| Widget"),
        "the table was flattened: {markdown:?}"
    );
}
