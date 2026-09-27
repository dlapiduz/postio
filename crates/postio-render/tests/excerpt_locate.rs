//! Spike S5 (Postio Focus, research R2): can a sentence be found again in
//! the rendered body from nothing but its excerpt?
//!
//! Focus stores a marker's sentence -- a question, a to-do -- as an excerpt
//! and an offset into the text the detector read, and the open-email dialog
//! highlights it in the rendered body (spec FR-035, tasks T065 and T066).
//! Stored offsets cannot be used directly: the render's text index is in
//! laid-out reading order, with `alt` text in and closed folds out. So the
//! plan is to find the excerpt with `TextIndex::find` (which ignores case
//! and diacritics) and, when it occurs more than once, take the occurrence
//! nearest its proportional position.
//!
//! This measures that plan across the render corpus: every sentence a
//! detector could store, from each of the two texts it might read, located
//! in the document the reader draws. It changes no code. It prints what it
//! found -- run it with `--nocapture` to read the table -- and it fails if a
//! rate falls below what this spike measured, so the locator T066 builds
//! cannot quietly do worse.
//!
//! Two sources of excerpts, because which one the detector reads is not
//! decided yet (T115):
//!
//! * **text first**: the `text/plain` part when there is one, else the HTML
//!   as `postio_body` flattens it -- what the search index reads;
//! * **what is drawn**: the HTML flattened when there is HTML, else the
//!   plain part -- the text of the part the reader actually renders.
//!
//! POSTIO-MEASUREMENT: its output is a table a person reads, and it renders
//! the whole corpus three times over (about ninety seconds in a debug
//! build), so it runs nightly (`--profile nightly`), where its floors still
//! hold the rates below.
//!
//! # What it found (2026-09-27, 69 fixtures rendered)
//!
//! | excerpts read from | sentences | as read | spaces collapsed | present |
//! |---|---|---|---|---|
//! | text first         | 1,005     | 90.5%   | 96.8%            | 97.3%   |
//! | what is drawn      | 1,006     | 92.3%   | 98.7%            | 99.5%   |
//!
//! The first run, before eight invitations joined the corpus, rendered 61
//! fixtures and found 91.7/97.7/98.2 and 92.8/98.8/99.5. The invitations'
//! plain parts list organisers, guests and links the HTML never draws, and
//! wrap hard: they cost the text-first reading most.
//!
//! * **The index writes all whitespace as one space**, a `<pre>` line break
//!   included, and `find` compares whitespace as it is: every excerpt that
//!   spans a hard-wrapped line misses. Collapsing the excerpt's own runs of
//!   whitespace before `find` recovers six points, with no change here.
//! * **Blocks and cells are line breaks and tabs in the index** where a
//!   flattened source has spaces (a table row, a heading run into its
//!   paragraph): a locator that treats any run of whitespace as any other,
//!   on both sides, recovers what is left of "present".
//! * **What is never drawn cannot be found**: a `<title>`, a hidden
//!   preheader, a blocked image's `alt`, a list-marker image. Own-text
//!   extraction (T115) should leave them out.
//! * **The plain alternative can say something else**: reading the
//!   `text/plain` part of a message the reader draws as HTML loses 13 more
//!   sentences than reading the HTML.
//! * **No sentence occurs twice in its own message** anywhere in the corpus,
//!   so the tiebreak is measured on every body sent twice over: it picks the
//!   right occurrence 1,939 times in 1,940 (text first) and 1,979 in 1,980
//!   (what is drawn). The one miss is a line of emoji in `html-cjk-emoji`.
//!
//! # The locator T066 built (2026-09-27)
//!
//! `TextIndex::locate` is measured beside the plan, on the same excerpts. It
//! treats any run of whitespace as any other on both sides, so it finds
//! everything present at all: 98.2% (text first) and 99.5% (what is drawn)
//! of the corpus as it is. Over every body twice over it chooses the copy
//! the sentence was read from 1,918 times in 1,918 and 1,948 in 1,948.
//! That is judged by which half of the drawn text it lands in, not by
//! counting occurrences: folding drops the emoji line's variation
//! selectors, so the count's offsets drift, and that is the plan's one
//! "miss" above. Its floors are these numbers rounded down.

mod support;

use std::collections::BTreeMap;
use std::ops::Range;

use postio_body::RemoteImages;
use postio_model::{MessageBody, test_corpus};
use postio_render::{RenderedDocument, TextIndex};
use postio_ui::reader::document::{self, Rendering};

/// A sentence the detector could store: its text, and where it starts.
struct Excerpt {
    text: String,
    /// In chars, into the source it was read from.
    offset: usize,
}

/// How an excerpt was found in the index, if it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum How {
    /// By `TextIndex::find`, exactly as it was read: research R2's plan.
    AsRead,
    /// Only once every run of whitespace in the excerpt was one space: work
    /// a locator can do itself, with no change to the renderer.
    SpacesCollapsed,
    /// Only with whitespace made alike on both sides: the index has a line
    /// break where the excerpt has a space. Needs `find` itself to treat any
    /// run of whitespace as any other.
    BothCollapsed,
    /// Not at all: the drawn text does not say it.
    Absent,
}

/// Which occurrence the plan took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Which {
    /// There was only one.
    Only,
    /// Several; the offset picked the right one.
    Right,
    /// Several; the offset picked a wrong one.
    Wrong,
    /// Several, but not as many as the source holds, so "right" has no
    /// answer.
    Uncounted,
    /// Found without ranges to choose between, or not found.
    Unresolved,
}

type Outcome = (How, Which);

fn located(how: How, which: Which, hows: &[How]) -> bool {
    hows.contains(&how) && matches!(which, Which::Only | Which::Right)
}

/// Both readings of a body's text, as the two sources above.
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

/// The sentences of `source` a detector could store: the message's own
/// text, not quoted history or a signature, split at sentence ends and
/// blank lines, and long enough to be a question or a request.
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
        // A signature ends the message's own text; a quote is not its own.
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

/// Case and diacritics folded, as `TextIndex::find` folds them.
fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    use unicode_normalization::char::is_combining_mark;
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// `text` with every run of whitespace one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many times the excerpt occurs in `source`, folded, and which of
/// those it is: the last occurrence starting at or before its offset.
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

/// The plan's tiebreak: the match nearest the excerpt's proportional place.
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

/// Where the locator T066 built puts the excerpt: `TextIndex::locate`, the
/// real thing, judged beside the plan it was built from.
///
/// It treats any run of whitespace as any other on both sides, so it can
/// find what the plan found only with `How::BothCollapsed`. One call, one
/// fold of the document, so measuring it keeps this test inside the nightly
/// profile's budget.
fn by_the_locator(index: &TextIndex, source: &str, excerpt: &Excerpt) -> Option<Range<usize>> {
    index.locate(postio_render::Excerpt {
        text: &excerpt.text,
        offset: excerpt.offset,
        source_len: source.chars().count(),
    })
}

/// Every outcome for one source, and the misses to print.
#[derive(Default)]
struct Tally {
    outcomes: BTreeMap<Outcome, usize>,
    /// What the locator T066 built made of the same excerpts: `Only` when
    /// it found the sentence, `Right` or `Wrong` for the occurrence it chose
    /// in a body said twice, `Unresolved` when it found nothing.
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

    /// The share located with `hows` allowed, the tiebreak getting it right.
    fn rate(&self, hows: &[How]) -> f64 {
        let located: usize = self
            .outcomes
            .iter()
            .filter(|((how, which), _)| located(*how, *which, hows))
            .map(|(_, count)| count)
            .sum();
        located as f64 / self.total().max(1) as f64
    }

    /// How often the tiebreak picked the right occurrence, of the times it
    /// had to choose.
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

    /// The share the locator T066 built found, at the right occurrence
    /// where there was more than one.
    fn by_the_locator(&self) -> f64 {
        let located: usize = [Which::Only, Which::Right]
            .iter()
            .filter_map(|which| self.locator.get(which))
            .sum();
        located as f64 / self.total().max(1) as f64
    }

    /// How often the locator picked the right occurrence, of the times it
    /// had to choose.
    fn locator_tiebreak(&self) -> (usize, usize) {
        let count = |wanted: Which| self.locator.get(&wanted).copied().unwrap_or(0);
        let right = count(Which::Right);
        (right, right + count(Which::Wrong))
    }

    /// The share present at all, whitespace aside.
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

/// `body` drawn the way the reader draws a message: its original rendering,
/// remote images blocked, light.
fn render(body: &MessageBody) -> Option<RenderedDocument> {
    let rendered = document::body_html_in(body, RemoteImages::Blocked, Rendering::Original, None);
    if rendered.over_cap.is_some() {
        // Drawn from its plain part instead: not the text the HTML source
        // holds, so not a measurement of finding.
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
                // No sentence occurs twice in its own message, so finding
                // it is finding it.
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

    // The floors are what the spike measured, rounded down, for each source
    // on its own: the locator T066 builds must do at least this well, and a
    // renderer change that loses text from the index shows here first. A
    // new fixture moves the rates, so the floors are measured again when
    // the corpus grows -- the eight invitations did, and their plain parts
    // hold text the HTML never draws.
    for (label, tally) in &tallies {
        let [as_read, collapsed, present] = match *label {
            "text first" => [0.90, 0.96, 0.97],
            _ => [0.92, 0.98, 0.99],
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
        // The locator itself (T066), measured 2026-09-27: 98.2% of what the
        // text part says, 99.5% of what is drawn -- everything present.
        assert!(
            tally.by_the_locator() >= 0.98,
            "{label}: the locator found {:.3}, below its measured 0.98",
            tally.by_the_locator()
        );
    }
}

#[test]
fn a_repeated_sentence_is_told_apart_by_where_it_sits() {
    // No sentence in the corpus occurs twice in its own message, so the
    // test above never reaches the tiebreak. Every body sent twice over, in
    // one message, puts every sentence in it twice.
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
                // Both texts are one body and then the same body again, so
                // the right occurrence is in the copy the excerpt was read
                // from: the same half of the drawn text.
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
