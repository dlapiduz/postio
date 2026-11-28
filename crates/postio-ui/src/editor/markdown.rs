//! Which typed sequences become which formatting command: Markdown as input, never the draft's
//! format (ADR 0003).
//! The table lives here so every frontend recognises the same set; each sequence maps to an
//! existing command, which keeps this from growing a dialect.
//! The bridge implements only the rows marked `bridged`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    LinePrefix,
    Wrapping,
    LineComplete,
    WordComplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Literal(&'static str),
    Ordered,
    Fence,
    DelimiterRow,
    BacktickRun,
    Link,
    AngleLink,
    Autolink,
}

impl Marker {
    /// The wrapping patterns and autolinks are matched over a run of text, so only their delimiters
    /// are checked.
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

    pub fn literal(self) -> Option<&'static str> {
        match self {
            Marker::Literal(literal) => Some(literal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// For `LinePrefix` the trailing space is implied; for `Wrapping` a literal is the marker on
    /// both sides.
    pub marker: Marker,
    /// In the order they apply (`***` is bold, then italic).
    pub commands: &'static [&'static str],
    pub trigger: Trigger,
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
use postio_body::{Conversion, ConversionKind, HeadingLevel, Href, ListKind, Mark, Task};

/// Longest marker first within each trigger: a recogniser checking `*` before `**` never sees `**`.
pub const SEQUENCES: &[Sequence] = &[
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
    row(Marker::Autolink, &["insert_link"], WordComplete, false),
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
    row(Literal("==="), &["heading_1"], LineComplete, false),
    // `---` under one paragraph line is a setext heading, alone in a block a divider.
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

/// The bridge recognises `1.` for the ordered row.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    Text,
    Enter,
}

pub fn conversion_before(block_text: &str, caret: usize, typed: Typed) -> Option<Conversion> {
    let before: String = block_text
        .chars()
        .take(caret)
        .map(|c| match c {
            '\u{a0}' => ' ',
            '\u{2028}' => '\n',
            other => other,
        })
        .collect();
    let complete = caret >= block_text.chars().count();
    SEQUENCES
        .iter()
        .find_map(|sequence| match (sequence.trigger, typed) {
            (LinePrefix, Typed::Text) => line_prefix(sequence, &before),
            (Wrapping, Typed::Text) => wrapping(sequence, &before),
            (LineComplete, Typed::Enter) if complete => line_complete(sequence, &before),
            _ => None,
        })
}

fn chars(text: &str) -> u32 {
    text.chars().count() as u32
}

fn line_prefix(sequence: &Sequence, before: &str) -> Option<Conversion> {
    let marker = before.strip_suffix(' ')?;
    if !sequence.marker.matches(marker) {
        return None;
    }
    let kind = match sequence.commands {
        ["bullet_list"] => ConversionKind::List(ListKind::Bullet),
        ["numbered_list"] => {
            let digits = &marker[..marker.len() - 1];
            ConversionKind::List(ListKind::Ordered {
                start: digits.parse().ok()?,
            })
        }
        ["quote_block"] => ConversionKind::Quote,
        ["task_list"] => ConversionKind::Task(Task::Open),
        ["task_list", "toggle_task"] => ConversionKind::Task(Task::Done),
        [heading] => ConversionKind::Heading(heading_level(heading)?),
        _ => return None,
    };
    Some(Conversion {
        trigger_len: chars(before),
        kind,
    })
}

fn heading_level(command: &str) -> Option<HeadingLevel> {
    HeadingLevel::from_digit(command.strip_prefix("heading_")?.parse().ok()?)
}

fn wrapping(sequence: &Sequence, before: &str) -> Option<Conversion> {
    match sequence.marker {
        Literal(marker) => {
            let marks = sequence
                .commands
                .iter()
                .map(|command| match *command {
                    "bold" => Some(Mark::Strong),
                    "italic" => Some(Mark::Emphasis),
                    "strikethrough" => Some(Mark::Strikethrough),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            wrapped(before, marker, marks)
        }
        Marker::BacktickRun => {
            let run = before.chars().rev().take_while(|c| *c == '`').count();
            wrapped(before, &"`".repeat(run.max(1)), vec![Mark::Code])
        }
        Marker::Link => {
            let inner = before.strip_suffix(')')?;
            let middle = inner.rfind("](")?;
            let href = Href::parse(&inner[middle + 2..])?;
            let open = inner[..middle].rfind('[')?;
            let label = &inner[open + 1..middle];
            if label.is_empty() || label.contains(']') {
                return None;
            }
            let trigger = &before[open..];
            Some(Conversion {
                trigger_len: chars(trigger),
                kind: ConversionKind::Link {
                    href,
                    label: (1, chars(label)),
                },
            })
        }
        Marker::AngleLink => {
            let inner = before.strip_suffix('>')?;
            let open = inner.rfind('<')?;
            let address = &inner[open + 1..];
            if address.is_empty() || address.contains(char::is_whitespace) || !address.contains(':')
            {
                return None;
            }
            let href = Href::parse(address)?;
            Some(Conversion {
                trigger_len: chars(&before[open..]),
                kind: ConversionKind::Link {
                    href,
                    label: (1, chars(address)),
                },
            })
        }
        _ => None,
    }
}

fn wrapped(before: &str, marker: &str, marks: Vec<Mark>) -> Option<Conversion> {
    let character = marker.chars().next()?;
    let width = marker.chars().count();
    let inner = before.strip_suffix(marker)?;
    if inner.ends_with(character) {
        return None;
    }
    let mut search = inner;
    let open = loop {
        let at = search.rfind(marker)?;
        let run_before = inner[..at]
            .chars()
            .rev()
            .take_while(|c| *c == character)
            .count();
        let run_after = inner[at + marker.len()..]
            .chars()
            .take_while(|c| *c == character)
            .count();
        if run_before == 0 && run_after == 0 {
            break at;
        }
        search = &inner[..at];
    };
    let content = &inner[open + marker.len()..];
    if content.is_empty() || content.contains(marker) {
        return None;
    }
    Some(Conversion {
        trigger_len: chars(content) + 2 * width as u32,
        kind: ConversionKind::Wrap {
            marks,
            marker_len: width as u32,
        },
    })
}

fn line_complete(sequence: &Sequence, before: &str) -> Option<Conversion> {
    let (above, line) = match before.rsplit_once('\n') {
        Some((above, line)) => (Some(above), line),
        None => (None, before),
    };
    if !sequence.marker.matches(line) {
        return None;
    }
    let lone = above.is_none();
    let kind = match (sequence.commands, lone) {
        (["heading_1"], false) => ConversionKind::Setext(HeadingLevel::One),
        (["heading_2", "divider"], false) => ConversionKind::Setext(HeadingLevel::Two),
        (["heading_2", "divider"] | ["divider"], true) => ConversionKind::Rule,
        (["code_block"], true) => {
            let info = line.trim_start_matches(['`', '~']).trim();
            ConversionKind::CodeBlock(postio_body::Language::parse(info))
        }
        _ => return None,
    };
    let trigger_len = match kind {
        ConversionKind::Setext(_) => chars(line) + 1,
        _ => chars(line),
    };
    Some(Conversion { trigger_len, kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::{CommandId, Context, registry};

    fn names(kind: &ConversionKind, commands: &[&str]) -> bool {
        let mark = |mark: &Mark| match mark {
            Mark::Strong => "bold",
            Mark::Emphasis => "italic",
            Mark::Strikethrough => "strikethrough",
            Mark::Code => "inline_code",
        };
        match kind {
            ConversionKind::Heading(level) => commands == [format!("heading_{}", level.digit())],
            ConversionKind::List(ListKind::Bullet) => commands == ["bullet_list"],
            ConversionKind::List(ListKind::Ordered { .. }) => commands == ["numbered_list"],
            ConversionKind::Task(Task::Open) => commands == ["task_list"],
            ConversionKind::Task(Task::Done) => commands == ["task_list", "toggle_task"],
            ConversionKind::Quote => commands == ["quote_block"],
            ConversionKind::Rule => commands.contains(&"divider"),
            ConversionKind::Setext(level) => {
                commands.contains(&format!("heading_{}", level.digit()).as_str())
            }
            ConversionKind::CodeBlock(_) => commands == ["code_block"],
            ConversionKind::Wrap { marks, .. } => {
                marks.iter().map(mark).collect::<Vec<_>>() == commands
            }
            ConversionKind::Link { .. } => commands == ["insert_link"],
        }
    }

    fn later(sequence: &Sequence) -> bool {
        matches!(sequence.marker, Marker::Autolink | Marker::DelimiterRow)
    }

    fn example(sequence: &Sequence) -> (String, Typed) {
        match (sequence.marker, sequence.trigger) {
            (Literal(marker), Wrapping) => (format!("{marker}word{marker}"), Typed::Text),
            (Literal(marker), LinePrefix) => (format!("{marker} "), Typed::Text),
            (Ordered, _) => ("3. ".to_owned(), Typed::Text),
            (Literal(marker), LineComplete) if sequence.commands[0].starts_with("heading") => {
                (format!("Title\n{marker}"), Typed::Enter)
            }
            (Literal(marker), LineComplete) => (marker.to_owned(), Typed::Enter),
            (Marker::Fence, _) => ("```rust".to_owned(), Typed::Enter),
            (Marker::DelimiterRow, _) => ("|---|---|".to_owned(), Typed::Enter),
            (Marker::BacktickRun, _) => ("`f()`".to_owned(), Typed::Text),
            (Marker::Link, _) => ("[site](https://example.com/)".to_owned(), Typed::Text),
            (Marker::AngleLink, _) => ("<https://example.com/>".to_owned(), Typed::Text),
            (Marker::Autolink, _) => ("https://example.com/ ".to_owned(), Typed::Text),
            (marker, trigger) => panic!("no example for {marker:?} {trigger:?}"),
        }
    }

    #[test]
    fn every_row_of_the_table_converts_to_what_it_names() {
        for sequence in SEQUENCES {
            let (text, typed) = example(sequence);
            let caret = text.chars().count();
            match conversion_before(&text, caret, typed) {
                Some(conversion) => {
                    assert!(
                        names(&conversion.kind, sequence.commands),
                        "{text:?} became {conversion:?}, not {:?}",
                        sequence.commands
                    );
                    assert!(conversion.trigger_len as usize <= caret);
                }
                None => assert!(later(sequence), "{text:?} ({sequence:?}) converted nothing"),
            }
        }
    }

    fn kind(text: &str, typed: Typed) -> Option<ConversionKind> {
        conversion_before(text, text.chars().count(), typed).map(|c| c.kind)
    }

    #[test]
    fn the_longest_marker_wins_and_a_half_closed_one_waits() {
        assert_eq!(
            kind("**x**", Typed::Text),
            Some(ConversionKind::Wrap {
                marks: vec![Mark::Strong],
                marker_len: 2
            })
        );
        assert_eq!(
            kind("***x***", Typed::Text),
            Some(ConversionKind::Wrap {
                marks: vec![Mark::Strong, Mark::Emphasis],
                marker_len: 3
            })
        );
        assert_eq!(
            kind("**loudly*", Typed::Text),
            None,
            "the first of two closing asterisks"
        );
        assert_eq!(
            kind("****", Typed::Text),
            None,
            "four asterisks are not a request"
        );
    }

    #[test]
    fn a_wrapping_trigger_is_only_the_wrapped_words() {
        let conversion = conversion_before("say **hi**", 10, Typed::Text).unwrap();
        assert_eq!(conversion.trigger_len, 6);
    }

    #[test]
    fn a_line_prefix_is_only_at_the_start_of_its_block() {
        assert_eq!(kind("a - ", Typed::Text), None);
        assert_eq!(
            kind("-\u{a0}", Typed::Text),
            Some(ConversionKind::List(ListKind::Bullet))
        );
        assert_eq!(
            kind("7) ", Typed::Text),
            Some(ConversionKind::List(ListKind::Ordered { start: 7 })),
            "the number is the list's start"
        );
    }

    #[test]
    fn a_line_complete_sequence_waits_for_enter_at_the_end_of_its_line() {
        assert_eq!(kind("---", Typed::Text), None);
        assert_eq!(kind("---", Typed::Enter), Some(ConversionKind::Rule));
        assert_eq!(
            kind("Title\u{2028}---", Typed::Enter),
            Some(ConversionKind::Setext(HeadingLevel::Two))
        );
        assert_eq!(
            conversion_before("---x", 3, Typed::Enter),
            None,
            "the caret is not at the end"
        );
        assert_eq!(
            kind("```rust", Typed::Enter),
            Some(ConversionKind::CodeBlock(postio_body::Language::parse(
                "rust"
            )))
        );
    }

    #[test]
    fn a_link_names_its_label_inside_the_trigger() {
        let conversion =
            conversion_before("see [the site](https://example.com/)", 36, Typed::Text).unwrap();
        assert_eq!(conversion.trigger_len, 32);
        assert_eq!(
            conversion.kind,
            ConversionKind::Link {
                href: Href::parse("https://example.com/").unwrap(),
                label: (1, 8)
            }
        );
    }

    #[test]
    fn every_sequence_reaches_a_formatting_command_and_nothing_else() {
        // A sequence with no command behind it would make document structure no other surface can
        // produce or undo.
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
        // A recogniser trying `*` first turns `**bold**` into italic around `*bold*`; a literal
        // that is an instance of a pattern must precede the pattern.
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
        // The table grew ahead of the rich surface; a row the bridge has no recogniser for would
        // consume a marker and format nothing.
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
