//! Which typed sequences become which formatting command. Markdown is input only, never the draft's
//! format (ADR 0003).
//! The table lives here so both frontends agree; the transformation runs in the WebView script (16
//! ms budget).
//! Every sequence maps to formatting an existing command already produces.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// At the start of a block when the marker is followed by a space; marker and space are
    /// consumed.
    LinePrefix,
    /// When the closing marker is typed around a run on one line; both markers are consumed.
    Wrapping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// For `LinePrefix` the trailing space is implied; for `Wrapping` the marker on both sides.
    pub marker: &'static str,
    pub command: &'static str,
    pub trigger: Trigger,
}

/// Longest marker first within each trigger, so `**` is not shadowed by `*`.
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
        marker: "#",
        command: "heading_1",
        trigger: Trigger::LinePrefix,
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
        const FORMATTING: [&str; 6] = [
            "bold",
            "italic",
            "heading_1",
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
    fn a_hash_and_a_space_make_a_heading_and_a_hash_alone_does_not() {
        // Block-start only: a `#` elsewhere is a channel, issue number or hashtag.
        let hash: Vec<_> = SEQUENCES
            .iter()
            .filter(|s| s.marker.starts_with('#'))
            .collect();
        assert_eq!(
            hash,
            [&Sequence {
                marker: "#",
                command: "heading_1",
                trigger: Trigger::LinePrefix,
            }],
            "the heading sequences are not the one the heading command has"
        );
    }

    #[test]
    fn longer_markers_are_listed_before_the_shorter_ones_they_contain() {
        // `*` tried before `**` would turn `**bold**` into an italic run.
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
        let mut commands: Vec<&str> = SEQUENCES.iter().map(|s| s.command).collect();
        commands.sort_unstable();
        commands.dedup();
        assert_eq!(
            commands,
            [
                "bold",
                "bullet_list",
                "heading_1",
                "italic",
                "numbered_list",
                "quote_block"
            ],
            "the set of commands markdown input can reach has changed"
        );
    }
}
