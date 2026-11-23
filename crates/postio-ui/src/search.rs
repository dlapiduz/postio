//! Query chips and the result readout, shared so both frontends show one query vocabulary.
//! The entry holds the whole query and the chips are a parse of it, so the entry stays the truth.

use std::time::Duration;

use postio_search::ParsedQuery;
use postio_search::query::{Field, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub index: usize,
    pub label: String,
    pub field: Field,
    pub negated: bool,
    /// A half-typed `from:` is still drawn: it shows the parser understood the keyword.
    pub complete: bool,
}

/// Free text is not a chip: it is the part still being edited.
pub fn chips(parsed: &ParsedQuery) -> Vec<Chip> {
    parsed
        .tokens()
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            let field = token.field()?;
            Some(Chip {
                index,
                label: token.raw.clone(),
                field,
                negated: token.negated(),
                complete: matches!(token.kind, TokenKind::Filter(_)),
            })
        })
        .collect()
}

pub fn spoken(chip: &Chip) -> String {
    let field = chip.field.keyword();
    let value = chip
        .label
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or_default();
    match (chip.negated, chip.complete) {
        (false, true) => format!("{field} {value}"),
        (true, true) => format!("not {field} {value}"),
        (false, false) => format!("{field}, no value yet"),
        (true, false) => format!("not {field}, no value yet"),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub hits: u64,
    pub capped: bool,
    pub elapsed: Duration,
    /// `false` while the corpus is still filling or an account in scope is reindexing (#352, #981):
    /// the count is a floor.
    pub corpus_complete: bool,
    /// Accounts a unified search could not reach, by sidebar name (ADR 0005 Q10: such a view names
    /// the account).
    /// Empty when all answered. Filled by the composition root: the executor sees only the store.
    pub unreachable: Vec<String>,
}

impl Outcome {
    pub fn of(results: &postio_search::SearchResults) -> Self {
        Outcome {
            hits: results.total_hits,
            capped: results.total_hits_capped,
            elapsed: results.elapsed,
            corpus_complete: results.corpus_complete,
            unreachable: Vec::new(),
        }
    }
}

const MARK_CLASS: &str = "postio-match";

/// `script` and `style` never survive the sanitizer; listed so the highlighter cannot corrupt one
/// that reaches it.
const OPAQUE_TAGS: [&str; 3] = ["script", "style", "title"];

/// Applied after sanitizing, since ammonia would strip `<mark>`; no sender string is interpolated.
/// Matches never cross a tag boundary, as FTS5 would not match across one.
pub fn mark_html(html: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return html.to_owned();
    }

    let mut out = String::with_capacity(html.len());
    let mut run = String::new();
    let mut rest = html;
    let mut opaque: Option<&str> = None;

    while !rest.is_empty() {
        let Some(next) = rest.find(['<', '&']) else {
            run.push_str(rest);
            break;
        };
        run.push_str(&rest[..next]);
        rest = &rest[next..];

        if rest.starts_with('&') {
            // An entity is one indivisible character; splitting it corrupts it.
            let end = rest
                .find(';')
                .filter(|end| *end <= 12)
                .map(|end| end + 1)
                .unwrap_or(1);
            flush(&mut out, &mut run, terms, opaque.is_none());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }

        let end = rest.find('>').map(|end| end + 1).unwrap_or(rest.len());
        let tag = &rest[..end];
        flush(&mut out, &mut run, terms, opaque.is_none());
        out.push_str(tag);
        rest = &rest[end..];

        let name = tag_name(tag);
        match opaque {
            Some(open) if tag.starts_with("</") && name == Some(open) => opaque = None,
            None if !tag.starts_with("</") => {
                if let Some(name) = name.filter(|name| OPAQUE_TAGS.contains(name)) {
                    opaque = Some(name);
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut run, terms, opaque.is_none());
    out
}

fn flush(out: &mut String, run: &mut String, terms: &[String], prose: bool) {
    if run.is_empty() {
        return;
    }
    if !prose {
        out.push_str(run);
        run.clear();
        return;
    }
    let highlighted = postio_search::highlight::highlight(run, terms);
    for (piece, matched) in highlighted.runs() {
        if matched {
            out.push_str("<mark class=\"");
            out.push_str(MARK_CLASS);
            out.push_str("\">");
            out.push_str(piece);
            out.push_str("</mark>");
        } else {
            out.push_str(piece);
        }
    }
    run.clear();
}

fn tag_name(tag: &str) -> Option<&str> {
    let body = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .trim_end_matches('>')
        .trim_end_matches('/');
    let name = body.split([' ', '\t', '\n', '\r']).next()?;
    (!name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric())).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn a_matched_word_is_wrapped_where_it_stands() {
        assert_eq!(
            mark_html("<p>the maildir index</p>", &terms(&["maildir"])),
            "<p>the <mark class=\"postio-match\">maildir</mark> index</p>"
        );
    }

    #[test]
    fn a_query_with_no_terms_leaves_the_markup_alone() {
        let html = "<p>the maildir index</p>";
        assert_eq!(mark_html(html, &[]), html);
    }

    #[test]
    fn a_term_inside_a_tag_is_not_a_word_on_the_page() {
        let html = r#"<p title="maildir">nothing</p>"#;
        assert_eq!(mark_html(html, &terms(&["maildir", "p"])), html);
    }

    #[test]
    fn a_match_never_crosses_a_tag_boundary() {
        let html = "<b>mail</b>dir";
        assert_eq!(
            mark_html(html, &terms(&["maildir"])),
            html,
            "FTS5 did not match across the tag either, so nothing here may"
        );
    }

    #[test]
    fn an_entity_survives_being_marked_around() {
        assert_eq!(
            mark_html("a &amp; maildir", &terms(&["maildir"])),
            "a &amp; <mark class=\"postio-match\">maildir</mark>"
        );
        assert_eq!(
            mark_html("a &amp; b", &terms(&["amp"])),
            "a &amp; b",
            "`&amp;` is one character, not the word `amp`"
        );
    }

    #[test]
    fn a_bare_ampersand_does_not_swallow_the_rest_of_the_body() {
        assert_eq!(
            mark_html("Tom & Jerry maildir", &terms(&["maildir"])),
            "Tom & Jerry <mark class=\"postio-match\">maildir</mark>"
        );
    }

    #[test]
    fn a_stylesheet_is_not_prose() {
        let html = "<style>.maildir { color: red }</style><p>maildir</p>";
        assert_eq!(
            mark_html(html, &terms(&["maildir"])),
            "<style>.maildir { color: red }</style><p><mark class=\"postio-match\">maildir</mark></p>",
            "marking inside a stylesheet would corrupt it"
        );
    }

    #[test]
    fn several_matches_across_several_elements_are_all_painted() {
        assert_eq!(
            mark_html("<p>maildir one</p><p>two maildir</p>", &terms(&["maildir"])),
            "<p><mark class=\"postio-match\">maildir</mark> one</p>\
             <p>two <mark class=\"postio-match\">maildir</mark></p>"
        );
    }
}

/// Two registry commands, so a footer never teaches a key nothing answers; the search bar's own Tab
/// sits between them.
const HINT_COMMANDS: [(postio_core::CommandId, &str); 2] = [
    (postio_core::CommandId::OpenMessage, "open"),
    (postio_core::CommandId::SaveSearch, "save as folder"),
];

const REFINE_HINT: (&str, &str) = ("Tab", "refine");

/// Read from the keymap so a rebinding reaches the footer; an unbound command is left out.
pub fn hints(keymap: &postio_core::Keymap) -> Vec<(String, &'static str)> {
    let bound = |(command, label): (postio_core::CommandId, &'static str)| {
        keymap.binding(command).map(|key| (key.to_string(), label))
    };
    let [open, save] = HINT_COMMANDS;
    [
        bound(open),
        Some((REFINE_HINT.0.to_owned(), REFINE_HINT.1)),
        bound(save),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod hint_tests {
    use super::*;

    #[test]
    fn the_footer_teaches_the_keys_in_force_rather_than_the_defaults() {
        // `defaults()`, not `default()`: the latter is an empty keymap.
        let keymap = postio_core::Keymap::defaults();
        let hints = hints(keymap);
        assert_eq!(hints.len(), HINT_COMMANDS.len() + 1);
        assert!(
            hints.iter().all(|(key, _)| !key.is_empty()),
            "a hint drawn with no key on it: {hints:?}"
        );
    }

    #[test]
    fn a_rebinding_reaches_the_footer() {
        let mut overrides = postio_config::keys::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("save_search".to_owned(), "mod+shift+k".to_owned());
        let keymap = postio_core::Keymap::resolve(&overrides);

        let hints = hints(&keymap);
        let (key, _) = hints
            .iter()
            .find(|(_, label)| *label == "save as folder")
            .expect("the hint is still offered");
        // Compare against the keymap's answer: `mod` is spelled per platform.
        assert_eq!(
            Some(key.as_str()),
            keymap.binding(postio_core::CommandId::SaveSearch),
            "the footer taught something other than the binding in force"
        );
        assert_ne!(
            Some(key.as_str()),
            postio_core::Keymap::defaults().binding(postio_core::CommandId::SaveSearch),
            "the override changed nothing, so this proves nothing"
        );
    }
}
