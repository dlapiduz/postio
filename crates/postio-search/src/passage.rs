//! Passages: the few words around a match that a result row shows.
//!
//! A results row shows, under its subject, about a line's worth of the text
//! that matched, with the matched words marked (spec 010, D7). [`cut`] makes
//! it from the text the index was built from:
//!
//! * **About [`WINDOW`] characters**, counted as characters rather than
//!   bytes, with the first match a third of the way in so the reader meets
//!   it with some lead-in and reads on past it.
//! * **Word edges**: the window starts and ends between whitespace-separated
//!   words, never inside one, and whitespace is collapsed to single spaces
//!   (a row is one line).
//! * **Ellipses are flags**, not characters: the surface draws them, and the
//!   ranges stay ranges into exactly the text it draws.
//! * **Never the first line**, when the row already shows it as the preview
//!   ([`FirstLine::Shown`]): the passage is cut around the first match
//!   *after* it, and when the only match is in the first line, it is the
//!   window that follows it. A row with no preview ([`FirstLine::Avoided`])
//!   still prefers a match after it, and falls back to the first line's own
//!   match rather than to a window with nothing marked.
//! * **Ranges land on the matched words** by [`highlight::find`]'s token
//!   rule, the index's own, as byte ranges into [`Passage::text`] -- always on
//!   character boundaries, whatever script the text is in.

use std::ops::Range;

use crate::highlight;

/// About how many characters a passage holds.
pub const WINDOW: usize = 120;

/// The words around a match, ready to draw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passage {
    /// About [`WINDOW`] characters, snapped to word edges, on one line.
    pub text: String,
    /// Byte ranges into `text` to highlight, in order, never overlapping.
    pub ranges: Vec<Range<usize>>,
    /// Text was cut before `text`: draw an ellipsis in front.
    pub elided_start: bool,
    /// Text was cut after `text`: draw an ellipsis behind.
    pub elided_end: bool,
}

/// What a passage does with its text's first line (D7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FirstLine {
    /// Cut around the first match, wherever it is: the text opens with no
    /// line the row shows (quoted history under the person's own words).
    #[default]
    Any,
    /// The row shows the first line as its preview, as a list row does:
    /// never in the passage; the window after it when it holds the only
    /// match.
    Shown,
    /// The row shows no preview, as the dropdown's hits and the results'
    /// rows do: around the first match after the first line, or around the
    /// first line's own match when it holds the only one -- a passage that
    /// marks what matched rather than an unmarked one, or none.
    Avoided,
}

/// The passage of `text` around the first place `terms` match, or `None`
/// when they match nowhere in it (or, with [`FirstLine::Shown`], only in a
/// first line with nothing after it).
///
/// `first_line` is what the row does with the message's first line (D7).
/// See the module docs for the rules.
pub fn cut(text: &str, terms: &[String], first_line: FirstLine) -> Option<Passage> {
    if highlight::find(text, terms).is_empty() {
        return None;
    }
    let region = match first_line {
        FirstLine::Any => text,
        FirstLine::Shown => after_first_line(text),
        FirstLine::Avoided => {
            let after = after_first_line(text);
            if highlight::find(after, terms).is_empty() {
                text
            } else {
                after
            }
        }
    };
    let flat = highlight::collapse_whitespace(region);
    if flat.is_empty() {
        return None;
    }
    let first = highlight::find(&flat, terms).into_iter().next();

    // Characters, with their byte offsets: the window is counted in the one
    // and cut in the other.
    let chars: Vec<(usize, char)> = flat.char_indices().collect();
    let total = chars.len();
    let char_at = |byte: usize| chars.partition_point(|(offset, _)| *offset < byte);
    let space = |index: usize| chars[index].1.is_whitespace();
    let (hit_start, hit_end) = match &first {
        Some(range) => (char_at(range.start), char_at(range.end)),
        None => (0, 0),
    };

    // Start a third of the room before the match, then onto a word's start:
    // forward when that does not pass the match, else back.
    let room = WINDOW.saturating_sub(hit_end - hit_start);
    let mut start = hit_start.saturating_sub(room / 3);
    if start + WINDOW > total {
        start = total.saturating_sub(WINDOW).min(hit_start);
    }
    if start > 0 && !space(start - 1) {
        let forward = (start..hit_start).find(|&i| space(i));
        match forward {
            Some(gap) => start = gap,
            None => {
                while start > 0 && !space(start - 1) {
                    start -= 1;
                }
            }
        }
    }
    while start < total && space(start) {
        start += 1;
    }

    // End a window on, then back onto a word's end: back when that does not
    // cut into the match, else forward.
    let mut end = (start + WINDOW).max(hit_end).min(total);
    if end < total && !space(end) {
        let back = (hit_end.max(start)..end).rev().find(|&i| space(i));
        match back {
            Some(gap) => end = gap,
            None => {
                while end < total && !space(end) {
                    end += 1;
                }
            }
        }
    }
    while end > start && space(end - 1) {
        end -= 1;
    }

    let byte = |index: usize| chars.get(index).map_or(flat.len(), |(offset, _)| *offset);
    let text = flat[byte(start)..byte(end)].to_owned();
    Some(Passage {
        ranges: highlight::find(&text, terms),
        elided_start: start > 0,
        elided_end: end < total,
        text,
    })
}

/// Everything after the first line that has anything on it.
fn after_first_line(text: &str) -> &str {
    let mut rest = text;
    while let Some((line, after)) = rest.split_once('\n') {
        if !line.trim().is_empty() {
            return after;
        }
        rest = after;
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// Every highlighted range, as the text it covers.
    fn marked(passage: &Passage) -> Vec<&str> {
        passage
            .ranges
            .iter()
            .map(|range| &passage.text[range.clone()])
            .collect()
    }

    const LONG: &str = "Morning all. Following up on the planning meeting from last \
        Tuesday, where we went through every line of the forecast together and agreed \
        that the atlas budget needs one more pass before it goes to finance for their \
        review, which they want by the end of next week at the latest, so please send \
        me your numbers soon and flag anything that looks off to you.";

    #[test]
    fn a_window_of_about_120_characters_snapped_to_word_edges() {
        let passage = cut(LONG, &terms(&["budget"]), FirstLine::Any).expect("a match");
        let chars = passage.text.chars().count();
        assert!(
            (100..=WINDOW + 10).contains(&chars),
            "{chars} characters: {:?}",
            passage.text
        );
        assert_eq!(marked(&passage), vec!["budget"]);
        assert!(passage.elided_start && passage.elided_end, "{passage:?}");

        // Word edges: the passage is a run of whole words of the text.
        let at = LONG.find(&passage.text).expect("a slice of the text");
        let before = LONG[..at].chars().next_back();
        let after = LONG[at + passage.text.len()..].chars().next();
        assert!(before.is_none_or(char::is_whitespace), "{passage:?}");
        assert!(after.is_none_or(char::is_whitespace), "{passage:?}");
        assert!(!passage.text.starts_with(' ') && !passage.text.ends_with(' '));
    }

    #[test]
    fn a_short_text_is_the_whole_passage_with_nothing_elided() {
        let passage = cut(
            "The atlas budget, final.",
            &terms(&["atlas", "final"]),
            FirstLine::Any,
        )
        .expect("a match");
        assert_eq!(passage.text, "The atlas budget, final.");
        assert_eq!(marked(&passage), vec!["atlas", "final"]);
        assert!(!passage.elided_start && !passage.elided_end);
    }

    #[test]
    fn lines_are_one_line_in_a_passage() {
        let passage = cut(
            "Hi Ada,\n\nthe atlas\n   budget is attached.\n",
            &terms(&["budget"]),
            FirstLine::Any,
        )
        .expect("a match");
        assert_eq!(passage.text, "Hi Ada, the atlas budget is attached.");
        assert_eq!(marked(&passage), vec!["budget"]);
    }

    #[test]
    fn never_the_first_line_when_it_is_shown_already() {
        // D7: the row's preview is the first line, so the passage is the
        // first match after it.
        let text = "Atlas budget, final numbers\nThanks for the atlas numbers, all good.";
        let passage = cut(text, &terms(&["atlas"]), FirstLine::Shown).expect("a match");
        assert_eq!(passage.text, "Thanks for the atlas numbers, all good.");
        assert_eq!(marked(&passage), vec!["atlas"]);
        assert!(!passage.elided_start, "it starts where the line does");

        // Unless asked to show it.
        let passage = cut(text, &terms(&["atlas"]), FirstLine::Any).expect("a match");
        assert!(passage.text.starts_with("Atlas budget"));
        assert_eq!(marked(&passage), vec!["Atlas", "atlas"]);
    }

    #[test]
    fn a_match_only_in_the_first_line_gives_the_window_after_it() {
        let text = "\n  Atlas kickoff notes\nWe met on Tuesday and agreed the plan.\nMore later.";
        let passage = cut(text, &terms(&["atlas"]), FirstLine::Shown).expect("the window after");
        assert_eq!(
            passage.text,
            "We met on Tuesday and agreed the plan. More later."
        );
        assert!(passage.ranges.is_empty(), "nothing in it matched");
        assert!(!passage.text.contains("Atlas"));

        assert_eq!(
            cut("Atlas kickoff notes", &terms(&["atlas"]), FirstLine::Shown),
            None,
            "a first line and nothing after it has no passage to show"
        );
    }

    #[test]
    fn a_row_with_no_preview_still_marks_a_match_only_in_the_first_line() {
        // The results view draws no preview: a one-line message whose match
        // is in that line gets the window around it, marked.
        let text = "Sharing the draft. Two more roles move the Atlas budget up by 9%.\n";
        let passage = cut(text, &terms(&["atlas", "budget"]), FirstLine::Avoided)
            .expect("the first line's match");
        assert_eq!(marked(&passage), vec!["Atlas", "budget"]);
        assert!(passage.text.contains("move the Atlas budget"));

        // A match after the first line is still preferred.
        let text = "Atlas budget, final numbers\nThanks for the atlas numbers, all good.";
        let passage = cut(text, &terms(&["atlas"]), FirstLine::Avoided).expect("a match");
        assert_eq!(passage.text, "Thanks for the atlas numbers, all good.");
        assert_eq!(marked(&passage), vec!["atlas"]);
    }

    #[test]
    fn ranges_land_on_the_matched_words_in_multibyte_text() {
        let text = "Grüße aus Zürich — 東京の会議 🎉 über das Atlas-Budget für Müller, \
            mit naïve Schätzungen und café-Notizen, die wir noch einmal durchgehen \
            müssen, bevor alles an die Finanzabteilung geht und dort geprüft wird.";
        let passage = cut(text, &terms(&["atlas", "müller"]), FirstLine::Any).expect("a match");
        assert_eq!(marked(&passage), vec!["Atlas", "Müller"]);
        for range in &passage.ranges {
            assert!(passage.text.is_char_boundary(range.start));
            assert!(passage.text.is_char_boundary(range.end));
        }
        assert!(passage.text.chars().count() <= WINDOW + 10);
    }

    #[test]
    fn a_long_window_into_multibyte_text_cuts_on_characters() {
        let text = "ä ".repeat(200) + "atlas " + &"ö ".repeat(200);
        let passage = cut(&text, &terms(&["atlas"]), FirstLine::Any).expect("a match");
        assert_eq!(marked(&passage), vec!["atlas"]);
        assert!(passage.elided_start && passage.elided_end);
        assert!((100..=WINDOW + 10).contains(&passage.text.chars().count()));
    }

    #[test]
    fn no_match_is_no_passage() {
        assert_eq!(cut(LONG, &terms(&["harbor"]), FirstLine::Any), None);
        assert_eq!(cut("", &terms(&["harbor"]), FirstLine::Any), None);
        assert_eq!(cut(LONG, &[], FirstLine::Any), None);
    }
}
