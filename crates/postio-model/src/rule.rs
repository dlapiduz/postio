//! `[[rules]]`, parsed: what a rule *is*, before anything runs one.
//!
//! ADR 0008 Q4 makes rules user-authored config rather than a database table,
//! and an **ordered array** rather than a map, because the only way to get
//! order out of a map is an `order = 3` field on every entry — which users
//! duplicate, skip, and have to renumber to insert a rule in the middle. The
//! file is the order.
//!
//! ```toml
//! [[rules]]
//! name    = "receipts"
//! query   = "from:billing has:attach"
//! actions = ["move:Receipts", "mark-read"]
//! stop    = true
//!
//! [[rules]]
//! name    = "needs-reply"
//! filter  = "needs-reply"      # reuse a named [filters] query
//! actions = ["flag"]
//! enabled = false              # dry-run it first
//! ```
//!
//! # Why the typing happens here
//!
//! `postio-config` keeps everything as text on purpose — that is what holds
//! its dependency list to four crates and none of them domain — so
//! `"move:Receipts"` is a string to it and `[`Action`]` is this crate's
//! business. It reads a config entry, hands over a [`RuleSource`], and gets
//! back either a [`Rule`] or the specific reason it is not one.
//!
//! # The query stays text, and that is not an omission
//!
//! Parsing `from:billing has:attach` belongs to `postio-search`, which
//! depends on *this* crate — so it cannot be called from here without
//! inverting the graph. It does not need to be: the query parser is total,
//! every input parses, and an unknown operator becomes free text rather than
//! an error. There is no "query that fails to parse" for this module to
//! reject; what there is, and what it does reject, is a rule with **no**
//! query, one that names both a query and a filter, and one that names a
//! `[filters]` entry that does not exist.
//!
//! # Nothing here runs anything
//!
//! No message is matched, no mailbox is moved, nothing is forwarded. The two
//! evaluation points are ADR 0008 Q3's and the actions are Q5's; this is the
//! schema and the parse they are both written against.

use std::fmt;

/// One thing a rule does when it matches.
///
/// ADR 0008 Q5's vocabulary. Two are constrained rather than merely listed:
///
/// * **[`Trash`](Action::Trash), never `delete`.** A rule moves mail to the
///   Trash mailbox; permanent removal is not available to one. Issue #5's
///   fourth criterion — a rule that errors never silently drops mail — is
///   much easier to hold when no rule can destroy anything in the first
///   place.
/// * **[`Forward`](Action::Forward) is the one that leaves the machine.**
///   `ARCHITECTURE.md` §11's test is "did the user ask for it", and writing
///   the rule is asking — so it parses here. Its three guards (never forward
///   what a rule already forwarded, refuse an address of a configured
///   account, rate-cap per hour) are the engine's, because each of them needs
///   something this module does not have: the message, the account list, a
///   clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `move:Receipts` — file the message in a mailbox by name.
    Move(String),
    /// `label:invoices` — apply a label, leaving the message where it is.
    Label(String),
    /// `flag` — the state the sidebar calls Flagged.
    Flag,
    /// `unflag`
    Unflag,
    /// `mark-read`
    MarkRead,
    /// `mark-unread`
    MarkUnread,
    /// `archive`
    Archive,
    /// `trash` — to the Trash mailbox. Never a permanent delete.
    Trash,
    /// `forward:ada@example.com`
    Forward(String),
}

impl Action {
    /// Parses one entry of a rule's `actions` list.
    ///
    /// `name` or `name:argument`, split at the **first** colon so a mailbox
    /// path may contain one — `move:Lists/harbour-dev` is one action, and
    /// splitting at the last colon would make it a mailbox called
    /// `Lists/harbour-dev` filed under `move:Lists`. The same rule the query
    /// language uses, for the same reason.
    pub fn parse(text: &str) -> Result<Action, RuleError> {
        let text = text.trim();
        let (name, argument) = match text.split_once(':') {
            Some((name, argument)) => (name.trim(), Some(argument.trim())),
            None => (text, None),
        };
        let needs = |argument: Option<&str>| -> Result<String, RuleError> {
            match argument.map(str::trim).filter(|value| !value.is_empty()) {
                Some(value) => Ok(value.to_owned()),
                None => Err(RuleError::ActionNeedsArgument {
                    action: name.to_owned(),
                }),
            }
        };
        let takes_none = |action: Action| -> Result<Action, RuleError> {
            match argument {
                None => Ok(action),
                Some(_) => Err(RuleError::ActionTakesNoArgument {
                    action: name.to_owned(),
                }),
            }
        };

        match name.to_ascii_lowercase().as_str() {
            "move" => Ok(Action::Move(needs(argument)?)),
            "label" => Ok(Action::Label(needs(argument)?)),
            "forward" => {
                let address = needs(argument)?;
                // Not validating the address properly -- `postio-model` shows
                // what arrived rather than refusing it, and a rule pointing at
                // an address that bounces is a delivery problem, not a config
                // one. What this catches is `forward:Receipts`, which is a
                // mailbox name somebody meant `move:` for and which would
                // otherwise fail silently at send time.
                if !address.contains('@') {
                    return Err(RuleError::ForwardNeedsAnAddress { target: address });
                }
                Ok(Action::Forward(address))
            }
            "flag" => takes_none(Action::Flag),
            "unflag" => takes_none(Action::Unflag),
            "mark-read" | "read" => takes_none(Action::MarkRead),
            "mark-unread" | "unread" => takes_none(Action::MarkUnread),
            "archive" => takes_none(Action::Archive),
            "trash" => takes_none(Action::Trash),
            // Named separately from an unknown action because it is in ADR
            // 0008 Q5's list and a reader will reasonably try it. One meaning
            // gets one spelling: short-circuit is `stop = true` on the rule
            // (Q4), and an action that also set it would be a second way to
            // say the same thing for the two to disagree about.
            "stop" => Err(RuleError::StopIsARuleKey),
            "delete" => Err(RuleError::DeleteIsNotAnAction),
            _ => Err(RuleError::UnknownAction {
                action: name.to_owned(),
            }),
        }
    }
}

/// One `[[rules]]` entry as text, however the caller read it.
///
/// Plain fields rather than anything `serde`-shaped: `postio-config` owns the
/// file format and this crate owns the meaning, and a struct with a
/// `Deserialize` here would put the file format in two places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSource {
    /// What the user called it. Shown in Attention and in the settings panel.
    pub name: String,
    /// An inline query, or `None` when [`filter`](Self::filter) names one.
    pub query: Option<String>,
    /// The `[filters]` entry whose query to reuse, so a query the user
    /// already tuned is not written twice.
    pub filter: Option<String>,
    /// The actions, in order, as written.
    pub actions: Vec<String>,
    /// Whether a match stops the rules below this one.
    pub stop: bool,
    /// Whether this rule runs at all. `false` is how a rule is dry-run.
    pub enabled: bool,
}

impl Default for RuleSource {
    fn default() -> Self {
        Self {
            name: String::new(),
            query: None,
            filter: None,
            actions: Vec::new(),
            stop: false,
            // The only field whose default is not the zero value: a rule
            // somebody wrote is a rule they meant to run, and `enabled` is
            // there to turn one *off*.
            enabled: true,
        }
    }
}

/// One rule, typed and resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// What the user called it.
    pub name: String,
    /// The query text, inline or taken from the named `[filters]` entry.
    ///
    /// Still text: parsing it is `postio-search`'s, which depends on this
    /// crate. See the module docs.
    pub query: String,
    /// The actions, in order. Never empty — a rule that does nothing is
    /// rejected rather than parsed.
    pub actions: Vec<Action>,
    /// Whether a match stops the rules below this one. Default `false`, so
    /// "add a label to everything from this list" cannot silently disable
    /// every rule under it (ADR 0008 Q4).
    pub stop: bool,
    /// Whether this rule runs.
    pub enabled: bool,
    /// The `[filters]` entry the query came from, when it came from one.
    ///
    /// Kept so the settings panel can show where a rule's query lives, and so
    /// an edit to the filter is visibly an edit to the rule.
    pub filter: Option<String>,
}

impl Rule {
    /// Types one entry, resolving `filter = "…"` through `filters`.
    ///
    /// `filters` answers a saved-search name with its query text —
    /// `postio-config` passes a lookup into its own `[filters]` map. A name
    /// it does not know is an error and never an empty query: a rule whose
    /// query silently became "" would match every message and then act on it.
    pub fn parse<'a>(
        source: &RuleSource,
        filters: impl Fn(&str) -> Option<&'a str>,
    ) -> Result<Rule, RuleError> {
        let query = match (
            source
                .query
                .as_deref()
                .map(str::trim)
                .filter(|q| !q.is_empty()),
            source
                .filter
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty()),
        ) {
            // Both is not a merge and not a precedence: it is a user who
            // means two different things and will get whichever one this
            // module picked. Say so instead.
            (Some(_), Some(filter)) => {
                return Err(RuleError::QueryAndFilter {
                    filter: filter.to_owned(),
                });
            }
            (Some(query), None) => query.to_owned(),
            (None, Some(filter)) => match filters(filter) {
                Some(query) if !query.trim().is_empty() => query.trim().to_owned(),
                Some(_) => {
                    return Err(RuleError::FilterHasNoQuery {
                        filter: filter.to_owned(),
                    });
                }
                None => {
                    return Err(RuleError::UnknownFilter {
                        filter: filter.to_owned(),
                    });
                }
            },
            (None, None) => return Err(RuleError::NoQuery),
        };

        // An empty action list is a rule that matches mail and does nothing
        // with it, which is indistinguishable from a rule that is broken.
        // Rejected rather than parsed and skipped at run time, so the user
        // hears about it in the settings panel rather than never.
        if source.actions.iter().all(|action| action.trim().is_empty()) {
            return Err(RuleError::NoActions);
        }

        let actions = source
            .actions
            .iter()
            .filter(|action| !action.trim().is_empty())
            .map(|action| Action::parse(action))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Rule {
            name: source.name.clone(),
            query,
            actions,
            stop: source.stop,
            enabled: source.enabled,
            filter: source.filter.clone().filter(|f| !f.trim().is_empty()),
        })
    }
}

/// Why a `[[rules]]` entry is not a rule.
///
/// Every variant names the specific thing that is wrong, because the
/// alternative is what ADR 0008 Q6 is about: an entry silently dropped is a
/// rule the user believes is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    /// Neither `query` nor `filter`.
    NoQuery,
    /// Both `query` and `filter`, which are two different intentions.
    QueryAndFilter {
        /// The `[filters]` entry that was named.
        filter: String,
    },
    /// `filter = "…"` names no `[filters]` entry.
    UnknownFilter {
        /// The name that resolved to nothing.
        filter: String,
    },
    /// `filter = "…"` names an entry whose own query is empty.
    FilterHasNoQuery {
        /// The name that resolved to an empty query.
        filter: String,
    },
    /// `actions` is empty, or every entry in it is blank.
    NoActions,
    /// An action Postio does not know.
    UnknownAction {
        /// What was written, before the colon.
        action: String,
    },
    /// `move`, `label` or `forward` with nothing after the colon.
    ActionNeedsArgument {
        /// The action that wanted one.
        action: String,
    },
    /// `flag:something` — an action that takes no argument, given one.
    ActionTakesNoArgument {
        /// The action that takes none.
        action: String,
    },
    /// `forward:Receipts` — a mailbox name where an address belongs.
    ForwardNeedsAnAddress {
        /// What was written after the colon.
        target: String,
    },
    /// `stop` written as an action rather than as the rule's own key.
    StopIsARuleKey,
    /// `delete` — deliberately not an action (ADR 0008 Q5).
    DeleteIsNotAnAction,
}

impl fmt::Display for RuleError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleError::NoQuery => {
                write!(out, "needs a `query` or a `filter` naming a saved search")
            }
            RuleError::QueryAndFilter { filter } => write!(
                out,
                "has both a `query` and `filter = \"{filter}\"`; keep one"
            ),
            RuleError::UnknownFilter { filter } => {
                write!(out, "names `[filters.{filter}]`, which does not exist")
            }
            RuleError::FilterHasNoQuery { filter } => {
                write!(out, "names `[filters.{filter}]`, whose query is empty")
            }
            RuleError::NoActions => write!(out, "has no actions, so it would do nothing"),
            RuleError::UnknownAction { action } => write!(
                out,
                "has an unknown action `{action}`; the actions are move, label, \
                 flag, unflag, mark-read, mark-unread, archive, trash and forward"
            ),
            RuleError::ActionNeedsArgument { action } => {
                write!(out, "has `{action}` with nothing after the colon")
            }
            RuleError::ActionTakesNoArgument { action } => {
                write!(
                    out,
                    "has `{action}` with an argument, which it does not take"
                )
            }
            RuleError::ForwardNeedsAnAddress { target } => write!(
                out,
                "has `forward:{target}`, which is not an address; `move:{target}` \
                 files mail in a mailbox"
            ),
            RuleError::StopIsARuleKey => write!(
                out,
                "lists `stop` as an action; it is the rule's own key, `stop = true`"
            ),
            RuleError::DeleteIsNotAnAction => write!(
                out,
                "lists `delete`; a rule may not destroy mail, so use `trash`"
            ),
        }
    }
}

impl std::error::Error for RuleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn source(actions: &[&str]) -> RuleSource {
        RuleSource {
            name: "receipts".to_owned(),
            query: Some("from:billing has:attach".to_owned()),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
            ..RuleSource::default()
        }
    }

    fn no_filters(_: &str) -> Option<&'static str> {
        None
    }

    #[test]
    fn a_rule_keeps_its_actions_in_the_order_they_were_written() {
        // Order is the whole reason `[[rules]]` is an array (ADR 0008 Q4),
        // and it has to survive inside one rule as well as between them:
        // `move` then `mark-read` and `mark-read` then `move` are the same
        // set and not the same rule.
        let rule = Rule::parse(&source(&["move:Receipts", "mark-read", "flag"]), no_filters)
            .expect("a rule");
        assert_eq!(
            rule.actions,
            vec![
                Action::Move("Receipts".to_owned()),
                Action::MarkRead,
                Action::Flag
            ]
        );
        assert_eq!(rule.query, "from:billing has:attach");
        assert_eq!(rule.name, "receipts");
    }

    #[test]
    fn stop_is_false_and_enabled_is_true_unless_the_file_says_otherwise() {
        // The asymmetry is deliberate. A `stop`-by-default engine makes "add
        // a label to everything from this list" silently disable every rule
        // below it; a rule somebody wrote is a rule they meant to run.
        let rule = Rule::parse(&source(&["flag"]), no_filters).expect("a rule");
        assert!(!rule.stop);
        assert!(rule.enabled);

        let dry_run = RuleSource {
            stop: true,
            enabled: false,
            ..source(&["flag"])
        };
        let rule = Rule::parse(&dry_run, no_filters).expect("a rule");
        assert!(rule.stop);
        assert!(!rule.enabled);
    }

    #[test]
    fn a_filter_reference_resolves_to_that_filters_query() {
        let filters = BTreeMap::from([("needs-reply", "is:unread from:team")]);
        let source = RuleSource {
            name: "nudge".to_owned(),
            filter: Some("needs-reply".to_owned()),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        };

        let rule = Rule::parse(&source, |name| filters.get(name).copied()).expect("a rule");

        assert_eq!(rule.query, "is:unread from:team");
        assert_eq!(
            rule.filter.as_deref(),
            Some("needs-reply"),
            "where the query came from is kept, so an edit to the filter is \
             visibly an edit to the rule"
        );
    }

    #[test]
    fn an_undefined_filter_name_is_a_named_error_and_never_an_empty_query() {
        // The acceptance criterion. A rule whose query silently became `""`
        // would match every message in the mailbox and then act on it.
        let source = RuleSource {
            name: "nudge".to_owned(),
            filter: Some("nope".to_owned()),
            actions: vec!["trash".to_owned()],
            ..RuleSource::default()
        };

        assert_eq!(
            Rule::parse(&source, no_filters),
            Err(RuleError::UnknownFilter {
                filter: "nope".to_owned()
            })
        );
    }

    #[test]
    fn a_filter_whose_own_query_is_empty_is_reported_separately() {
        // Distinguishable from the name being unknown, because the fix is a
        // different one: this filter exists and needs a query.
        let filters = BTreeMap::from([("blank", "   ")]);
        let source = RuleSource {
            name: "nudge".to_owned(),
            filter: Some("blank".to_owned()),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        };
        assert_eq!(
            Rule::parse(&source, |name| filters.get(name).copied()),
            Err(RuleError::FilterHasNoQuery {
                filter: "blank".to_owned()
            })
        );
    }

    #[test]
    fn a_rule_with_neither_query_nor_filter_is_rejected() {
        let source = RuleSource {
            name: "empty".to_owned(),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        };
        assert_eq!(Rule::parse(&source, no_filters), Err(RuleError::NoQuery));
    }

    #[test]
    fn naming_both_a_query_and_a_filter_is_rejected_rather_than_resolved() {
        // Two intentions. Picking one silently gives the user whichever this
        // module happened to prefer.
        let filters = BTreeMap::from([("needs-reply", "is:unread")]);
        let source = RuleSource {
            name: "both".to_owned(),
            query: Some("from:ada".to_owned()),
            filter: Some("needs-reply".to_owned()),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        };
        assert_eq!(
            Rule::parse(&source, |name| filters.get(name).copied()),
            Err(RuleError::QueryAndFilter {
                filter: "needs-reply".to_owned()
            })
        );
    }

    #[test]
    fn a_rule_that_would_do_nothing_is_rejected() {
        assert_eq!(
            Rule::parse(&source(&[]), no_filters),
            Err(RuleError::NoActions)
        );
        assert_eq!(
            Rule::parse(&source(&["", "  "]), no_filters),
            Err(RuleError::NoActions),
            "blank entries are not actions, so a list of them is no list"
        );
    }

    #[test]
    fn every_action_in_the_vocabulary_parses_to_its_own_variant() {
        // ADR 0008 Q5's list, asserted one for one rather than as a count:
        // a table that maps two names to one variant passes a count and
        // files mail somewhere nobody chose.
        assert_eq!(
            Action::parse("move:Receipts"),
            Ok(Action::Move("Receipts".into()))
        );
        assert_eq!(
            Action::parse("label:invoices"),
            Ok(Action::Label("invoices".into()))
        );
        assert_eq!(Action::parse("flag"), Ok(Action::Flag));
        assert_eq!(Action::parse("unflag"), Ok(Action::Unflag));
        assert_eq!(Action::parse("mark-read"), Ok(Action::MarkRead));
        assert_eq!(Action::parse("mark-unread"), Ok(Action::MarkUnread));
        assert_eq!(Action::parse("archive"), Ok(Action::Archive));
        assert_eq!(Action::parse("trash"), Ok(Action::Trash));
        assert_eq!(
            Action::parse("forward:ada@example.com"),
            Ok(Action::Forward("ada@example.com".into()))
        );
    }

    #[test]
    fn an_action_argument_is_split_at_the_first_colon() {
        // A mailbox path contains colons and slashes. Splitting at the last
        // one turns `move:Lists/harbour-dev` into a mailbox nobody has.
        assert_eq!(
            Action::parse("move:Lists/harbour-dev"),
            Ok(Action::Move("Lists/harbour-dev".into()))
        );
        assert_eq!(
            Action::parse("label:re: invoices"),
            Ok(Action::Label("re: invoices".into())),
            "everything after the first colon is the argument"
        );
    }

    #[test]
    fn an_action_name_is_case_insensitive_and_trimmed() {
        assert_eq!(
            Action::parse("  MOVE:Receipts  "),
            Ok(Action::Move("Receipts".into()))
        );
        assert_eq!(Action::parse("Mark-Read"), Ok(Action::MarkRead));
    }

    #[test]
    fn an_unknown_action_names_itself_rather_than_being_skipped() {
        assert_eq!(
            Action::parse("teleport:away"),
            Err(RuleError::UnknownAction {
                action: "teleport".to_owned()
            })
        );
        assert_eq!(
            Rule::parse(&source(&["flag", "teleport:away"]), no_filters),
            Err(RuleError::UnknownAction {
                action: "teleport".to_owned()
            }),
            "one bad action fails the rule; the others must not run without it"
        );
    }

    #[test]
    fn an_action_that_needs_an_argument_says_so_when_it_has_none() {
        for action in ["move", "label", "forward", "move:", "label:  "] {
            let name = action.split(':').next().unwrap().trim();
            assert_eq!(
                Action::parse(action),
                Err(RuleError::ActionNeedsArgument {
                    action: name.to_owned()
                }),
                "{action}"
            );
        }
    }

    #[test]
    fn an_action_that_takes_no_argument_says_so_when_it_is_given_one() {
        // `flag:important` reads like it ought to work and does not; saying
        // nothing would file the mail and drop the intent.
        assert_eq!(
            Action::parse("flag:important"),
            Err(RuleError::ActionTakesNoArgument {
                action: "flag".to_owned()
            })
        );
        assert_eq!(
            Action::parse("archive:Old"),
            Err(RuleError::ActionTakesNoArgument {
                action: "archive".to_owned()
            })
        );
    }

    #[test]
    fn forwarding_to_something_that_is_not_an_address_is_caught_here() {
        // `forward:Receipts` is somebody who meant `move:`. Left alone it
        // fails at send time, which is after the message has been handled.
        assert_eq!(
            Action::parse("forward:Receipts"),
            Err(RuleError::ForwardNeedsAnAddress {
                target: "Receipts".to_owned()
            })
        );
    }

    #[test]
    fn stop_and_delete_are_refused_by_name_rather_than_as_unknown() {
        // Both are in ADR 0008's text, so a reader will try them. "Unknown
        // action `stop`" would be a lie about why.
        assert_eq!(Action::parse("stop"), Err(RuleError::StopIsARuleKey));
        assert_eq!(Action::parse("delete"), Err(RuleError::DeleteIsNotAnAction));
    }

    #[test]
    fn every_error_says_what_to_do_about_it() {
        // These reach the settings panel's validity line, where they are all
        // a user gets. An error that names no fix is a bug report addressed
        // to somebody who cannot file one.
        let messages = [
            RuleError::NoQuery.to_string(),
            RuleError::UnknownFilter {
                filter: "nope".into(),
            }
            .to_string(),
            RuleError::NoActions.to_string(),
            RuleError::UnknownAction {
                action: "teleport".into(),
            }
            .to_string(),
            RuleError::StopIsARuleKey.to_string(),
            RuleError::DeleteIsNotAnAction.to_string(),
            RuleError::ForwardNeedsAnAddress {
                target: "Receipts".into(),
            }
            .to_string(),
        ];
        for message in messages {
            assert!(!message.is_empty());
            assert!(
                message
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_lowercase() || c == '`'),
                "reads as a clause after the rule's name: {message:?}"
            );
            assert!(!message.contains('\n'), "one line: {message:?}");
        }
        assert!(RuleError::DeleteIsNotAnAction.to_string().contains("trash"));
        assert!(
            RuleError::ForwardNeedsAnAddress {
                target: "Receipts".into()
            }
            .to_string()
            .contains("move:Receipts"),
            "and points at the action they probably meant"
        );
    }
}
