//! Passages: the few words around a match a result row shows, matched words marked. About
//! [`WINDOW`] characters snapped to word edges, whitespace collapsed, the first match a third of
//! the way in; ellipses are flags the surface draws.
//! With [`FirstLine::Shown`] the first line is never in the passage; ranges are byte ranges on
//! character boundaries by [`highlight::find`]'s token rule.

use std::ops::Range;

use crate::highlight;

pub const WINDOW: usize = 120;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passage {
    pub text: String,
    pub ranges: Vec<Range<usize>>,
    pub elided_start: bool,
    pub elided_end: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FirstLine {
    #[default]
    Any,
    Shown,
    /// Prefers a match after the first line, and falls back to the first line's own match rather
    /// than an unmarked window.
    Avoided,
}

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

    let chars: Vec<(usize, char)> = flat.char_indices().collect();
    let total = chars.len();
    let char_at = |byte: usize| chars.partition_point(|(offset, _)| *offset < byte);
    let space = |index: usize| chars[index].1.is_whitespace();
    let (hit_start, hit_end) = match &first {
        Some(range) => (char_at(range.start), char_at(range.end)),
        None => (0, 0),
    };

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
        let text = "Atlas budget, final numbers\nThanks for the atlas numbers, all good.";
        let passage = cut(text, &terms(&["atlas"]), FirstLine::Shown).expect("a match");
        assert_eq!(passage.text, "Thanks for the atlas numbers, all good.");
        assert_eq!(marked(&passage), vec!["atlas"]);
        assert!(!passage.elided_start, "it starts where the line does");

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
        let text = "Sharing the draft. Two more roles move the Atlas budget up by 9%.\n";
        let passage = cut(text, &terms(&["atlas", "budget"]), FirstLine::Avoided)
            .expect("the first line's match");
        assert_eq!(marked(&passage), vec!["Atlas", "budget"]);
        assert!(passage.text.contains("move the Atlas budget"));

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
