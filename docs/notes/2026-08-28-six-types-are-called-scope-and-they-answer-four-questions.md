# Six types are called *Scope*, and they answer four questions (2026-08-28, #670)

Before adding a seventh, or before reading a `scope` field and assuming you
know what it holds: the word is heavily overloaded in this workspace, and the
overloads are all legitimate — they are genuinely different questions that
happen to want the same English word.

| Type | Home | Question |
|---|---|---|
| `AccountScope` (re-exported as `postio_core::state::Scope`) | `postio-model` | **Which accounts?** `Unified` or one account. #186 moved it down here from `postio_core::state` so search and the list could not disagree; its doc comment is the best short argument in the tree for moving a type down a crate. |
| `postio_search::facets::Scope` | `postio-search` | **Which slice does a search look at?** `AllMail` / `Inbox` / `Lists` — the canvas's standing, no-typing rescope. Not a mailbox id, deliberately. |
| `ListScope` | `postio-model` (was `postio-runtime::store`) | **Which messages is this view showing?** `Mailbox` / `Account` / `Unified` / `Flagged` / `Snoozed` / `Thread`. What the message list is paged over. `Account` is *one* account's every folder; `Unified` (#185) is every account's, and the two are not spellings of each other. |
| `ViewScope` | `postio-core::state` | **What is a whole-view selection relative to?** `Mailbox` / `Flagged` / `Unified { accounts }`, and no more: `Ctrl+A` is not a gesture inside a thread and nothing needs a `Snoozed` predicate yet. The aggregate carries the accounts it was scoped to, which is why this is the one `*Scope` that is not `Copy` (#811). |
| `FeedScope` | *deleted by #670* | Was `postio-gtk`'s own spelling of `ListScope`. |
| `ScopeFfi` | `postio-ffi` | Not a question — the uniffi ABI mirror of `ListScope`, with `i64` fields. A wire format, the way `ExtCommand` is the owned counterpart of `CommandSpec`. |

**The pair worth understanding is `ListScope` and `ViewScope`**, because they
look like one type spelled twice and are not. `ViewScope` is the *result of a
rule* applied to a `ListScope` — `postio_core::aim::view_scope` — and its
smaller variant set is the point: a `ViewScope` that cannot be constructed
from a thread drill-in is what makes "no whole-view gesture inside a
conversation" a compiler check rather than a conformance table two frontends
have to keep passing. Collapsing them into one type with a predicate would be
one type fewer and a strictly weaker guarantee.

**`ViewScope::Unified` carries a list, and that is the guarantee, not an
accident** (#811). Every other variant is an id. The aggregate is a set of
accounts because the unified list can be showing more accounts than a
selection is *about*: an account Postio cannot reach is still drawn — its
synced mail is real mail, and ADR 0005 Q10 is emphatic that hiding it would
be the worse lie — and a `Ctrl+A` made while it was away is not about it. The
accounts are therefore fixed when the gesture is made and never looked up
again. Resolving them at verb time instead has a hole the other way round: an
account that reconnects between the `Ctrl+A` and the `a` joins a selection
the user was never shown, and a selection that silently *grows* cannot be
spotted in the summary. Two aggregates over different account sets are
different views for the same reason, so one does not inherit the other's
selection.

**The rule of thumb.** A new `*Scope` is warranted when it answers a question
none of the above asks, and it belongs in the lowest crate that all its
readers share — which #186 and #670 both discovered the same way, by finding
a second crate that needed the same value and could not reach it.
