//! Spike: can a sentence be found again in the rendered body from nothing but its excerpt? Measures
//! `TextIndex::locate` over the render corpus and fails if a rate falls below its floor.
//!
//! POSTIO-MEASUREMENT: its output is a table a person reads, and it renders
//! the whole corpus three times over (about ninety seconds in a debug
//! build), so it runs nightly (`--profile nightly`), where its floors still
//! hold the rates below.

// The render suite's shared fonts and requests: this is a binary of its
// own, so the nightly can run it apart (`POSTIO-MEASUREMENT`).
#[path = "render_suite/support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::ops::Range;

use postio_body::RemoteImages;
use postio_model::{MessageBody, test_corpus};
use postio_render::{RenderedDocument, TextIndex};
use postio_ui::reader::document::{self, Rendering};

struct Excerpt {
    text: String,
    offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum How {
    AsRead,
    SpacesCollapsed,
    BothCollapsed,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Which {
    Only,
    Right,
    Wrong,
    Uncounted,
    Unresolved,
}

type Outcome = (How, Which);

fn located(how: How, which: Which, hows: &[How]) -> bool {
    hows.contains(&how) && matches!(which, Which::Only | Which::Right)
}

fn sources(body: &MessageBody) -> [(&'static str, Option<String>); 2] {
    let text = body.text.clone().filter(|text| !text.trim().is_empty());
    let html = body
        .html
        .as_deref()
        .map(|html| postio_body::parse(html).to_search_text())
        .filter(|text| !text.trim().is_empty());
    [
        ("text first", text.clone().or_else(|| html.clone())),
        ("what is drawn", html.or(text)),
    ]
    .map(|(label, source)| (label, source.map(|source| source.replace("\r\n", "\n"))))
}

fn excerpts(source: &str) -> Vec<Excerpt> {
    let mut out = Vec::new();
    let mut sentence = String::new();
    let mut start = 0;
    let mut offset = 0;
    let flush = |sentence: &mut String, start: usize, out: &mut Vec<Excerpt>| {
        let trimmed = sentence.trim();
        let leading = sentence.chars().count() - sentence.trim_start().chars().count();
        if trimmed.split_whitespace().count() >= 4 && trimmed.chars().count() >= 20 {
            out.push(Excerpt {
                text: trimmed.to_owned(),
                offset: start + leading,
            });
        }
        sentence.clear();
    };
    for line in source.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if bare == "-- " || bare == "--" {
            break;
        }
        let quoted = bare.trim_start().starts_with('>');
        if quoted || bare.trim().is_empty() {
            flush(&mut sentence, start, &mut out);
            offset += line.chars().count();
            start = offset;
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if sentence.is_empty() {
                start = offset;
            }
            sentence.push(c);
            offset += 1;
            let ends = matches!(c, '.' | '?' | '!')
                && chars.peek().is_none_or(|next| next.is_whitespace());
            if ends {
                flush(&mut sentence, start, &mut out);
            }
        }
    }
    flush(&mut sentence, start, &mut out);
    out
}

fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    use unicode_normalization::char::is_combining_mark;
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ordinal_in_source(source: &str, excerpt: &Excerpt) -> (usize, usize) {
    let folded = fold(source);
    let needle = fold(&excerpt.text);
    let mut count = 0;
    let mut ordinal = 0;
    let mut from = 0;
    while let Some(found) = folded[from..].find(&needle) {
        let at = from + found;
        if folded[..at].chars().count() <= excerpt.offset {
            ordinal = count;
        }
        count += 1;
        from = at + needle.len().max(1);
    }
    (count.max(1), ordinal)
}

fn nearest(matches: &[Range<usize>], index: &TextIndex, excerpt: &Excerpt, source: &str) -> usize {
    let want = excerpt.offset as f64 / source.chars().count().max(1) as f64;
    let index_len = index.text.chars().count().max(1) as f64;
    let place = |range: &Range<usize>| range.start as f64 / index_len;
    (0..matches.len())
        .min_by(|a, b| {
            (place(&matches[*a]) - want)
                .abs()
                .total_cmp(&(place(&matches[*b]) - want).abs())
        })
        .expect("at least two matches")
}

fn locate(index: &TextIndex, source: &str, excerpt: &Excerpt) -> Outcome {
    for (how, needle) in [
        (How::AsRead, excerpt.text.clone()),
        (How::SpacesCollapsed, collapse(&excerpt.text)),
    ] {
        let matches = index.find(&needle);
        let which = match matches.len() {
            0 => continue,
            1 => Which::Only,
            found => {
                let (count, ordinal) = ordinal_in_source(source, excerpt);
                if count != found {
                    Which::Uncounted
                } else if nearest(&matches, index, excerpt, source) == ordinal {
                    Which::Right
                } else {
                    Which::Wrong
                }
            }
        };
        return (how, which);
    }
    let how = if collapse(&fold(&index.text)).contains(&collapse(&fold(&excerpt.text))) {
        How::BothCollapsed
    } else {
        How::Absent
    };
    (how, Which::Unresolved)
}

fn by_the_locator(index: &TextIndex, source: &str, excerpt: &Excerpt) -> Option<Range<usize>> {
    index.locate(postio_render::Excerpt {
        text: &excerpt.text,
        offset: excerpt.offset,
        source_len: source.chars().count(),
    })
}

#[derive(Default)]
struct Tally {
    outcomes: BTreeMap<Outcome, usize>,
    locator: BTreeMap<Which, usize>,
    misses: Vec<String>,
}

impl Tally {
    fn add(
        &mut self,
        label: &str,
        fixture: &str,
        excerpt: &Excerpt,
        outcome: Outcome,
        locator: Which,
    ) {
        *self.outcomes.entry(outcome).or_default() += 1;
        *self.locator.entry(locator).or_default() += 1;
        let (how, which) = outcome;
        if !located(how, which, &[How::AsRead]) {
            self.misses.push(format!(
                "{label:>13} | {fixture:<36} | {how:?}/{which:?} | {:?}",
                excerpt.text.chars().take(64).collect::<String>()
            ));
        }
    }

    fn total(&self) -> usize {
        self.outcomes.values().sum()
    }

    fn rate(&self, hows: &[How]) -> f64 {
        let located: usize = self
            .outcomes
            .iter()
            .filter(|((how, which), _)| located(*how, *which, hows))
            .map(|(_, count)| count)
            .sum();
        located as f64 / self.total().max(1) as f64
    }

    fn tiebreak(&self) -> (usize, usize) {
        let count = |wanted: Which| -> usize {
            self.outcomes
                .iter()
                .filter(|((_, which), _)| *which == wanted)
                .map(|(_, count)| count)
                .sum()
        };
        let right = count(Which::Right);
        (right, right + count(Which::Wrong))
    }

    fn by_the_locator(&self) -> f64 {
        let located: usize = [Which::Only, Which::Right]
            .iter()
            .filter_map(|which| self.locator.get(which))
            .sum();
        located as f64 / self.total().max(1) as f64
    }

    fn locator_tiebreak(&self) -> (usize, usize) {
        let count = |wanted: Which| self.locator.get(&wanted).copied().unwrap_or(0);
        let right = count(Which::Right);
        (right, right + count(Which::Wrong))
    }

    fn present(&self) -> f64 {
        let present: usize = self
            .outcomes
            .iter()
            .filter(|((how, _), _)| *how != How::Absent)
            .map(|(_, count)| count)
            .sum();
        present as f64 / self.total().max(1) as f64
    }

    fn print(&self, label: &str) {
        println!(
            "{label}: {} sentences; located as read {:.1}%, with the excerpt's \
             spaces collapsed {:.1}%, present at all {:.1}%",
            self.total(),
            self.rate(&[How::AsRead]) * 100.0,
            self.rate(&[How::AsRead, How::SpacesCollapsed]) * 100.0,
            self.present() * 100.0,
        );
        for ((how, which), count) in &self.outcomes {
            println!("    {how:?}/{which:?}: {count}");
        }
        println!(
            "  by the locator (TextIndex::locate): {:.1}%",
            self.by_the_locator() * 100.0
        );
        for (which, count) in &self.locator {
            println!("    {which:?}: {count}");
        }
    }
}

fn render(body: &MessageBody) -> Option<RenderedDocument> {
    let rendered = document::body_html_in(body, RemoteImages::Blocked, Rendering::Original, None);
    if rendered.over_cap.is_some() {
        return None;
    }
    let html = document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        document::sheet_for(Rendering::Original, false),
    );
    Some(support::render(&support::request_for(html, support::LIGHT)))
}

#[test]
fn a_sentence_is_found_again_from_its_excerpt() {
    let mut tallies = [
        ("text first", Tally::default()),
        ("what is drawn", Tally::default()),
    ];
    let mut rendered = 0;
    for fixture in test_corpus::all() {
        let Some(request) = support::request(fixture.name(), support::LIGHT) else {
            continue;
        };
        let document = support::render(&request);
        rendered += 1;
        let parsed = postio_model::mime::parse(fixture.bytes());
        for ((label, source), (_, tally)) in sources(&parsed.body).into_iter().zip(&mut tallies) {
            let Some(source) = source else { continue };
            for excerpt in excerpts(&source) {
                let outcome = locate(&document.text, &source, &excerpt);
                let locator = match by_the_locator(&document.text, &source, &excerpt) {
                    Some(_) => Which::Only,
                    None => Which::Unresolved,
                };
                tally.add(label, fixture.name(), &excerpt, outcome, locator);
            }
        }
    }

    println!("\nS5, the corpus as it is: {rendered} fixtures rendered\n");
    for (label, tally) in &tallies {
        tally.print(label);
    }
    println!("\nNot located as read:");
    for (_, tally) in &tallies {
        for miss in &tally.misses {
            println!("  {miss}");
        }
    }

    // The floors are what the spike measured, rounded down: a renderer change that loses text from
    // the index shows here first, and a new fixture moves the rates.
    for (label, tally) in &tallies {
        let [as_read, collapsed, present, located] = match *label {
            "text first" => [0.89, 0.95, 0.96, 0.96],
            _ => [0.91, 0.97, 0.98, 0.98],
        };
        assert!(
            tally.total() >= 100,
            "{label}: only {} sentences to measure",
            tally.total()
        );
        for (hows, floor, name) in [
            (&[How::AsRead][..], as_read, "as read"),
            (
                &[How::AsRead, How::SpacesCollapsed][..],
                collapsed,
                "with spaces collapsed",
            ),
        ] {
            let rate = tally.rate(hows);
            assert!(
                rate >= floor,
                "{label}: {rate:.3} located {name}, below the spike's {floor}"
            );
        }
        assert!(
            tally.present() >= present,
            "{label}: {:.3} present at all, below the spike's {present}",
            tally.present()
        );
        assert!(
            tally.by_the_locator() >= located,
            "{label}: the locator found {:.3}, below its measured {located}",
            tally.by_the_locator()
        );
    }
}

#[test]
fn a_repeated_sentence_is_told_apart_by_where_it_sits() {
    // No sentence occurs twice in its own message, so the test above never reaches the tiebreak:
    // sending every body twice puts every sentence in twice.
    let mut tallies = [
        ("text first", Tally::default()),
        ("what is drawn", Tally::default()),
    ];
    for fixture in test_corpus::all() {
        let body = postio_model::mime::parse(fixture.bytes()).body;
        if body.is_empty() {
            continue;
        }
        let doubled = MessageBody {
            text: body.text.as_ref().map(|text| format!("{text}\n\n{text}")),
            html: body.html.as_ref().map(|html| format!("{html}\n{html}")),
        };
        let Some(document) = render(&doubled) else {
            continue;
        };
        for ((label, source), (_, tally)) in sources(&doubled).into_iter().zip(&mut tallies) {
            let Some(source) = source else { continue };
            let (source_len, drawn_len) =
                (source.chars().count(), document.text.text.chars().count());
            for excerpt in excerpts(&source) {
                let outcome = locate(&document.text, &source, &excerpt);
                let locator = match by_the_locator(&document.text, &source, &excerpt) {
                    Some(found)
                        if (found.start * 2 < drawn_len) == (excerpt.offset * 2 < source_len) =>
                    {
                        Which::Right
                    }
                    Some(_) => Which::Wrong,
                    None => Which::Unresolved,
                };
                tally.add(label, fixture.name(), &excerpt, outcome, locator);
            }
        }
    }

    println!("\nS5, every body twice over:\n");
    for (label, tally) in &tallies {
        tally.print(label);
    }
    println!("\nPicked the wrong one, or could not count:");
    for (_, tally) in &tallies {
        for miss in tally
            .misses
            .iter()
            .filter(|miss| miss.contains("/Wrong") || miss.contains("/Uncounted"))
        {
            println!("  {miss}");
        }
    }

    for (label, tally) in &tallies {
        let (right, decided) = tally.locator_tiebreak();
        println!("{label}: the locator picked right {right} times in {decided}");
        assert!(
            right as f64 >= decided as f64 * 0.999,
            "{label}: the locator picked right {right} times in {decided}"
        );
        let (right, decided) = tally.tiebreak();
        println!("{label}: the tiebreak picked right {right} times in {decided}");
        assert!(
            decided >= 1000,
            "{label}: the tiebreak only had to choose {decided} times"
        );
        assert!(
            right as f64 >= decided as f64 * 0.999,
            "{label}: the tiebreak picked right {right} times in {decided}"
        );
    }
}
