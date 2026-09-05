//! Finding a command: fuzzy matching over the command registry.
//!
//! # Why it exists
//!
//! docs/PRODUCT.md §8 asks that every command be reachable without memorizing
//! a binding. The palette is that promise kept: it is generated from
//! [`postio_core::registry`], so a command that exists is a command that can
//! be found, and one that gains a binding shows it here without anybody
//! editing a second list.
//!
//! # Why it is here rather than in a frontend
//!
//! It was `postio-gtk`'s until #658. Nothing in the matching, the ranking or
//! the context filter is about a toolkit — they are product decisions, and
//! ADR 0019 Q5 named this among what a second frontend must share rather than
//! re-derive. **Swift must not write its own fuzzy match**: the ranking is
//! this, and two rankings mean the same query offers different things on each
//! platform.
//!
//! What each frontend keeps is the *drawing*. [`Entry::positions`] are byte
//! indices into the title, deliberately, rather than pre-escaped markup —
//! `postio-gtk` turns them into Pango bold and Swift builds an
//! `AttributedString` from the same numbers.
//!
//! # Two halves
//!
//! Everything that decides *what to show* is [`entries`] and [`score`], pure
//! functions over a [`postio_core::Keymap`]. They are unit-tested with no
//! display and no main loop, which is also what makes the 16 ms budget
//! measurable rather than a hope.
//!
//! # What the rows say
//!
//! Title, then the binding in force on the right — the same arrangement the
//! canvas uses for the key hints on a focused row, so a key learned in the
//! palette looks the same when it appears in the list.

use postio_core::{ActionId, Context, Keymap, Scope, registry};

/// How many rows the palette will show at once.
///
/// The list scrolls past this; the cap is on what is *built*, so a query that
/// matches everything costs the same as one that matches three things.
///
/// Was 32, which turned out to be exactly `Context::List`'s reachable count
/// at the time -- so tight it was already a coincidence, not a bound. #438
/// added two commands there and an empty query silently stopped listing
/// everything reachable, which is the one thing this cap is not supposed to
/// cost (see `an_empty_query_lists_everything_reachable_in_registry_order`).
/// Raised for headroom, not tuned to a new exact count -- the next command
/// added to a full context should not trip this again.
pub const MAX_ROWS: usize = 48;

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Where a query matched, and how well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Higher is better. Only comparable between candidates for one query.
    pub score: i32,
    /// Byte indices in the candidate that the query matched, ascending.
    pub positions: Vec<usize>,
}

/// A match at the start of a word is worth far more than one mid-word: typing
/// `cp` should find "Command palette" ahead of "Copy".
const WORD_START: i32 = 24;
/// Each character that continues an unbroken run.
const CONSECUTIVE: i32 = 12;
/// Every character skipped over, up to a floor — a match spread across a long
/// title is still a match, just a worse one.
const GAP: i32 = -2;
/// The most any single gap can cost.
const GAP_FLOOR: i32 = -12;

/// Scores `query` as a subsequence of `candidate`, case-insensitively.
///
/// Returns `None` when the query is not a subsequence at all. An empty query
/// matches everything with a score of zero, which is what makes an
/// unfiltered palette fall back to registry order.
///
/// Greedy left-to-right: the first place each character can go is where it
/// goes. That is not optimal scoring — a backtracking matcher would find that
/// `cp` scores better against "Command **p**alette" than "**C**ommand
/// **p**alette" — but it is linear, and for a few dozen short titles the
/// difference is invisible while the cost of getting it wrong at scale is not.
pub fn score(query: &str, candidate: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }

    let mut positions = Vec::new();
    let mut total = 0;
    let mut run = 0;
    let mut last: Option<usize> = None;
    let mut haystack = candidate.char_indices().peekable();

    for wanted in query.chars() {
        let wanted = wanted.to_ascii_lowercase();
        let mut found = None;
        for (index, character) in haystack.by_ref() {
            if character.to_ascii_lowercase() == wanted {
                found = Some(index);
                break;
            }
        }
        let index = found?;

        if last.is_some_and(|previous| previous + 1 == index) {
            run += 1;
            total += CONSECUTIVE * run.min(3);
        } else {
            run = 0;
            if starts_a_word(candidate, index) {
                total += WORD_START;
            }
            if let Some(previous) = last {
                let skipped = (index - previous - 1) as i32;
                total += (GAP * skipped).max(GAP_FLOOR);
            } else {
                // Leading noise before the first match is worth less than a gap
                // inside it: "message" matching "Next message" is fine.
                total += (GAP * index as i32).max(GAP_FLOOR);
            }
        }

        positions.push(index);
        last = Some(index);
    }

    // Between two candidates that match equally well, the shorter one is the
    // one the user meant.
    total -= candidate.len() as i32 / 8;

    Some(Match {
        score: total,
        positions,
    })
}

/// Whether the byte at `index` begins a word.
fn starts_a_word(candidate: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    candidate[..index]
        .chars()
        .next_back()
        .is_some_and(|previous| !previous.is_alphanumeric())
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// One row of the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The command this row runs.
    ///
    /// An [`ActionId`], because `Ctrl+K` is where a registered command has to
    /// be equal to a built-in: it has a query box, so it can absorb a
    /// vocabulary that grows, which is exactly what the right-click menu
    /// cannot do.
    pub id: ActionId,
    /// Its title, as the registry gives it.
    pub title: &'static str,
    /// Its binding in force, or `None` when it is palette-only.
    ///
    /// Read from the live [`Keymap`], so a `[keys]` override shows here without
    /// anything else being told.
    pub binding: Option<String>,
    /// Byte indices in `title` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// Penalty for matching the stable id rather than the title.
///
/// `archive_thread` is findable by typing `archive_th`, but a title match for
/// the same query should always rank above it.
const ID_PENALTY: i32 = 40;

/// The rows to show for `query`, best first.
///
/// Filtered to commands reachable in `context` — offering to send a draft from
/// the message list is a row the user can only be disappointed by — and to
/// those `scope` satisfies, so a unified view does not offer a `Move` with no
/// account to move within (#182). An empty query returns everything
/// applicable, in registry order.
pub fn entries(keymap: &Keymap, context: Context, scope: Scope, query: &str) -> Vec<Entry> {
    let query = query.trim();
    let mut found: Vec<Entry> = registry::reachable_in(context, scope)
        .filter_map(|spec| {
            let by_title = score(query, spec.title);
            let by_id = score(query, spec.id.as_str());
            let matched = match (by_title, by_id) {
                (Some(title), _) => title,
                (None, Some(id)) => Match {
                    score: id.score - ID_PENALTY,
                    positions: Vec::new(),
                },
                (None, None) => return None,
            };
            Some(Entry {
                id: spec.id,
                title: spec.title,
                binding: keymap.binding(spec.id).map(str::to_owned),
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();

    // Stable, so an empty query — every score zero — comes out in registry
    // order rather than in whatever order the sort happened to leave it.
    found.sort_by_key(|entry| std::cmp::Reverse(entry.score));
    found.truncate(MAX_ROWS);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::CommandId;
    use postio_model::AccountId;

    /// These tests are about context filtering, so they run in the scope
    /// where every command is available: one account's own mailboxes.
    fn an_account() -> Scope {
        Scope::Account(AccountId::new(1))
    }

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    // -- matching ---------------------------------------------------------

    #[test]
    fn a_query_that_is_not_a_subsequence_does_not_match() {
        assert!(score("zzz", "Archive").is_none());
        assert!(score("ahcrive", "Archive").is_none(), "order matters");
    }

    #[test]
    fn an_empty_query_matches_everything_equally() {
        assert_eq!(score("", "Archive").expect("a match").score, 0);
        assert_eq!(score("", "Reply to all").expect("a match").score, 0);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("ARCH", "Archive").is_some());
        assert!(score("arch", "Archive").is_some());
    }

    #[test]
    fn the_positions_are_where_it_matched() {
        let matched = score("arc", "Archive").expect("a match");

        assert_eq!(matched.positions, vec![0, 1, 2]);
    }

    #[test]
    fn word_initials_beat_a_run_buried_mid_word() {
        let initials = score("ra", "Reply to all").expect("a match").score;
        let buried = score("ra", "Forward").expect("a match").score;

        assert!(
            initials > buried,
            "initials {initials} should beat mid-word {buried}"
        );
    }

    #[test]
    fn a_prefix_beats_the_same_letters_further_in() {
        // Synthetic rather than two registry titles: what is being pinned is the
        // scoring rule, and a real pair would also differ in length and word
        // boundaries.
        let prefix = score("com", "compose").expect("a match").score;
        let later = score("com", "recompose").expect("a match").score;

        assert!(prefix > later, "{prefix} vs {later}");
    }

    #[test]
    fn between_equal_matches_the_shorter_title_wins() {
        let short = score("f", "Flag").expect("a match").score;
        let long = score("f", "Forward the whole conversation")
            .expect("a match")
            .score;

        assert!(short > long, "{short} vs {long}");
    }

    // -- entries ----------------------------------------------------------

    #[test]
    fn an_empty_query_lists_everything_reachable_in_registry_order() {
        let listed = entries(&defaults(), Context::List, an_account(), "");
        let expected: Vec<ActionId> = registry::reachable(Context::List)
            .map(|spec| spec.id)
            .collect();

        assert_eq!(
            listed.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn every_registry_command_is_reachable_from_some_context() {
        for spec in registry::all() {
            let reachable = Context::ALL.iter().any(|context| {
                entries(&defaults(), *context, an_account(), spec.title)
                    .iter()
                    .any(|entry| entry.id == spec.id.into())
            });
            assert!(reachable, "`{}` cannot be found in the palette", spec.id);
        }
    }

    #[test]
    fn the_context_filter_hides_what_does_not_apply() {
        let from_the_list = entries(&defaults(), Context::List, an_account(), "send");
        assert!(
            !from_the_list
                .iter()
                .any(|entry| entry.id == ActionId::Builtin(CommandId::Send)),
            "offering to send from the message list is a row that can only disappoint"
        );

        let from_the_composer = entries(&defaults(), Context::Composer, an_account(), "send");
        assert!(
            from_the_composer
                .iter()
                .any(|entry| entry.id == ActionId::Builtin(CommandId::Send))
        );
    }

    #[test]
    fn a_command_is_findable_by_its_id_as_well_as_its_title() {
        let found = entries(&defaults(), Context::List, an_account(), "archive_th");

        assert_eq!(
            found.first().map(|entry| entry.id),
            Some(ActionId::Builtin(CommandId::ArchiveThread))
        );
    }

    #[test]
    fn a_title_match_outranks_an_id_match_for_the_same_query() {
        let found = entries(&defaults(), Context::List, an_account(), "archive");
        let ranks: Vec<ActionId> = found.iter().map(|entry| entry.id).collect();

        let archive = ranks
            .iter()
            .position(|id| *id == ActionId::Builtin(CommandId::Archive));
        assert_eq!(archive, Some(0), "{ranks:?}");
    }

    #[test]
    fn rows_carry_the_binding_in_force() {
        let listed = entries(&defaults(), Context::List, an_account(), "archive");
        let archive = listed
            .iter()
            .find(|entry| entry.id == ActionId::Builtin(CommandId::Archive))
            .expect("archive");

        assert_eq!(archive.binding.as_deref(), Some("a"));
    }

    #[test]
    fn a_rebound_command_shows_its_new_key() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_owned(), "y".to_owned());
        let keymap = Keymap::resolve(&overrides);

        let listed = entries(&keymap, Context::List, an_account(), "archive");
        let archive = listed
            .iter()
            .find(|entry| entry.id == ActionId::Builtin(CommandId::Archive))
            .expect("archive");

        assert_eq!(
            archive.binding.as_deref(),
            Some("y"),
            "the palette reads the live keymap, not the registry default"
        );
    }

    /// The palette is the surface #182's acceptance names: a unified view
    /// must not offer a destination it has no way to pick. The registry
    /// decides; this proves the answer actually reaches the rows.
    #[test]
    fn a_unified_view_offers_no_move_and_an_account_view_does() {
        let in_account: Vec<&str> = entries(&defaults(), Context::List, an_account(), "move")
            .iter()
            .map(|entry| entry.title)
            .collect();
        assert!(
            in_account.contains(&"Move to…"),
            "an account view is exactly where moving into a folder means something: {in_account:?}"
        );

        let unified: Vec<&str> = entries(&defaults(), Context::List, Scope::Unified, "move")
            .iter()
            .map(|entry| entry.title)
            .collect();
        assert!(
            !unified.contains(&"Move to…"),
            "offering Move across every account promises a folder the user was \
             never given the chance to pick: {unified:?}"
        );
    }

    #[test]
    fn a_query_that_matches_nothing_lists_nothing() {
        assert!(entries(&defaults(), Context::List, an_account(), "zzzzz").is_empty());
    }

    #[test]
    fn the_list_is_capped() {
        let listed = entries(&defaults(), Context::List, an_account(), "");

        assert!(listed.len() <= MAX_ROWS);
    }
}
