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

/// The readout, as the canvas writes it: `14 hits · 11 ms`.
///
/// No thousands separators, because the canvas' own scope counts are written
/// `4291` and two number formats in one column would read as two kinds of
/// number. A capped count is written `10000+ hits` rather than a bare figure,
/// so a floor never passes for a total.
/// A corpus still filling adds `· still syncing`, and nothing otherwise.
///
/// The wording is a *state that ends*, which is the whole of #352's design
/// call. ADR 0016 backfills every folder to completion by default, so "you do
/// not have this mail" would be false — the honest thing is that the answer is
/// not final yet. A count was rejected for the same reason: it would be a
/// draining queue reported as an alarm.
///
/// It says so once, here, rather than per result: a caveat repeated down a
/// list of hits stops being read by the third one.
pub fn readout(outcome: &Outcome) -> String {
    let mut line = format!("{} · {}", hits(outcome), elapsed(outcome.elapsed));
    if !outcome.corpus_complete {
        line.push_str(" · still syncing");
    }
    // Both, when both are true. They are different facts with different
    // fixes -- one ends on its own under ADR 0016, the other needs the
    // account to come back -- so neither may hide the other.
    match outcome.unreachable.as_slice() {
        [] => {}
        // One name fits and is worth more than a count: it says which
        // account to go and look at.
        [only] => line.push_str(&format!(" · {only} unreachable")),
        // Past one it does not fit, and a fixed slot is what keeps the field
        // from breathing per keystroke. The count still says there is more
        // than one to fix; `spoken_readout` carries the names.
        many => line.push_str(&format!(" · {} unreachable", many.len())),
    }
    line
}

/// The readout as a screen reader should hear it — the same facts, in words,
/// because "·" and "ms" are punctuation and an abbreviation rather than
/// something to read aloud.
pub fn spoken_readout(outcome: &Outcome) -> String {
    let elapsed = outcome.elapsed.as_millis();
    let counted = match elapsed {
        0 => format!("{}, in under a millisecond", hits(outcome)),
        1 => format!("{}, in 1 millisecond", hits(outcome)),
        _ => format!("{}, in {elapsed} milliseconds", hits(outcome)),
    };
    // The spoken form carries the sentence the visible one has no room for.
    // Three words are enough to *flag* a state beside a number; they are not
    // enough to explain one to somebody who cannot see the rest of the
    // window.
    let mut spoken = counted;
    if !outcome.corpus_complete {
        spoken.push_str(
            ". This account is still syncing, so messages whose text has not \
             arrived yet could not be searched.",
        );
    }
    // Every name, which is the whole reason the spoken form exists: the
    // visible caveat has room to flag the state and, past one account, not to
    // say which ones.
    if !outcome.unreachable.is_empty() {
        spoken.push_str(&format!(
            ". {} could not be searched, so this answer may be short.",
            and_list(&outcome.unreachable)
        ));
    }
    spoken
}

/// `a`, `a and b`, `a, b and c` — a list as somebody reads it aloud.
fn and_list(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, head)) => format!("{} and {last}", head.join(", ")),
    }
}

fn hits(outcome: &Outcome) -> String {
    match (outcome.hits, outcome.capped) {
        (_, true) => format!("{}+ hits", outcome.hits),
        (0, _) => "no hits".to_string(),
        (1, _) => "1 hit".to_string(),
        (hits, _) => format!("{hits} hits"),
    }
}

/// A duration, in the unit that makes it readable.
///
/// Sub-millisecond is written `<1 ms` rather than `0 ms`: the search did
/// happen, and a zero would read as one that did not.
fn elapsed(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    match millis {
        0 => "<1 ms".to_string(),
        1..=9_999 => format!("{millis} ms"),
        _ => format!("{:.1} s", elapsed.as_secs_f64()),
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
/// Three, and each is a registry command rather than a string: a footer that
/// taught a key nothing answers is worse than one that taught none, which is
/// the same argument `row::hints` makes one pane over.
const HINT_COMMANDS: [(postio_core::CommandId, &str); 3] = [
    (postio_core::CommandId::OpenMessage, "open"),
    (postio_core::CommandId::CyclePane, "refine"),
    (postio_core::CommandId::SaveSearch, "save as folder"),
];

/// The key hints the search bar announces, as `(key, label)` pairs.
///
/// Read from the keymap, so a rebinding reaches the footer. A command with
/// no binding in force is left out rather than drawn without one.
pub fn hints(keymap: &postio_core::Keymap) -> Vec<(String, &'static str)> {
    HINT_COMMANDS
        .iter()
        .filter_map(|(command, label)| {
            keymap
                .binding(*command)
                .map(|key| (key.to_string(), *label))
        })
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
        assert_eq!(hints.len(), HINT_COMMANDS.len());
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

/// What a screen reader says for one row of the scope rail — `Inbox only,
/// 2 matches` (#1157).
///
/// A sentence rather than a label beside a number, because the number is the
/// point: the rail says what switching *would* find before anybody switches.
/// A zero is said too, for the reason the rail draws one — an empty scope is
/// worth knowing about before choosing it. Shared, so both rails say it the
/// same way.
pub fn scope_spoken(scope: postio_search::facets::Scope, hits: u64) -> String {
    match hits {
        1 => format!("{}, 1 match", scope.label()),
        hits => format!("{}, {hits} matches", scope.label()),
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use postio_search::facets::Scope;

    #[test]
    fn a_scope_row_says_its_count_in_words() {
        assert_eq!(scope_spoken(Scope::Inbox, 2), "Inbox only, 2 matches");
        assert_eq!(
            scope_spoken(Scope::AllMail, 1),
            "All mail, 1 match",
            "one is not plural"
        );
        assert_eq!(
            scope_spoken(Scope::Lists, 0),
            "Lists, 0 matches",
            "a zero is said, not hidden"
        );
    }
}
