//! A Mac controller, an inbox and a search engine that answers the results
//! view's reads from a fixed set of conversations: what `results.rs` and
//! `history.rs` drive (spec 010 step 3).
//!
//! The engine is the shape the host's is: a page of conversations in the
//! order asked, every answer carrying the same totals and facets, months
//! counted by each conversation's newest match (D4).

#![allow(dead_code)]

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    Effect, FocusController, Input, Intent, Policy, PopoverView, QueryView, QuickLookView, Reply,
    Request, ResultsView, RowFacts, Rows, Ticket,
};
use postio_model::{AddressId, EmailAddress, LabelId, MailboxId, MessageId, ThreadId};
use postio_search::facets::{Count, MonthCount, SearchFacets, months_ending};
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationMatch, ConversationOrder, ConversationResults,
    FacetNames, Match, Passage, RankReason, Source,
};

/// An inbox of `len` conversations.
pub struct List {
    pub rows: Vec<RowFacts>,
}

impl List {
    pub fn of(len: i64) -> Self {
        List {
            rows: (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: false,
                    threads: vec![ThreadId::new(500 + at)],
                    writes: false,
                })
                .collect(),
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.rows.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.rows.get(position as usize).cloned()
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
}

/// Saturday 26 September 2026, mid-afternoon, as the screens are.
pub fn today() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 9, 26, 15, 0, 0).unwrap()
}

/// The Mac's controller, its clock stopped on the screens' day.
pub fn mac() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    assert!(focus.policy().caps.results_view);
    let _ = focus.handle(Input::Clock(Some(today())));
    focus
}

pub fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

pub fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

pub fn asked(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

/// The last results view among `effects`.
pub fn results_view(effects: &[Effect]) -> Option<ResultsView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ShowResults(view) => Some(*view),
            _ => None,
        })
}

/// The last query view among `effects`.
pub fn query_view(effects: &[Effect]) -> Option<QueryView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Query(view) => Some(view),
            _ => None,
        })
}

/// The last filter popover among `effects`: `Some(None)` when it closed.
pub fn popover_view(effects: &[Effect]) -> Option<Option<PopoverView>> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Popover(view) => Some(view.map(|view| *view)),
            _ => None,
        })
}

/// The last no-results page among `effects`: `Some(None)` when it went.
pub fn no_results_view(effects: &[Effect]) -> Option<Option<postio_focus::NoResultsView>> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Relaxations(view) => Some(view.map(|view| *view)),
            _ => None,
        })
}

/// The last Quick Look among `effects`: `Some(None)` when it closed.
pub fn quick_look_view(effects: &[Effect]) -> Option<Option<QuickLookView>> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::QuickLook(view) => Some(view.map(|view| *view)),
            _ => None,
        })
}

/// Every match in conversation `n`, oldest first, as the engine reads
/// them: an earlier message from Ben, the best message's own words, the
/// subject.
pub fn conversation_matches(key: ConversationKey) -> Vec<ConversationMatch> {
    let ConversationKey::Thread(thread) = key else {
        return Vec::new();
    };
    let n = thread.get() - 2000;
    let best = conversation(n);
    let subject = best.subject.clone().unwrap_or_default();
    let marks =
        postio_search::highlight::find(&subject, &["atlas".to_owned(), "budget".to_owned()]);
    vec![
        ConversationMatch {
            message: Some(MessageId::new(500 + n)),
            from: Some(ben()),
            found: Match {
                source: Source::Body,
                passage: Some(passage_for(MessageId::new(500 + n))),
                when: Some(when(n) - chrono::TimeDelta::days(1)),
            },
        },
        ConversationMatch {
            message: Some(best.best),
            from: best.from.clone(),
            found: Match {
                source: Source::Body,
                passage: Some(passage_for(best.best)),
                when: Some(when(n)),
            },
        },
        ConversationMatch {
            message: None,
            from: None,
            found: Match {
                source: Source::Subject,
                passage: Some(Passage {
                    text: subject,
                    ranges: marks,
                    elided_start: false,
                    elided_end: false,
                }),
                when: None,
            },
        },
    ]
}

/// The query each results read among `effects` asked for.
pub fn queries_asked(effects: &[Effect]) -> Vec<String> {
    asked(effects)
        .into_iter()
        .filter_map(|request| match request {
            Request::ResultsPage { query, .. } => Some(query.input().to_owned()),
            _ => None,
        })
        .collect()
}

pub fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

pub fn ada() -> EmailAddress {
    EmailAddress::new(Some("Ada Moreno"), "ada@example.com")
}

pub fn tomas() -> EmailAddress {
    EmailAddress::new(Some("Tom\u{e1}s Reyes"), "tomas@example.com")
}

/// Who most of the conversations were written to.
pub fn ben() -> EmailAddress {
    EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com")
}

/// Conversations in the engine, newest first: more than a page of them.
pub const CONVERSATIONS: i64 = 120;

/// When conversation `n` (0 the newest) last matched: one every two days
/// back from the 26th of September, so the 120 span September to February.
pub fn when(n: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap() - chrono::TimeDelta::days(n * 2)
}

/// Conversation `n`: its best message is `1000 + n`.
pub fn conversation(n: i64) -> ConversationHit {
    let from = if n % 2 == 0 { ada() } else { tomas() };
    ConversationHit {
        key: ConversationKey::Thread(ThreadId::new(2000 + n)),
        best: MessageId::new(1000 + n),
        mailbox_id: MailboxId::new(1),
        subject: Some(format!("Atlas budget, part {n}")),
        from: Some(from),
        newest_match: when(n),
        messages: if n == 0 { 3 } else { 1 },
        unread: n == 0,
        has_attachments: n == 0,
        labels: if n == 0 {
            vec![LabelId::new(3)]
        } else {
            Vec::new()
        },
        score: n as f64,
        reasons: match n {
            7 => vec![RankReason::Replied, RankReason::Matches(3)],
            2 => vec![RankReason::Flagged, RankReason::Matches(2)],
            30 => vec![RankReason::FrequentSender, RankReason::InFileName],
            _ => vec![RankReason::Matches(1)],
        },
        matches: vec![Match {
            source: if n == 4 { Source::Quoted } else { Source::Body },
            passage: None,
            when: None,
        }],
    }
}

/// The order the engine ranks them by Best match: 7, 2 and 30 first.
pub fn best_match() -> Vec<i64> {
    let mut order = vec![7, 2, 30];
    order.extend((0..CONVERSATIONS).filter(|n| ![7, 2, 30].contains(n)));
    order
}

/// The timeline the engine counts: each conversation in the month of its
/// newest match (D4).
pub fn months() -> [MonthCount; 12] {
    let firsts = months_ending(today().date_naive());
    firsts.map(|month| MonthCount {
        month,
        conversations: (0..CONVERSATIONS)
            .filter(|n| {
                let day = when(*n).date_naive();
                day >= month && day < month.checked_add_months(chrono::Months::new(1)).unwrap()
            })
            .count() as u64,
    })
}

/// How many conversations the engine finds for `query`: a `from:` Ada
/// keeps hers, the even ones, and `-from:` Ada the others.
pub fn matching(query: &postio_search::ParsedQuery) -> Vec<i64> {
    let ada = |negated: bool| {
        query.filters().any(|clause| {
            clause.negated == negated
                && matches!(&clause.filter, postio_search::query::Filter::From(who) if who.contains("ada"))
        })
    };
    let (only, never) = (ada(false), ada(true));
    // Nothing is called "v4" (screen 13's "budget v4").
    if query.filters().any(|clause| {
        !clause.negated
            && matches!(&clause.filter, postio_search::query::Filter::Subject(words) if words.contains("v4"))
    }) {
        return Vec::new();
    }
    // `after:` and `before:` keep the days they name, by the newest match.
    let (mut after, mut before) = (None, None);
    for clause in query.filters().filter(|clause| !clause.negated) {
        match clause.filter {
            postio_search::query::Filter::After(day) => after = Some(day),
            postio_search::query::Filter::Before(day) => before = Some(day),
            _ => {}
        }
    }
    (0..CONVERSATIONS)
        .filter(|n| !only || n % 2 == 0)
        .filter(|n| !never || n % 2 == 1)
        .filter(|n| after.is_none_or(|day| when(*n).date_naive() >= day))
        .filter(|n| before.is_none_or(|day| when(*n).date_naive() < day))
        .collect()
}

/// The engine's answer to a page of `query` in `order`.
pub fn page(
    query: &postio_search::ParsedQuery,
    order: ConversationOrder,
    offset: u32,
    limit: u32,
) -> ConversationResults {
    let found = matching(query);
    let ranked: Vec<i64> = match order {
        ConversationOrder::Newest => found.clone(),
        _ => best_match()
            .into_iter()
            .filter(|n| found.contains(n))
            .collect(),
    };
    let hits = ranked
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|n| conversation(*n))
        .collect();
    let mut facets = SearchFacets {
        senders: vec![
            Count {
                id: AddressId::new(1),
                conversations: 24,
            },
            Count {
                id: AddressId::new(2),
                conversations: 24,
            },
        ],
        recipients: vec![Count {
            id: AddressId::new(4),
            conversations: 7,
        }],
        labels: vec![Count {
            id: LabelId::new(3),
            conversations: 1,
        }],
        folders: vec![
            Count {
                id: MailboxId::new(1),
                conversations: 117,
            },
            Count {
                id: MailboxId::new(2),
                conversations: 3,
            },
        ],
        attachment: 1,
        months: months(),
        // Last 7 days, last 30 days, this quarter, this year, any time.
        presets: [2, 6, 12, 17, found.len() as u64],
        ..SearchFacets::default()
    };
    if found.len() as i64 != CONVERSATIONS {
        facets.months = months_ending(today().date_naive()).map(|month| MonthCount {
            month,
            conversations: found
                .iter()
                .filter(|n| {
                    let day = when(**n).date_naive();
                    day >= month && day < month.checked_add_months(chrono::Months::new(1)).unwrap()
                })
                .count() as u64,
        });
    }
    ConversationResults {
        hits,
        total: found.len() as u64,
        capped: false,
        messages_searched: 18_204,
        corpus_complete: true,
        contents_complete: true,
        facets,
        files: 12,
        people: 6,
        names: FacetNames {
            people: vec![
                (AddressId::new(1), ada()),
                (AddressId::new(2), tomas()),
                (AddressId::new(4), ben()),
            ],
            labels: vec![(LabelId::new(3), "Atlas".to_owned())],
            label_colors: vec![(LabelId::new(3), "#c08a2e".to_owned())],
            folders: vec![
                (MailboxId::new(1), "Inbox".to_owned()),
                (MailboxId::new(2), "Archive".to_owned()),
            ],
        },
        elapsed: std::time::Duration::from_millis(41),
    }
}

/// A passage for `message`, with "atlas" and "budget" marked.
pub fn passage_for(message: MessageId) -> Passage {
    let text = format!("the final numbers for the Atlas budget, message {message}");
    let start = text.find("Atlas").unwrap();
    Passage {
        ranges: vec![start..start + 5, start + 6..start + 12],
        text,
        elided_start: true,
        elided_end: false,
    }
}

/// The engine's answer to one results request, if it is one.
pub fn reply_to(request: &Request) -> Option<Reply> {
    match request {
        Request::ResultsPage {
            query,
            order,
            offset,
            limit,
            stamp,
        } => Some(Reply::ResultsPage {
            stamp: *stamp,
            order: *order,
            offset: *offset,
            answer: Ok(Box::new(page(query, *order, *offset, *limit))),
        }),
        Request::ResultsPassages { hits, stamp, .. } => Some(Reply::ResultsPassages {
            stamp: *stamp,
            answer: Ok(hits
                .iter()
                .map(|(message, sources)| {
                    (
                        *message,
                        sources
                            .iter()
                            .map(|source| Match {
                                source: source.clone(),
                                passage: Some(passage_for(*message)),
                                when: None,
                            })
                            .collect(),
                    )
                })
                .collect()),
        }),
        Request::QuickLookMatches { key, stamp, .. } => Some(Reply::QuickLookMatches {
            stamp: *stamp,
            answer: Ok(conversation_matches(*key)),
        }),
        _ => None,
    }
}

/// Answer every results read among `effects`, and the reads those answers
/// ask, until none is left: the effects of it all, `effects` first.
pub fn settle(focus: &mut FocusController, effects: Vec<Effect>, rows: &List) -> Vec<Effect> {
    let mut all = effects.clone();
    let mut waiting = effects;
    loop {
        let mut next = Vec::new();
        for (ticket, request) in asks(&waiting) {
            if let Some(reply) = reply_to(&request) {
                next.extend(focus.handle_on(Input::Reply(ticket, reply), rows));
            }
        }
        if next.is_empty() {
            return all;
        }
        all.extend(next.clone());
        waiting = next;
    }
}

/// `/`, `words` typed, then ⌘↩: the results asked for, not yet answered.
pub fn show_all(focus: &mut FocusController, words: &str, rows: &List) -> Vec<Effect> {
    let _ = run(focus, CommandId::Search, rows);
    let _ = focus.handle_on(
        Input::Typed {
            text: words.to_owned(),
        },
        rows,
    );
    run(focus, CommandId::ShowAllResults, rows)
}

/// `/`, `words`, ⌘↩, and every read answered.
pub fn search(focus: &mut FocusController, words: &str, rows: &List) -> Vec<Effect> {
    let effects = show_all(focus, words, rows);
    settle(focus, effects, rows)
}

/// The first day of a month, for the group titles.
pub fn month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).unwrap()
}
