//! The list pane's state machine: what to say when there are no rows to
//! show, or when the rows on screen cannot be vouched for.
//!
//! Six named states — inbox zero, offline, sync failure, no matches, a
//! partial aggregate, and the store still opening — each of which "names the
//! local store and gives a key, not a shrug" (canvas 3d). The state is
//! **derived, not stored**: [`derive()`] and [`derive_aggregate`] are pure
//! functions of what the pane knows right now — the [`SyncStatus`], the row
//! count, the local store's counts, the query — so there is no second state
//! to fall out of step with the first, and every rule here can be proven red
//! in a second with no display.
//!
//! It lives here, toolkit-free, so a second frontend gets the same six
//! states and the same precedence between them rather than rederiving
//! either. What stays behind in `postio-gtk`'s `list_state` is the widget:
//! the copy each state draws, and the plate-or-banner rendering that
//! [`State::placement`] decides.

use std::time::Instant;

use postio_core::ConnectionState;

use crate::status::SyncStatus;

/// What the list pane shows in place of rows.
///
/// `None` from [`derive()`] means there are rows to show and the widget should
/// stay out of the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing left to triage in the mailbox in view.
    InboxZero {
        /// When the last sync completed; `None` before the first one.
        last_sync: Option<Instant>,
        /// Messages still in the local store, elsewhere, and searchable.
        stored: u64,
    },
    /// No connection right now; local mail is still fully usable.
    Offline {
        /// Local writes waiting to reach the server.
        queued: u64,
    },
    /// The sync engine cannot reach the server.
    Failing {
        /// The actual error, phrased for the user. Never a shrug.
        reason: String,
    },
    /// A search matched nothing.
    ///
    /// Separate from [`InboxZero`](State::InboxZero) because the mailbox is
    /// not empty — the query is. Telling someone who searched for an invoice
    /// that they have nothing left to triage is a different statement, and a
    /// false one.
    NoMatches {
        /// What was searched for, shown back so what to widen is visible.
        query: String,
        /// Accounts that could not be reached, and so could not be fully
        /// searched. Empty is the ordinary single-account answer.
        ///
        /// ADR 0005 Q10 calls an empty result set the single most important
        /// instance of the omission rule: someone searches for an invoice,
        /// finds nothing, and concludes it does not exist. "Nothing matched"
        /// is a claim about the whole corpus, and a corpus short an account
        /// cannot support it.
        incomplete: Vec<String>,
    },
    /// An aggregate view is showing what it has, and it is not everything.
    ///
    /// ADR 0005 Q10: *a view that cannot include an account says so, names
    /// the account, and stays usable.* Rows from the accounts that did answer
    /// are real mail and stay readable underneath — this is a
    /// [`Placement::Banner`] whenever there is anything to put it over.
    ///
    /// **The rows of a named account are not missing, they are unrefreshed.**
    /// Postio is local-first, so an offline account's synced mail is still in
    /// this list; what cannot be vouched for is that it is current. The
    /// wording says exactly that, because "showing 1 of 2 accounts" would be
    /// its own lie whenever the absent account has mail on disk — which is
    /// almost always.
    Partial {
        /// The accounts that did not answer, in the order the sidebar draws
        /// them — which is the order their hues are keyed to.
        accounts: Vec<String>,
    },
    /// The window is up and the store is not open yet (#1114).
    ///
    /// Postio presents its window before it has opened anything, so this is
    /// the only state in the family that is about the *application* rather
    /// than about mail. It outranks every other: there is no connection
    /// worth describing, no mailbox to be empty, and no query to have
    /// matched nothing, because there is nothing behind the window yet.
    ///
    /// It is also the only one with a threshold — see [`derive_opening`].
    /// An ordinary start never shows it.
    Opening {
        /// What is being waited on, because the four are different waits and
        /// two of them can legitimately take tens of seconds.
        waiting: Waiting,
    },
}

/// What a start that has not finished is actually waiting on.
///
/// Four waits, named separately because "Updating your mailbox's storage" is
/// a different promise from "Opening your mailbox" — and because the two that
/// can legitimately take tens of seconds are the two a person most needs told
/// about. Measured on the live install: a schema migration held a launch for
/// 12.6 s with nothing on screen, and a keyring prompt held another for 28 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// The store key is being read out of the OS keyring.
    ///
    /// A D-Bus round trip against a service that may be showing a passphrase
    /// prompt of its own, which is why this can be the longest of the four
    /// and the one least under Postio's control.
    Keyring,
    /// The encrypted database is being opened.
    Store,
    /// Schema migrations are being applied.
    Migrating,
    /// The local search index is being built or rebuilt.
    Indexing,
}

impl Waiting {
    /// Every wait, in the order a start meets them.
    pub const ALL: [Waiting; 4] = [
        Waiting::Keyring,
        Waiting::Store,
        Waiting::Migrating,
        Waiting::Indexing,
    ];
}

/// How long a start may take before it is worth saying anything.
///
/// Twice `docs/PRODUCT.md` §18's 500 ms budget: by here the start has already
/// failed its own budget, so there is no risk of speaking over an ordinary
/// one. And far enough past the measured ~150 ms store phase that it cannot
/// fire on a healthy launch at all — which matters more than the exact
/// number, because a plate that appears and is gone inside 100 ms is the
/// flicker §18 forbids rather than the reassurance it was meant to be.
pub const OPENING_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(1);

/// What the list pane shows while the store is still opening, if anything.
///
/// `None` below the threshold, and that is the whole of #1114's "normal case
/// — nothing that will disappear": no spinner, no skeleton rows, no
/// "Loading…", no progress of any kind. The ordinary start draws its first
/// frame and then fills it, with nothing in between to be removed.
pub fn derive_opening(waiting: Waiting, waited: std::time::Duration) -> Option<State> {
    (waited >= OPENING_THRESHOLD).then_some(State::Opening { waiting })
}

/// The heading and the line under it for one wait.
///
/// Split out of `describe` so the copy can be asserted on without a
/// display, the same reason [`fn@derive`] is a pure function.
pub fn describe_wait(waiting: Waiting) -> (&'static str, &'static str) {
    match waiting {
        // Named as the keyring rather than as Postio, because what to *do*
        // about it is somewhere else entirely: an unlock prompt that is
        // behind another window, or a keyring that is not running.
        Waiting::Keyring => (
            "Opening your mailbox",
            "Waiting for the keyring to unlock the local store.",
        ),
        Waiting::Store => ("Opening your mailbox", "Reading the local store from disk."),
        // A different promise, and deliberately so: this one changes the
        // store rather than reading it, it is once per upgrade, and it is
        // the wait that has actually taken tens of seconds on a real
        // mailbox.
        Waiting::Migrating => (
            "Updating your mailbox\u{2019}s storage",
            "This happens once after an update, and the mail is not touched.",
        ),
        Waiting::Indexing => (
            "Rebuilding the search index",
            "Your mail is all here; searching it will be ready in a moment.",
        ),
    }
}

/// How much of the pane a [`State`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Opaque, filling the pane. Correct only when there is nothing under it
    /// — an empty mailbox, whatever the reason it is empty.
    Full,
    /// A strip over the top of the rows, which stay visible and scrollable
    /// underneath.
    Banner,
}

impl State {
    /// Whether this state may share the pane with rows, or has to fill it.
    ///
    /// `item_count` is the same count [`derive()`] was given: `InboxZero` never
    /// needs it, since being empty is what put it here in the first place,
    /// but `Offline` and `Failing` can arrive with a full mailbox loaded
    /// underneath, and that is exactly the mail the banner treatment exists
    /// to keep visible.
    pub fn placement(&self, item_count: u64) -> Placement {
        match self {
            // Nothing has been read, so there is nothing underneath to
            // protect: the plate is the pane.
            State::Opening { .. } => Placement::Full,
            State::InboxZero { .. } | State::NoMatches { .. } => Placement::Full,
            State::Offline { .. } | State::Failing { .. } | State::Partial { .. } => {
                if item_count == 0 {
                    Placement::Full
                } else {
                    Placement::Banner
                }
            }
        }
    }
}

/// Which state the list pane shows, from what it knows right now.
///
/// `item_count` is what the loaded page of the windowed model reports for
/// the mailbox in view; `stored` and `queued` come from the local store and
/// do not depend on the server being reachable at all — that is the whole
/// point of "everything already synced still opens."
///
/// `searching` is the query the list is showing results for, or `None` when
/// it is showing a mailbox. It is passed in rather than inferred from the
/// query box, because a box with text in it is not the same thing as a list
/// showing that text's results — the box stays up after `Esc` puts the
/// folder back.
///
/// [`ConnectionState::Connecting`] folds into [`State::Offline`]: from the
/// user's chair both mean "not connected right now, local mail still
/// works," and a fourth named state for a transition that resolves itself
/// would be a state nobody could tell apart from the one before it.
pub fn derive(
    status: &SyncStatus,
    item_count: u64,
    stored: u64,
    queued: u64,
    searching: Option<&str>,
) -> Option<State> {
    // A search answers for itself, ahead of the connection. The index is
    // local and it answered completely, so "Offline — reading local mail"
    // over an empty result set would be true and useless: the local mail is
    // exactly what was just searched. A search that *did* match still gets
    // the connection's banner over its rows, because that is a fact about
    // the rows rather than about the query.
    if let Some(query) = searching.filter(|_| item_count == 0) {
        return Some(State::NoMatches {
            query: query.to_string(),
            // One account, and it is the one that just answered: there is no
            // other account whose absence could have hidden a match.
            incomplete: Vec::new(),
        });
    }
    match status.state {
        ConnectionState::Failing { .. } => Some(State::Failing {
            reason: status
                .detail
                .clone()
                .unwrap_or_else(|| "the server did not say why".to_string()),
        }),
        ConnectionState::Offline | ConnectionState::Connecting => Some(State::Offline { queued }),
        ConnectionState::Online if item_count == 0 => Some(State::InboxZero {
            last_sync: status.last_sync,
            stored,
        }),
        ConnectionState::Online => None,
    }
}

/// Whether an account's contribution to an aggregate can be vouched for.
///
/// Public because it is also what a whole-view selection is scoped by: the
/// accounts the banner does *not* name are exactly the accounts `Ctrl+A` here
/// is about, and two spellings of "reachable" would let the banner and the
/// selection disagree about which account is which (#811).
///
/// [`ConnectionState::Connecting`] is deliberately *not* a reason to name an
/// account. The single-account states fold it into
/// [`State::Offline`] because from the user's chair both mean
/// "local mail still works", and that is right for a whole-pane statement
/// about the one account they are looking at. A banner is a different act: it
/// names an account, and one that appears for the two seconds an account
/// takes to connect — on every launch, for every account — is how people
/// learn to stop reading banners.
pub fn is_current(status: &SyncStatus) -> bool {
    match status.state {
        ConnectionState::Online | ConnectionState::Connecting => true,
        ConnectionState::Offline | ConnectionState::Failing { .. } => false,
    }
}

/// Which state an *aggregate* view shows — the unified list, across accounts.
///
/// ADR 0005 Q10's rule: **a view that cannot include an account says so,
/// names the account, and stays usable.** [`fn@derive`] answers for one account
/// and cannot express this; the difference is not the number of statuses but
/// that a whole-pane "Offline" would be a claim about every account when only
/// one of them is away.
///
/// `accounts` carries one entry per **enabled** account, in the sidebar's own
/// order. That an account disabled by the user simply is not in the list is
/// the whole of Q10's disabled-account rule: it drops out silently and
/// correctly, because the user asked for that, and there is nothing to
/// disclose about a view that is showing what it was told to show.
///
/// The order of the checks is the argument:
///
/// 1. **A search that matched nothing** answers first, and carries the
///    accounts it could not reach — the instance Q10 calls the most
///    important, because "nothing matched" reads as proof.
/// 2. **An account that did not answer** outranks everything else, including
///    inbox zero: "nothing left to triage" is a claim about every account.
/// 3. Otherwise the aggregate behaves as one healthy view does.
pub fn derive_aggregate(
    accounts: &[(String, SyncStatus)],
    item_count: u64,
    stored: u64,
    searching: Option<&str>,
) -> Option<State> {
    let absent: Vec<String> = accounts
        .iter()
        .filter(|(_, status)| !is_current(status))
        .map(|(name, _)| name.clone())
        .collect();

    if let Some(query) = searching.filter(|_| item_count == 0) {
        return Some(State::NoMatches {
            query: query.to_string(),
            incomplete: absent,
        });
    }
    if !absent.is_empty() {
        return Some(State::Partial { accounts: absent });
    }

    // Every account answered, so the aggregate can speak with one voice --
    // and the only thing left worth saying is that there is nothing in it.
    // The oldest last sync across the accounts, because the freshest would
    // overstate how current the view is.
    if item_count == 0 && !accounts.is_empty() {
        return Some(State::InboxZero {
            last_sync: accounts
                .iter()
                .map(|(_, status)| status.last_sync)
                .min()
                .flatten(),
            stored,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn status(state: ConnectionState) -> SyncStatus {
        SyncStatus {
            state,
            ..SyncStatus::default()
        }
    }

    #[test]
    fn an_ordinary_start_says_nothing_at_all() {
        // #1114's first acceptance line, and the whole reason the threshold
        // exists: the measured store phase is tens of milliseconds, and
        // anything drawn and removed inside that is flicker. `PRODUCT.md`
        // §18 allows a transition of ≤100ms *or none*, and this path adds
        // none.
        for waited in [Duration::ZERO, Duration::from_millis(999)] {
            assert_eq!(
                derive_opening(Waiting::Store, waited),
                None,
                "a plate at {waited:?} would be on screen for less time than \
                 it takes to read, and gone before anybody could"
            );
        }
    }

    #[test]
    fn a_start_that_has_already_failed_its_budget_says_what_it_is_waiting_on() {
        // Twice `PRODUCT.md` §18's 500ms budget, and far enough past the
        // measured ~150ms that it can never fire on an ordinary start.
        let plate = derive_opening(Waiting::Migrating, OPENING_THRESHOLD)
            .expect("past the threshold, the wait is worth naming");
        assert_eq!(
            plate,
            State::Opening {
                waiting: Waiting::Migrating
            }
        );
        assert_eq!(
            plate.placement(0),
            Placement::Full,
            "there are no rows behind this one — there is no store to have \
             read any — so there is nothing for a banner to protect"
        );
    }

    #[test]
    fn each_wait_is_named_as_itself() {
        // #1114: "one line of specific copy naming what is being waited on
        // — the keyring read and the database open are different waits and
        // the line should say which." So it is the *line* that has to be
        // unique. The heading is deliberately shared by the two ordinary
        // waits, because both are the same promise to the reader: your
        // mailbox is opening.
        let details: Vec<&str> = Waiting::ALL.iter().map(|w| describe_wait(*w).1).collect();
        let mut unique = details.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            details.len(),
            "two waits say the same thing, so the line names a wait it is \
             not: {details:?}"
        );

        assert_eq!(describe_wait(Waiting::Keyring).0, "Opening your mailbox");
        assert_eq!(describe_wait(Waiting::Store).0, "Opening your mailbox");
        // And the two that change the store rather than reading it promise
        // something else, because they are something else: they are once per
        // upgrade and they are the waits that have actually taken tens of
        // seconds on a real mailbox.
        for different in [Waiting::Migrating, Waiting::Indexing] {
            assert_ne!(
                describe_wait(different).0,
                "Opening your mailbox",
                "{different:?} is not the same promise as opening a mailbox"
            );
        }
    }

    #[test]
    fn an_empty_online_mailbox_is_inbox_zero() {
        let derived = derive(&status(ConnectionState::Online), 0, 4291, 0, None);
        assert_eq!(
            derived,
            Some(State::InboxZero {
                last_sync: None,
                stored: 4291,
            })
        );
    }

    #[test]
    fn a_search_that_matched_nothing_does_not_claim_the_inbox_is_clear() {
        // The same inputs that make an empty mailbox `InboxZero`. What
        // changes the answer is that the emptiness belongs to the query.
        let derived = derive(
            &status(ConnectionState::Online),
            0,
            4291,
            0,
            Some("from:ada invoice"),
        );
        assert_eq!(
            derived,
            Some(State::NoMatches {
                query: "from:ada invoice".to_string(),
                incomplete: Vec::new(),
            })
        );
        // Nothing underneath it to keep visible.
        assert_eq!(derived.unwrap().placement(0), Placement::Full);
    }

    #[test]
    fn a_search_answers_for_itself_whatever_the_connection_is_doing() {
        // The index is local and it answered completely, so a connection
        // state over an empty result set would be true and useless -- the
        // local mail is exactly what was just searched.
        for state in [
            ConnectionState::Offline,
            ConnectionState::Connecting,
            ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
        ] {
            assert!(
                matches!(
                    derive(&status(state), 0, 4291, 2, Some("invoice")),
                    Some(State::NoMatches { .. })
                ),
                "{state:?} spoke over the search"
            );
        }
    }

    #[test]
    fn a_search_that_found_something_still_hears_about_the_connection() {
        // The banner is a fact about the rows, not about the query, so
        // finding hits does not silence it.
        assert_eq!(
            derive(
                &status(ConnectionState::Online),
                14,
                4291,
                0,
                Some("invoice")
            ),
            None,
            "a search with hits invented a state of its own"
        );
        let derived = derive(
            &status(ConnectionState::Offline),
            14,
            4291,
            2,
            Some("invoice"),
        );
        assert_eq!(derived, Some(State::Offline { queued: 2 }));
        assert_eq!(
            derived.unwrap().placement(14),
            Placement::Banner,
            "the hits were hidden behind the connection"
        );
    }

    #[test]
    fn a_populated_online_mailbox_has_no_named_state() {
        assert_eq!(
            derive(&status(ConnectionState::Online), 12, 4291, 0, None),
            None
        );
    }

    #[test]
    fn offline_is_the_state_regardless_of_how_many_rows_are_loaded() {
        // "Everything already synced still opens" is true whether the
        // mailbox in view is empty or not; the point is the connection, not
        // the count. Whether that turns into a full plate or a banner is
        // `State::placement`'s decision, not `derive`'s — see the
        // `placement` tests below, which is where `postio-ma4` actually
        // lived: this state was always right, only how much of the pane it
        // took was wrong.
        assert_eq!(
            derive(&status(ConnectionState::Offline), 12, 0, 2, None),
            Some(State::Offline { queued: 2 })
        );
    }

    #[test]
    fn only_an_empty_mailbox_gets_the_full_opaque_plate() {
        let offline = State::Offline { queued: 2 };
        let failing = State::Failing {
            reason: "IMAP rejected the credentials.".to_string(),
        };

        // `postio-ma4`: offline or failing with rows already loaded must not
        // hide mail that is synced and readable — canvas 3d's "nothing is a
        // dead end" and CLAUDE.md's "everything already synced still opens"
        // are both broken by an opaque plate over rows that are right there.
        assert_eq!(offline.placement(12), Placement::Banner);
        assert_eq!(failing.placement(12), Placement::Banner);

        // Nothing underneath to hide: the full plate is the right answer,
        // not a banner floating over an empty pane.
        assert_eq!(offline.placement(0), Placement::Full);
        assert_eq!(failing.placement(0), Placement::Full);
    }

    #[test]
    fn inbox_zero_is_always_the_full_plate() {
        // True by construction -- `derive` only ever produces `InboxZero`
        // when `item_count` is already 0 -- but the state's own rule should
        // not silently depend on that invariant holding elsewhere.
        let empty = State::InboxZero {
            last_sync: None,
            stored: 0,
        };
        assert_eq!(empty.placement(0), Placement::Full);
    }

    #[test]
    fn connecting_reads_the_same_as_offline() {
        assert_eq!(
            derive(&status(ConnectionState::Connecting), 0, 0, 0, None),
            Some(State::Offline { queued: 0 })
        );
    }

    #[test]
    fn a_failing_connection_never_shrugs() {
        let with_reason = SyncStatus {
            state: ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
            detail: Some("AUTHENTICATIONFAILED".to_string()),
            ..SyncStatus::default()
        };
        assert_eq!(
            derive(&with_reason, 0, 0, 0, None),
            Some(State::Failing {
                reason: "AUTHENTICATIONFAILED".to_string(),
            })
        );

        let without_reason = status(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        });
        let State::Failing { reason } = derive(&without_reason, 0, 0, 0, None).unwrap() else {
            panic!("failing status did not produce a failing state");
        };
        assert!(!reason.is_empty(), "a failing state never shows nothing");
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;

    fn status(state: ConnectionState) -> SyncStatus {
        SyncStatus {
            state,
            ..SyncStatus::default()
        }
    }

    fn named(entries: &[(&str, ConnectionState)]) -> Vec<(String, SyncStatus)> {
        entries
            .iter()
            .map(|(name, state)| ((*name).to_owned(), status(*state)))
            .collect()
    }

    #[test]
    fn every_account_online_says_nothing_at_all() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            None,
            "a complete view has nothing to disclose, and a banner that is \
             always up is a banner nobody reads"
        );
    }

    #[test]
    fn an_unreachable_account_is_named_over_the_rows_it_could_not_refresh() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 40, 4291, None);
        assert_eq!(
            derived,
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            }),
            "ADR 0005 Q10: the view names the account it cannot vouch for"
        );
        assert_eq!(
            derived.unwrap().placement(40),
            Placement::Banner,
            "the rows are real mail and stay readable -- covering them to say \
             the view is incomplete keeps the promise in words and breaks it \
             on screen"
        );
    }

    #[test]
    fn a_failing_account_counts_as_one_it_cannot_vouch_for_too() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            (
                "Personal",
                ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
            ),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            })
        );
    }

    #[test]
    fn connecting_is_not_worth_naming_because_it_resolves_itself() {
        // The single-account states fold `Connecting` into `Offline`, because
        // from the user's chair both mean "local mail still works". A banner
        // is different: it names an account, and one that appears for the two
        // seconds an account takes to connect -- on every launch, for every
        // account -- teaches people to stop reading banners.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Connecting),
        ]);
        assert_eq!(derive_aggregate(&accounts, 40, 4291, None), None);
    }

    #[test]
    fn every_unreachable_account_is_named_in_the_order_given() {
        let accounts = named(&[
            ("Work", ConnectionState::Offline),
            ("Personal", ConnectionState::Online),
            ("Archive", ConnectionState::Offline),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 40, 4291, None),
            Some(State::Partial {
                accounts: vec!["Work".to_owned(), "Archive".to_owned()],
            }),
            "naming one of two absent accounts is its own lie by omission, \
             and the order is the sidebar's so the colours line up"
        );
    }

    #[test]
    fn a_search_that_found_nothing_while_an_account_is_away_says_so() {
        // ADR 0005 Q10 calls this the single most important instance of the
        // rule: someone searches for an invoice, finds nothing, and concludes
        // it does not exist. `NoMatches` on its own is a claim about the
        // whole corpus, and here the corpus is short an account.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 0, 4291, Some("invoice"));
        assert_eq!(
            derived,
            Some(State::NoMatches {
                query: "invoice".to_owned(),
                incomplete: vec!["Personal".to_owned()],
            }),
            "an empty result set has to carry the accounts it could not \
             search fully, or it reads as proof the mail is not there"
        );
    }

    #[test]
    fn a_search_that_found_nothing_with_everything_online_is_a_plain_no_match() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 0, 4291, Some("invoice")),
            Some(State::NoMatches {
                query: "invoice".to_owned(),
                incomplete: Vec::new(),
            }),
            "nothing to disclose, so the answer is the same one a single \
             account gives"
        );
    }

    #[test]
    fn an_empty_aggregate_with_everything_online_is_still_inbox_zero() {
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Online),
        ]);
        assert_eq!(
            derive_aggregate(&accounts, 0, 4291, None),
            Some(State::InboxZero {
                last_sync: None,
                stored: 4291,
            })
        );
    }

    #[test]
    fn an_unreachable_account_outranks_inbox_zero_when_there_are_no_rows() {
        // "Nothing left to triage" is a claim about every account, and one of
        // them did not answer. With nothing underneath, it takes the plate.
        let accounts = named(&[
            ("Work", ConnectionState::Online),
            ("Personal", ConnectionState::Offline),
        ]);
        let derived = derive_aggregate(&accounts, 0, 4291, None);
        assert_eq!(
            derived,
            Some(State::Partial {
                accounts: vec!["Personal".to_owned()],
            })
        );
        assert_eq!(derived.unwrap().placement(0), Placement::Full);
    }

    #[test]
    fn a_disabled_account_never_reaches_here_so_it_is_never_named() {
        // ADR 0005 Q10: `enabled = 0` drops out of Unified silently and
        // correctly, because the user asked for that. It is expressed as the
        // caller passing only enabled accounts -- an empty list is a view
        // with nothing to disclose rather than one that is degraded.
        assert_eq!(derive_aggregate(&[], 0, 0, None), None);
    }
}
