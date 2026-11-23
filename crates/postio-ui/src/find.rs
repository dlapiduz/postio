//! Find in the message on screen over the lines a frontend drew; toolkit-free so a terminal and a
//! window count the same.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub from: usize,
    pub to: usize,
}

/// Case-insensitive, within one line, non-overlapping; an empty query matches nothing.
pub fn matches<S: AsRef<str>>(lines: &[S], query: &str) -> Vec<Match> {
    let wanted = folded(query);
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        let text = folded(text.as_ref());
        let mut at = 0;
        while at + wanted.len() <= text.len() {
            if text[at..at + wanted.len()] == wanted[..] {
                found.push(Match {
                    line,
                    from: at,
                    to: at + wanted.len(),
                });
                at += wanted.len();
            } else {
                at += 1;
            }
        }
    }
    found
}

/// Lowercased per character so an index in the result is an index in `text`.
fn folded(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

pub fn first_from(found: &[Match], from_line: usize) -> Option<usize> {
    found
        .iter()
        .position(|found| found.line >= from_line)
        .or(if found.is_empty() { None } else { Some(0) })
}

pub fn step(current: Option<usize>, total: usize, forward: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => total - 1,
        (Some(at), true) => (at + 1) % total,
        (Some(at), false) => (at + total - 1) % total,
    })
}

pub fn count_label(query: &str, current: Option<usize>, total: usize) -> String {
    if query.is_empty() {
        String::new()
    } else if total == 0 {
        "No matches".to_owned()
    } else {
        format!(
            "{} of {total}",
            current.map_or(1, |at| at.min(total - 1) + 1)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_match_is_found_ignoring_case_line_by_line() {
        let lines = ["The harbor and the HARBOR", "no match here", "Harbor"];
        assert_eq!(
            matches(&lines, "harbor"),
            vec![
                Match {
                    line: 0,
                    from: 4,
                    to: 10
                },
                Match {
                    line: 0,
                    from: 19,
                    to: 25
                },
                Match {
                    line: 2,
                    from: 0,
                    to: 6
                },
            ]
        );
    }

    #[test]
    fn matches_do_not_overlap_and_an_empty_query_has_none() {
        assert_eq!(matches(&["aaaa"], "aa").len(), 2);
        assert!(matches(&["aaaa"], "").is_empty());
        assert!(matches(&["abc"], "abd").is_empty());
    }

    #[test]
    fn a_match_is_counted_in_characters_not_bytes() {
        let found = matches(&["Tomás Ñandú ñandú"], "ñandú");
        assert_eq!(
            found,
            vec![
                Match {
                    line: 0,
                    from: 6,
                    to: 11
                },
                Match {
                    line: 0,
                    from: 12,
                    to: 17
                },
            ]
        );
    }

    #[test]
    fn the_first_match_shown_is_the_one_where_the_person_is_reading() {
        let found = matches(&["a", "b a", "c", "a"], "a");
        assert_eq!(first_from(&found, 0), Some(0));
        assert_eq!(first_from(&found, 2), Some(2));
        assert_eq!(first_from(&found, 4), Some(0), "wraps to the first");
        assert_eq!(first_from(&[], 0), None);
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(Some(1), 3, true), Some(2));
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(None, 0, true), None);
    }

    #[test]
    fn the_field_says_which_match_of_how_many_or_that_there_are_none() {
        assert_eq!(count_label("", None, 0), "");
        assert_eq!(count_label("tide", None, 0), "No matches");
        assert_eq!(count_label("tide", Some(1), 5), "2 of 5");
        assert_eq!(count_label("tide", None, 5), "1 of 5");
    }
}
