//! Fixtures name their resource under `<family>.vector.example.com`; with remote content blocked it
//! must appear nowhere in sanitised markup, styles, treated markup, or a reply or forward (#1926).

use postio_body::sanitize::{self, RemoteImages};
use postio_body::treatment;
use postio_model::test_corpus;

const SUFFIX: &str = "vector.example.com";

const FAMILIES: [&str; 8] = [
    "hostile-remote-stylesheet-link",
    "hostile-remote-srcset-picture",
    "hostile-remote-media-poster",
    "hostile-remote-frames-embeds",
    "hostile-remote-svg-images",
    "hostile-remote-meta-refresh",
    "hostile-remote-css-obfuscated",
    "hostile-remote-attribute-variants",
];

fn leaks(emitted: &str) -> Vec<String> {
    let lower = emitted.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(SUFFIX) {
        let at = from + at;
        let start = lower[..at]
            .rfind(['"', '\'', ' ', '(', '>'])
            .map_or(0, |i| i + 1);
        found.push(lower[start..(at + SUFFIX.len()).min(lower.len())].to_owned());
        from = at + SUFFIX.len();
    }
    found
}

fn message(name: &str) -> postio_model::Message {
    let fixture = test_corpus::get(name).unwrap_or_else(|| panic!("{name} is not in the corpus"));
    let message = fixture.parse();
    let html = message.body.html.as_deref().expect("a text/html fixture");
    assert!(html.contains(SUFFIX), "{name} lost its remote references");
    message
}

#[test]
fn blocked_remote_content_leaves_no_network_url_in_the_sanitised_document() {
    let mut failures = Vec::new();
    for name in FAMILIES {
        let message = message(name);
        let html = message.body.html.as_deref().unwrap();
        let sanitized = sanitize::sanitize_body(html, RemoteImages::Blocked);
        let treated = treatment::app_colours(&sanitized.html);
        for (what, emitted) in [
            ("html", &sanitized.html),
            ("styles", &sanitized.styles),
            ("treated html", &treated),
        ] {
            if !leaks(emitted).is_empty() {
                failures.push(format!("{name}: {what} names {:?}", leaks(emitted)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_reply_and_a_forward_quote_no_network_url() {
    let mut failures = Vec::new();
    for name in FAMILIES {
        let message = message(name);
        for forward in [false, true] {
            let body = postio_body::replying::quoted_body(&message, forward);
            let kind = if forward { "forward" } else { "reply" };
            for (what, emitted) in [
                ("html", body.html.as_deref().unwrap_or_default()),
                ("text", body.text.as_deref().unwrap_or_default()),
            ] {
                if !leaks(emitted).is_empty() {
                    failures.push(format!(
                        "{name}: the {kind}'s {what} names {:?}",
                        leaks(emitted)
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
