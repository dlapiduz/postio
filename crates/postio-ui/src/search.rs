//! Reading a search query as chips, and saying what a search turned out to be.
//!
//! Canvas 2b: `/` opens a bar, parsed operators become chips, and the
//! right-hand end of the field says how many hits and how long they took.
//! None of that is a toolkit's business — it is a reading of a
//! [`postio_search::ParsedQuery`] and a sentence about a result set.
//!
//! Both frontends draw these, so neither re-derives them: a second reading
//! of the query would be a second query vocabulary on screen, and the chips
//! in particular are how a user *learns* Postio's query language, so two of
//! them is two languages. The same goes for the readout wording and its
//! screen-reader form, and for what "still syncing" means.
//!
//! # Where the chips live
//!
//! The entry holds the *whole* query, and the chips are a parse of it drawn
//! alongside. They are a reading of what is typed, not a second store that
//! could disagree with it — which is why [`postio_search::ParsedQuery`] hands
//! out spans into the input.
//!
//! The alternative — lifting completed operators out of the entry into
//! standalone chips — is a nicer picture and a worse editor: the caret can no
//! longer move through the query, and every edit becomes a merge between two
//! representations. This way the entry is the truth and the chips follow it.

use std::time::Duration;

use postio_search::ParsedQuery;
use postio_search::query::{Field, TokenKind};

/// One chip: an operator the parser recognized in the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// Position of the token in [`ParsedQuery::tokens`].
    pub index: usize,
    /// The exact source text, so what the chip says is what is in the entry.
    pub label: String,
    /// The operator it belongs to.
    pub field: Field,
    /// Whether it was negated with a leading `-`.
    pub negated: bool,
    /// Whether the operator has a value yet. A half-typed `from:` is still
    /// worth drawing — it tells the user the parser understood the keyword.
    pub complete: bool,
}

/// The chips to draw for a query, in the order they were typed.
///
/// Free text is not a chip: it stays plain, because it is the part the user is
/// usually still editing.
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

/// How a chip reads to a screen reader.
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

// ---------------------------------------------------------------------------
// The live readout — canvas 2b's `14 hits · 11 ms`
// ---------------------------------------------------------------------------

/// What one search turned out to be.
///
/// The three numbers canvas 2b puts at the right-hand end of the field, and
/// the same three [`postio_search::SearchResults`] carries — this is that,
/// minus the hits themselves, because the readout does not need them and
/// copying a page of results to draw a number would be silly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// How many messages matched.
    pub hits: u64,
    /// Whether `hits` is a floor rather than the true count. See
    /// [`postio_search::SearchResults::total_hits_capped`].
    pub capped: bool,
    /// How long the search took.
    pub elapsed: Duration,
    /// Whether every message in the searched scope had a body to search.
    ///
    /// `false` adds the caveat: the hits come from a corpus that is still
    /// filling, so the count is a floor for a second reason (#352). Also
    /// `false` while an account in scope is rebuilding its local search
    /// index (#981, `postio_session::reindex_account`) — a message can drop
    /// out of results mid-rebuild the same way one that has not backfilled
    /// yet does, and it is the same honest caveat either way.
    pub corpus_complete: bool,
    /// Accounts a unified search could not reach, by the name the sidebar
    /// shows, in the sidebar's order.
    ///
    /// ADR 0005 Q10: *a view that cannot include an account says so, names
    /// the account, and stays usable.* Empty for a single-account search,
    /// which leaves nothing out, and empty for a unified search whose
    /// accounts all answered — the ordinary case, and the one this must not
    /// become furniture in.
    ///
    /// Names rather than ids because the only thing that ever reads it is a
    /// sentence a person reads, and a `Vec` rather than a `bool` because Q10
    /// asks for the account to be named: "some accounts are missing" is the
    /// disclosure people learn to ignore, since it never says which one to go
    /// and fix.
    ///
    /// **Not from [`postio_search::SearchResults`].** Which accounts answered
    /// is a fact about connections, and the executor only ever sees the
    /// store — a search of a store whose account is offline reads exactly
    /// like one whose account is fine. The composition root joins the two.
    pub unreachable: Vec<String>,
}

impl Outcome {
    /// Reads the outcome off a finished search.
    pub fn of(results: &postio_search::SearchResults) -> Self {
        Outcome {
            hits: results.total_hits,
            capped: results.total_hits_capped,
            elapsed: results.elapsed,
            corpus_complete: results.corpus_complete,
            // Filled by the caller: see the field.
            unreachable: Vec::new(),
        }
    }
}

/// What a search's refinements say when there are none to offer.
///
/// Never a blank space and never a shrug: the two reasons a shortlist can be
/// empty are different, and which one it is decides what the next keystroke
/// should be.
pub const NOTHING_MATCHED: &str = "Nothing matched, so there is nothing to narrow.";
/// [`NOTHING_MATCHED`]'s other half: there were matches, all alike.
pub const NOTHING_TO_NARROW: &str = "Every match is alike — nothing left to narrow by.";

// ---------------------------------------------------------------------------
// Painting the match — canvas 2b's "preview · match highlighted"
// ---------------------------------------------------------------------------

/// The class the reader stylesheet tints. See `reader.css`.
const MARK_CLASS: &str = "postio-match";

/// Tags whose contents are not prose and must not be marked.
///
/// `script` and `style` never survive `postio_body::sanitize`, and
/// `title` never appears in a body fragment — they are here because
/// "the sanitizer removes it" is a fact about another module, and a
/// highlighter that would corrupt a stylesheet if one ever reached it is one
/// bad refactor away from doing so.
const OPAQUE_TAGS: [&str; 3] = ["script", "style", "title"];

/// Wraps every place `terms` match in `html` with a `<mark>` the reader
/// stylesheet tints.
///
/// Applied *after* sanitizing, not before: ammonia would strip the `<mark>`
/// as an unknown tag, and marking first would mean running a matcher over
/// markup that has not been cleaned yet. What goes in is already-safe HTML
/// and what comes out adds one fixed literal tag to it — no attacker-shaped
/// string is ever interpolated.
///
/// Matches never cross a tag boundary. `<b>mail</b>dir` is two text runs and
/// FTS5 would not have matched `maildir` across them either, so the
/// highlighting agrees with why the message was a hit.
pub fn mark_html(html: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return html.to_owned();
    }

    let mut out = String::with_capacity(html.len());
    let mut run = String::new();
    let mut rest = html;
    // `Some(tag)` while inside an element whose contents are not prose.
    let mut opaque: Option<&str> = None;

    while !rest.is_empty() {
        let Some(next) = rest.find(['<', '&']) else {
            run.push_str(rest);
            break;
        };
        run.push_str(&rest[..next]);
        rest = &rest[next..];

        if rest.starts_with('&') {
            // An entity is one indivisible character as far as the reader is
            // concerned, and splitting one would corrupt it. It also ends the
            // token run, which is right: `&amp;` is punctuation.
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

        // A tag. Copy it through untouched, and note whether it opens or
        // closes something whose contents must be left alone.
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

/// Empties `run` into `out`, marking the matches if this run is prose.
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

/// The lower-cased element name of a tag, opening or closing.
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

    // -- painting the match -----------------------------------------------

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
        // `title` is an attribute here, and `p` an element name. Marking
        // either would produce markup, not a highlight.
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

/// The commands the search bar hints at, and the canvas's labels for them
/// (canvas 05: `Ret open · Tab refine · C-s save as folder`).
///
/// Two are registry commands rather than strings: a footer that taught a key
/// nothing answers is worse than one that taught none, which is the same
/// argument `row::hints` makes one pane over. The third, Tab into the chips,
/// is the search bar's own: it was bare Tab's pane cycle before the
/// three-pane app's commands went (specs/009-focus-macos R5), and now no
/// command owns it, so it is fixed and `hints` places it between the two.
const HINT_COMMANDS: [(postio_core::CommandId, &str); 2] = [
    (postio_core::CommandId::OpenMessage, "open"),
    (postio_core::CommandId::SaveSearch, "save as folder"),
];

/// The search bar's own Tab, into the refine chips.
const REFINE_HINT: (&str, &str) = ("Tab", "refine");

/// The key hints the search bar announces, as `(key, label)` pairs.
///
/// Read from the keymap, so a rebinding reaches the footer. A command with
/// no binding in force is left out rather than drawn without one.
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
        // The whole reason this reads the keymap: a footer promising `⌘S`
        // to somebody who rebound it is teaching them something false, and
        // the footer is the only place most people will ever read it.
        // `defaults()`, not `default()`: the latter is an *empty* keymap,
        // which is a real state (nothing bound) and not the one the footer
        // is drawn from.
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
        // The footer is the only place most people will ever read these
        // keys, so one that kept teaching the registry default after a
        // rebinding would be teaching something false to exactly the person
        // who took the trouble to change it.
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
        // Against the keymap's own answer rather than against the string
        // that was written: `mod` is spelled for the platform on the way
        // through — Command on a Mac, Control on freedesktop — so asserting
        // the literal would pass on one machine and fail on the other.
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
