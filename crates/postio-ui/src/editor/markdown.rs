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
//! there is no sequence for a table or a footnote because there is no command
//! for one, and adding a sequence without a command would create document
//! structure no other surface could make or undo.

/// Where a sequence is recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Recognised at the start of a block, when the marker is followed by a
    /// space — `# `, `- `, `> `. The marker and the space are consumed.
    LinePrefix,
    /// Recognised when the closing marker is typed around a run of text on
    /// one line — `**bold**`. Both markers are consumed.
    Wrapping,
}

/// One typed sequence and the command it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// What is typed. For [`Trigger::LinePrefix`] the trailing space is
    /// implied and not written here; for [`Trigger::Wrapping`] this is the
    /// marker that appears on both sides.
    pub marker: &'static str,
    /// The `postio-core` command id this produces, as the editing bridge
    /// already spells it in both directions.
    pub command: &'static str,
    /// Where it is recognised.
    pub trigger: Trigger,
}

/// Every supported sequence.
///
/// Ordered longest marker first within each trigger, because a recogniser
/// that checks `*` before `**` never sees `**` — the ordering is part of the
/// contract rather than an implementation detail of one frontend.
pub const SEQUENCES: &[Sequence] = &[
    Sequence {
        marker: "**",
        command: "bold",
        trigger: Trigger::Wrapping,
    },
    Sequence {
        marker: "*",
        command: "italic",
        trigger: Trigger::Wrapping,
    },
    Sequence {
        marker: "_",
        command: "italic",
        trigger: Trigger::Wrapping,
    },
    Sequence {
        marker: "-",
        command: "bullet_list",
        trigger: Trigger::LinePrefix,
    },
    Sequence {
        marker: "*",
        command: "bullet_list",
        trigger: Trigger::LinePrefix,
    },
    Sequence {
        marker: "1.",
        command: "numbered_list",
        trigger: Trigger::LinePrefix,
    },
    Sequence {
        marker: ">",
        command: "quote_block",
        trigger: Trigger::LinePrefix,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sequence_reaches_a_formatting_command_and_nothing_else() {
        // FR-068. A sequence with no command behind it would make document
        // structure no other surface can produce or undo -- which is the
        // point at which "markdown input" has quietly become a dialect.
        const FORMATTING: [&str; 5] = [
            "bold",
            "italic",
            "bullet_list",
            "numbered_list",
            "quote_block",
        ];
        for sequence in SEQUENCES {
            assert!(
                FORMATTING.contains(&sequence.command),
                "{:?} reaches {:?}, which is not one of the editor's \
                 formatting commands",
                sequence.marker,
                sequence.command
            );
        }
    }

    #[test]
    fn no_heading_sequence_until_there_is_a_heading_command() {
        // `# ` is the sequence people reach for first and it is deliberately
        // absent: the composer's toolbar and the registry have no heading
        // verb, so `# ` would be the one way to make an `<h1>` in a draft --
        // unreachable from the palette, unremovable by any control, and
        // invisible to the `?` sheet. When a heading command exists this
        // table grows a row and this test changes with it.
        assert!(
            !SEQUENCES.iter().any(|s| s.marker.starts_with('#')),
            "a heading sequence arrived without a heading command"
        );
    }

    #[test]
    fn longer_markers_are_listed_before_the_shorter_ones_they_contain() {
        // A recogniser that tries `*` first turns `**bold**` into an italic
        // run wrapping `*bold*`. The ordering is the contract, not an
        // implementation detail, because every frontend walks this table.
        for (index, sequence) in SEQUENCES.iter().enumerate() {
            for earlier in &SEQUENCES[..index] {
                if earlier.trigger != sequence.trigger {
                    continue;
                }
                assert!(
                    !sequence.marker.starts_with(earlier.marker),
                    "{:?} is listed after {:?}, which is a prefix of it, so \
                     the shorter marker would always match first",
                    sequence.marker,
                    earlier.marker
                );
            }
        }
    }

    #[test]
    fn the_reachable_set_is_the_formatting_set_without_duplicates() {
        // The bound FR-068 states, pinned as a list. A sequence added for a
        // command that is not in here is exactly the change this makes
        // visible.
        let mut commands: Vec<&str> = SEQUENCES.iter().map(|s| s.command).collect();
        commands.sort_unstable();
        commands.dedup();
        assert_eq!(
            commands,
            [
                "bold",
                "bullet_list",
                "italic",
                "numbered_list",
                "quote_block"
            ],
            "the set of commands markdown input can reach has changed"
        );
    }
}
