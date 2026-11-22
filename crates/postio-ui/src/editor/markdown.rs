//! Which typed sequences become which formatting command.
//!
//! Markdown as *input*, never as the draft's format. The spec's first
//! clarification settled that on 2026-09-10 and ADR 0003 had settled it
//! before: a draft is rich text, and `**bold**` is a way of reaching the bold
//! command with the keyboard rather than a dialect the message is written in.
//! Nothing here parses a document; nothing here runs on send.
//!
//! # Why the table is in `postio-ui` when only GTK can act on it
//!
//! The mechanism is genuinely frontend-local — the transformation happens in
//! the WebView's own script, because a round trip to Rust per keystroke does
//! not fit a 16 ms budget. The *rule* is not: two frontends each inventing
//! their own set is how `- ` comes to make a list on one platform and not the
//! other. So the set lives here, both frontends read it, and a test in each
//! can assert it implements this table and no more.
//!
//! # What bounds the set
//!
//! FR-068: every sequence maps to formatting an existing command already
//! produces. That is what keeps this from growing into a markdown dialect —
//! there is no sequence for a footnote because there is no command for one,
//! and adding a sequence without a command would create document structure
//! no other surface could make or undo.
//!
//! # Spec 014: the table is a contract ahead of its surface
//!
//! Spec 014 (`specs/014-compose-formatting/contracts/sequences.md`) widened
//! the table to every construct the composer authors, beside spec 002's
//! original rows. The rich surface that will type them live is ADR 0039's
//! native composer, so the table is written now as the test that surface
//! has to meet. Today's WebKit bridge implements only the rows marked
//! [`Sequence::bridged`], and [`bridged`] is what `postio_ui::compose`
//! generates into it.

/// Where a sequence is recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Recognised at the start of a block, when the marker is followed by a
    /// space — `# `, `- `, `> `. The marker and the space are consumed.
    LinePrefix,
    /// Recognised when the closing marker is typed around a run of text on
    /// one line — `**bold**`. Both markers are consumed.
    Wrapping,
    /// Recognised on Enter, when the block reads exactly the marker —
    /// `---`, a fence, a table's delimiter row.
    LineComplete,
    /// Recognised on the space or punctuation that ends a word — a bare
    /// autolink.
    WordComplete,
}

/// What is typed: a fixed string, or one of four small patterns.
///
/// An enum rather than a regular expression, so the table stays enumerable
/// and each pattern is a rule a unit case can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// Exactly these characters.
    Literal(&'static str),
    /// An ordered list's marker: one to nine digits, then `.` or `)`.
    Ordered,
    /// A code fence: three or more backticks or tildes, then an optional
    /// language word.
    Fence,
    /// A table's delimiter row, `| :-- | --: |`, under a pipe row of the
    /// same width.
    DelimiterRow,
    /// A run of backticks, closed by a run of the same length.
    BacktickRun,
    /// `[text](url)`, closed by its `)`.
    Link,
    /// `<url>`, closed by its `>`.
    AngleLink,
    /// A GFM extended autolink (`www.`, `http(s)://`, an address).
    Autolink,
}

impl Marker {
    /// Whether `text` is this marker. For the patterns, whether it is one
    /// instance of the pattern; the wrapping patterns ([`Marker::Link`],
    /// [`Marker::AngleLink`], [`Marker::BacktickRun`]) and autolinks are
    /// matched over a run of text, so only their delimiters are checked.
    pub fn matches(self, text: &str) -> bool {
        match self {
            Marker::Literal(literal) => text == literal,
            Marker::Ordered => {
                let Some(digits) = text.strip_suffix('.').or_else(|| text.strip_suffix(')')) else {
                    return false;
                };
                (1..=9).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
            }
            Marker::Fence => {
                let fence = text.trim_end();
                let Some(first) = fence.chars().next().filter(|c| matches!(c, '`' | '~')) else {
                    return false;
                };
                let run = fence.chars().take_while(|c| *c == first).count();
                let info = fence[run..].trim();
                run >= 3
                    && (info.is_empty() || postio_body::Language::parse(info).is_some())
                    && !(first == '`' && info.contains('`'))
            }
            Marker::DelimiterRow => {
                let row = text.trim();
                let inner = row.trim_start_matches('|').trim_end_matches('|');
                row.contains('-')
                    && (row.starts_with('|') || row.contains('|'))
                    && inner.split('|').all(|cell| {
                        let cell = cell.trim();
                        let dashes = cell.trim_start_matches(':').trim_end_matches(':');
                        !dashes.is_empty() && dashes.bytes().all(|b| b == b'-')
                    })
            }
            Marker::BacktickRun => !text.is_empty() && text.bytes().all(|b| b == b'`'),
            Marker::Link => text == ")",
            Marker::AngleLink => text == ">",
            Marker::Autolink => false,
        }
    }

    /// The literal, if this is one.
    pub fn literal(self) -> Option<&'static str> {
        match self {
            Marker::Literal(literal) => Some(literal),
            _ => None,
        }
    }
}

/// One typed sequence and the commands it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// What is typed. For [`Trigger::LinePrefix`] the trailing space is
    /// implied; for [`Trigger::Wrapping`] a literal is the marker on both
    /// sides.
    pub marker: Marker,
    /// The `postio-core` command ids this produces, in the order they apply
    /// (`***` is bold, then italic), as the editing bridge spells them.
    pub commands: &'static [&'static str],
    /// Where it is recognised.
    pub trigger: Trigger,
    /// Whether today's WebKit bridge implements the row (spec 014 R15).
    pub bridged: bool,
}

const fn row(
    marker: Marker,
    commands: &'static [&'static str],
    trigger: Trigger,
    bridged: bool,
) -> Sequence {
    Sequence {
        marker,
        commands,
        trigger,
        bridged,
    }
}

use Marker::{Literal, Ordered};
use Trigger::{LineComplete, LinePrefix, WordComplete, Wrapping};

/// Every supported sequence, spec 014's `contracts/sequences.md` row for row.
///
/// Ordered longest marker first within each trigger, because a recogniser
/// that checks `*` before `**` never sees `**` — the ordering is part of the
/// contract rather than an implementation detail of one frontend.
pub const SEQUENCES: &[Sequence] = &[
    // -- Wrapping ---------------------------------------------------------
    row(Literal("***"), &["bold", "italic"], Wrapping, false),
    row(Literal("___"), &["bold", "italic"], Wrapping, false),
    row(Literal("**"), &["bold"], Wrapping, true),
    row(Literal("__"), &["bold"], Wrapping, false),
    row(Literal("~~"), &["strikethrough"], Wrapping, false),
    row(Literal("*"), &["italic"], Wrapping, true),
    row(Literal("_"), &["italic"], Wrapping, true),
    row(Marker::BacktickRun, &["inline_code"], Wrapping, false),
    row(Marker::Link, &["insert_link"], Wrapping, false),
    row(Marker::AngleLink, &["insert_link"], Wrapping, false),
    // -- WordComplete -----------------------------------------------------
    row(Marker::Autolink, &["insert_link"], WordComplete, false),
    // -- LinePrefix -------------------------------------------------------
    row(Literal("######"), &["heading_6"], LinePrefix, false),
    row(Literal("#####"), &["heading_5"], LinePrefix, false),
    row(Literal("####"), &["heading_4"], LinePrefix, false),
    row(Literal("###"), &["heading_3"], LinePrefix, false),
    row(Literal("##"), &["heading_2"], LinePrefix, false),
    row(Literal("#"), &["heading_1"], LinePrefix, true),
    row(Literal("- [ ]"), &["task_list"], LinePrefix, false),
    row(Literal("* [ ]"), &["task_list"], LinePrefix, false),
    row(Literal("+ [ ]"), &["task_list"], LinePrefix, false),
    row(
        Literal("- [x]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("* [x]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("+ [x]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("- [X]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("* [X]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("+ [X]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(Literal("[ ]"), &["task_list"], LinePrefix, false),
    row(
        Literal("[x]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(
        Literal("[X]"),
        &["task_list", "toggle_task"],
        LinePrefix,
        false,
    ),
    row(Literal("-"), &["bullet_list"], LinePrefix, true),
    row(Literal("*"), &["bullet_list"], LinePrefix, true),
    row(Literal("+"), &["bullet_list"], LinePrefix, false),
    row(Ordered, &["numbered_list"], LinePrefix, true),
    row(Literal(">"), &["quote_block"], LinePrefix, true),
    // -- LineComplete -----------------------------------------------------
    row(Literal("==="), &["heading_1"], LineComplete, false),
    // `---` under one paragraph line is a setext heading, and alone in a
    // block a divider: the same marker, told apart by the line above.
    row(
        Literal("---"),
        &["heading_2", "divider"],
        LineComplete,
        false,
    ),
    row(Literal("***"), &["divider"], LineComplete, false),
    row(Literal("___"), &["divider"], LineComplete, false),
    row(Marker::Fence, &["code_block"], LineComplete, false),
    row(Marker::DelimiterRow, &["table"], LineComplete, false),
];

/// The rows today's WebKit bridge implements, as it spells them: the
/// marker string, the one command, and the trigger.
///
/// The bridge recognises `1.` for the ordered row, which is the one
/// instance of [`Marker::Ordered`] it knows.
pub fn bridged() -> impl Iterator<Item = (&'static str, &'static str, Trigger)> {
    SEQUENCES
        .iter()
        .filter(|sequence| sequence.bridged)
        .map(|sequence| {
            let marker = match sequence.marker {
                Literal(literal) => literal,
                Ordered => "1.",
                other => unreachable!("the bridge has no pattern {other:?}"),
            };
            let [command] = sequence.commands else {
                unreachable!("the bridge runs one command per row")
            };
            (marker, *command, sequence.trigger)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::{CommandId, Context, registry};

    #[test]
    fn every_sequence_reaches_a_formatting_command_and_nothing_else() {
        // FR-068. A sequence with no command behind it would make document
        // structure no other surface can produce or undo -- which is the
        // point at which "markdown input" has quietly become a dialect.
        // Every command a row names is registered, in the composer.
        for sequence in SEQUENCES {
            assert!(!sequence.commands.is_empty(), "{sequence:?}");
            for command in sequence.commands {
                let id: CommandId = command
                    .parse()
                    .unwrap_or_else(|_| panic!("{sequence:?} reaches `{command}`, no command"));
                assert!(
                    registry::get(id).contexts.contains(Context::Composer),
                    "{sequence:?} reaches `{command}`, which is not a composer command"
                );
            }
        }
    }

    #[test]
    fn the_reachable_set_is_the_formatting_set_without_duplicates() {
        // The bound FR-068 states, pinned as a list (spec 014 widened it).
        let mut commands: Vec<&str> = SEQUENCES
            .iter()
            .flat_map(|s| s.commands.iter().copied())
            .collect();
        commands.sort_unstable();
        commands.dedup();
        assert_eq!(
            commands,
            [
                "bold",
                "bullet_list",
                "code_block",
                "divider",
                "heading_1",
                "heading_2",
                "heading_3",
                "heading_4",
                "heading_5",
                "heading_6",
                "inline_code",
                "insert_link",
                "italic",
                "numbered_list",
                "quote_block",
                "strikethrough",
                "table",
                "task_list",
                "toggle_task",
            ],
            "the set of commands markdown input can reach has changed"
        );
    }

    #[test]
    fn six_hashes_make_six_headings_at_the_start_of_a_block() {
        let hashes: Vec<_> = SEQUENCES
            .iter()
            .filter_map(|s| Some((s.marker.literal()?, s.commands, s.trigger)))
            .filter(|(marker, _, _)| marker.starts_with('#'))
            .collect();
        assert_eq!(hashes.len(), 6);
        for (marker, commands, trigger) in hashes {
            assert_eq!(trigger, LinePrefix);
            assert_eq!(commands, [format!("heading_{}", marker.len()).as_str()]);
        }
    }

    #[test]
    fn longer_markers_are_listed_before_the_shorter_ones_they_contain() {
        // A recogniser that tries `*` first turns `**bold**` into an italic
        // run wrapping `*bold*`, and one that tries `-` first never sees
        // `- [ ]`. Across patterns too: a literal that is an instance of a
        // pattern must come before the pattern.
        for (index, sequence) in SEQUENCES.iter().enumerate() {
            for earlier in &SEQUENCES[..index] {
                if earlier.trigger != sequence.trigger {
                    continue;
                }
                if let (Some(later), Some(first)) =
                    (sequence.marker.literal(), earlier.marker.literal())
                {
                    assert!(
                        !later.starts_with(first),
                        "{later:?} is listed after {first:?}, which is a prefix of it"
                    );
                }
                if let Some(later) = sequence.marker.literal() {
                    assert!(
                        earlier.marker.literal().is_some() || !earlier.marker.matches(later),
                        "{later:?} is listed after the pattern {:?}, which matches it",
                        earlier.marker
                    );
                }
            }
        }
    }

    #[test]
    fn the_ordered_marker_is_digits_then_a_dot_or_a_paren() {
        for yes in ["1.", "7.", "7)", "123456789."] {
            assert!(Ordered.matches(yes), "{yes}");
        }
        for no in ["1", ".", "a.", "1234567890.", "1.)", "-1."] {
            assert!(!Ordered.matches(no), "{no}");
        }
    }

    #[test]
    fn a_fence_is_three_or_more_with_an_optional_language() {
        for yes in ["```", "~~~", "````", "```rust", "~~~ c++", "```objective-c"] {
            assert!(Marker::Fence.matches(yes), "{yes}");
        }
        for no in ["``", "~~", "```a b", "``` `x`", "--- ", "```\"x\""] {
            assert!(!Marker::Fence.matches(no), "{no}");
        }
    }

    #[test]
    fn a_delimiter_row_is_dashes_with_optional_colons() {
        for yes in ["|---|---|", "| :-- | --: |", "|:-:|", "--- | ---"] {
            assert!(Marker::DelimiterRow.matches(yes), "{yes}");
        }
        for no in ["| a | b |", "|", "---", "| -x- |", "| : |"] {
            assert!(!Marker::DelimiterRow.matches(no), "{no}");
        }
    }

    #[test]
    fn the_bridge_is_handed_the_rows_it_implements_and_no_more() {
        // Spec 014 R15: the table grew ahead of the rich surface, and
        // today's editor.js implements these eight. A row the bridge was
        // handed and has no recogniser for would consume a marker and
        // format nothing.
        let rows: Vec<_> = bridged().collect();
        assert_eq!(
            rows,
            [
                ("**", "bold", Wrapping),
                ("*", "italic", Wrapping),
                ("_", "italic", Wrapping),
                ("#", "heading_1", LinePrefix),
                ("-", "bullet_list", LinePrefix),
                ("*", "bullet_list", LinePrefix),
                ("1.", "numbered_list", LinePrefix),
                (">", "quote_block", LinePrefix),
            ]
        );
    }
}
