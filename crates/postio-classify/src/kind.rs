//! What sort of correspondent a sender is (specs/011-focus-contacts, plan
//! section 5): a pure function over counted evidence and a table of rules.
//!
//! [`detect`] reads [`KindSignals`], the counts a sender's newest messages
//! gave, and [`KindRules`], the data in `data/kinds.toml`. It touches no
//! store and no network, and cannot send. A user's own choice of kind is
//! never decided here: storage keeps it beside the detector's and the
//! detector's answer never replaces it.

use std::fmt;
use std::sync::LazyLock;

use postio_model::{ContactKind, KindSignals};
use serde::Deserialize;

/// Which detector decided a kind. Raise it when a rule changes enough that
/// every detected kind should be decided again; storage records it per
/// contact (`kind_version`).
pub const KIND_VERSION: u32 = 1;

static SHIPPED: LazyLock<KindRules> = LazyLock::new(|| {
    KindRules::parse(include_str!("../data/kinds.toml")).expect("data/kinds.toml loads")
});

/// The tables detection decides by, as `data/kinds.toml` ships them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindRules {
    robot_local: Vec<String>,
    role_local: Vec<String>,
    company_min_people: u32,
    bulk_words: Vec<String>,
    freemail_domains: Vec<String>,
    habit_percent: u32,
}

/// Why a table does not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindRulesError(String);

impl fmt::Display for KindRulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "kind rules: {}", self.0)
    }
}

impl std::error::Error for KindRulesError {}

impl KindRules {
    /// The rules Postio ships.
    pub fn shipped() -> &'static KindRules {
        &SHIPPED
    }

    /// Rules from TOML, refusing an unknown key and any table that would
    /// make everyone a person.
    pub fn parse(text: &str) -> Result<KindRules, KindRulesError> {
        let rules: KindRules = toml::from_str(text).map_err(|e| KindRulesError(e.to_string()))?;
        if rules.habit_percent == 0 || rules.habit_percent > 100 {
            return Err(KindRulesError("habit_percent is between 1 and 100".into()));
        }
        if rules.robot_local.is_empty() || rules.role_local.is_empty() {
            return Err(KindRulesError(
                "robot_local and role_local are not empty".into(),
            ));
        }
        Ok(rules)
    }
}

/// A decided kind, and how sure the evidence makes it (0.0 to 1.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detected {
    /// What the sender looks like.
    pub kind: ContactKind,
    /// A clear case is near 1; "no evidence, so a person" is low.
    pub confidence: f32,
}

/// Decides a kind. First match wins, and the user having written to the
/// sender floors them at a person unless they are a list or a newsletter.
pub fn detect(signals: &KindSignals, rules: &KindRules) -> Detected {
    let counted = signals.messages.max(1);
    let habit = |with: u32| signals.messages > 0 && with * 100 >= counted * rules.habit_percent;
    let share = |with: u32| (with as f32 / counted as f32).min(1.0);
    let (local, domain) = signals
        .address
        .rsplit_once('@')
        .map(|(l, d)| (l.to_lowercase(), d.to_lowercase()))
        .unwrap_or_default();
    let words = words(&local);
    let name_words: Vec<String> = signals
        .name
        .as_deref()
        .map(|name| {
            name.split(|c: char| !c.is_alphanumeric())
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_default();
    let bulk_name = rules
        .bulk_words
        .iter()
        .any(|w| words.contains(&w.as_str()) || name_words.iter().any(|n| n == w));

    let unsubscribe = habit(signals.with_unsubscribe);
    let newsletter_evidence = unsubscribe || habit(signals.with_bulk) || bulk_name;
    let listed = habit(signals.with_list_id) || habit(signals.with_list_precedence);

    // 1. A list, unless the same mail is plainly a newsletter.
    if listed && !(unsubscribe && newsletter_evidence) {
        return Detected {
            kind: ContactKind::List,
            confidence: share(signals.with_list_id.max(signals.with_list_precedence)),
        };
    }
    // 2. A newsletter: an unsubscribe offered with bulk evidence beside it,
    //    or a list that also offers one.
    if unsubscribe && (newsletter_evidence || listed) {
        return Detected {
            kind: ContactKind::Newsletter,
            confidence: 0.6 + 0.4 * share(signals.with_unsubscribe),
        };
    }
    // A reply from the user is evidence of a human, and wins from here on.
    if signals.written_to > 0 {
        return Detected {
            kind: ContactKind::Person,
            confidence: 0.9,
        };
    }
    // 3. A robot: it says so, or its mail says so most of the time.
    let robot_address = rules
        .robot_local
        .iter()
        .any(|pattern| matches_words(&words, pattern));
    if robot_address
        || habit(signals.with_junk)
        || habit(signals.with_auto_generated)
        || habit(signals.with_auto_replied)
    {
        return Detected {
            kind: ContactKind::Automated,
            confidence: if robot_address {
                0.9
            } else {
                share(signals.with_auto_generated)
            },
        };
    }
    // 4. An organisation's role mailbox, where the user knows its people.
    let role = rules
        .role_local
        .iter()
        .any(|pattern| matches_words(&words, pattern));
    let freemail = rules.freemail_domains.contains(&domain);
    if role && !freemail && signals.people_at_domain >= rules.company_min_people {
        return Detected {
            kind: ContactKind::Company,
            confidence: 0.75,
        };
    }
    // 5. Otherwise a person, which is also what no evidence at all means.
    Detected {
        kind: ContactKind::Person,
        confidence: if signals.messages == 0 { 0.3 } else { 0.6 },
    }
}

/// A local part's words: split at `.`, `-`, `_` and `+`.
fn words(local: &str) -> Vec<&str> {
    local
        .split(['.', '-', '_', '+'])
        .filter(|word| !word.is_empty())
        .collect()
}

/// Whether `pattern`'s words (`no-reply` is `no`, `reply`) appear in order
/// and together among `words`.
fn matches_words(words: &[&str], pattern: &str) -> bool {
    let wanted: Vec<&str> = pattern.split('-').collect();
    !wanted.is_empty()
        && words
            .windows(wanted.len())
            .any(|window| window == wanted.as_slice())
}
