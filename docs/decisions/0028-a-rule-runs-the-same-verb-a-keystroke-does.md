# ADR 0028 — A rule runs the same verb a keystroke does, from one storage-level implementation

- **Status:** Accepted (2026-09-04)
- **Date:** 2026-09-04
- **Decision by:** `/ux-architect`, on [#481](https://github.com/dlapiduz/postio/issues/481), which the maintainer routed here after a session claimed it, found two questions in the way, and released it without writing code.
- **Issue:** [#481](https://github.com/dlapiduz/postio/issues/481)
- **Amends:** [ADR 0008](0008-filters-and-rules.md) Q5, whose `forward` guards are restated — one of them was transcribed inverted into #481 — and whose Consequences gain the seam a rules pass in `postio-sync` needs in order to obey the last paragraph of that same Q5.
- **Related:** [ADR 0005](0005-multiple-accounts.md) Q9 (the cross-account saga a rule never reaches), `ARCHITECTURE.md` §1 (local-first) and §11 (did the user ask for it), `PRODUCT.md` §21 (privacy), [#5](https://github.com/dlapiduz/postio/issues/5) (the rules epic), [#480](https://github.com/dlapiduz/postio/issues/480) (the typed `Rule`, landed), [#482](https://github.com/dlapiduz/postio/issues/482) (the two evaluation points)
- **Decision:** **the mutating half of `postio_session::actions` moves down into `postio-storage`, and both the command bus and the rules pass call it.** One implementation of "trash this message" writes the row, enqueues `Operation::Delete { from, trash }`, and knows nothing about who asked. And **`forward` keeps ADR 0008 Q5's guard, not #481's**: arbitrary targets, refusing the user's own account addresses — with the target constrained to be a literal, which is what actually answers the threat #481 was reaching for.

---

## Q1 — Where the executor lives: below both callers, in `postio-storage`

ADR 0008 Q5 already decided the requirement, in a sentence that reads as a
throwaway and is not:

> **Every action is local-first, exactly like a keystroke** (`ARCHITECTURE.md` §1):
> SQLite write, enqueue the remote operation, emit the event. **There is no
> rules-only mutation path**, which means rules inherit offline behaviour,
> reconciliation and event flow for free.

That forecloses the first of #481's three shapes. Re-implementing the verbs
inside `postio-sync` against the same repositories *is* a rules-only mutation
path; it satisfies "existing repositories" and fails "no second
implementation", and the parts that would drift are named ones —
`Operation::Delete { from, trash }` rather than a plain move, the undo entry,
the events the panes repaint from.

The second shape — move the pass up into `postio-session` and have
`postio-sync` merely announce an arrival — is foreclosed by ADR 0008 Q3:

> Header-only rules run in the sync pass that inserts the message, **in the
> same transaction as the insert, before any event is emitted.** The user never
> sees it land in the Inbox first.

Across a crate seam that is not a transaction any more. The whole value of
running on arrival is that the mail is already filed the first time it is
visible.

So the third shape is the only one the accepted ADR leaves open, and the
useful contribution here is that **it is much cheaper than #481's comment
feared.** It reads as "largest change" because `postio_session::actions` is
2,500 lines in the composition root. Most of that is not the mutation.

**What a verb actually needs.** `relocate_rows` — the shared body behind
archive, move and trash — reaches for `MailboxRepository`,
`MessageRepository`, `OperationQueueRepository`, `Operation`, and its own
`Destination`. Every one of those already lives in `postio-storage` or
`postio-model`, and `postio-sync` already depends on both. What is *not*
storage is thin and separable: `UndoKind` (which entry to push),
`CommandError` (how a bus reports a refusal), `Applied` (what the panes
repaint from) and the resolution of "what is this command aimed at" against
`SharedState`.

**So the seam is:**

| Half | Crate | Knows about |
|---|---|---|
| Write the rows, enqueue the operation | `postio-storage` | message ids, mailboxes, `Operation` |
| Resolve the target from the view, push undo, emit | `postio-session` | `Command`, `SharedState`, `UndoStack`, `EventSink` |

`postio_session::actions` keeps its shape and its module docs; its verbs become
"resolve, call the storage verb, record, emit". The rules pass in `postio-sync`
calls the same storage verb with an explicit message id, and does neither of
the other two.

**No new crate, and no cycle.** `postio-storage` gains nothing it did not
already have — it is the crate where SQLite is allowed, `OperationQueueRepository`
already lives there, and enqueueing beside the write it belongs to is the
layer it belongs in. Nothing in `postio-storage` learns what a `Command` is.
Lifting the verbs into a *new* crate below both was the obvious reading of
"lift them below" and is rejected: the workspace pays for every crate in
compile graph, boundary checks and CI, and the only thing a new crate would
buy over `postio-storage` is a name.

### The three things that will go wrong if this lands carelessly

**1. The storage verb must run inside a transaction it did not open.**
`relocate_rows` calls `connection.transaction()` itself today, and it must:
the local write and its queue row have to commit together, with the queue row
first because enqueue snapshots server coordinates the move then nulls (#289).
But ADR 0008 Q3 requires the rule's action to be in *the same transaction as
the insert*, which the sync pass owns. So the storage verb takes a
`&Transaction`, and the interactive path opens one and passes it in. A verb
that opens its own transaction cannot be called from a rule at all, and
discovering that after the move is a rewrite of every verb rather than of one
signature.

**2. A rule pushes no undo entry, and this is deliberate.** Undo walks back
through *the user's* history; a rule firing during a sync is not in it, and an
`u` that reverses something the user never saw is worse than no undo. It costs
nothing in recoverability, because ADR 0008 Q5 already refuses `delete` to a
rule — nothing a rule does is unrecoverable, and every one of its effects is
reversible by the ordinary verb on the ordinary message.

It does leave a real gap, named here rather than hidden: **a rule that files
mail wrongly is discovered by not finding the mail.** The honest fix is that a
message should be able to say which rule touched it, and that is a feature
larger than this issue — filed separately rather than smuggled in: [#1118](https://github.com/dlapiduz/postio/issues/1118).

**3. Cross-account never arises.** `relocate_rows` falls through to ADR 0005
Q9's three-phase saga when the destination belongs to another account. A rule's
`move:` names a mailbox within the account the rule is scoped to, so the
storage verb keeps the branch and the rules pass never reaches it. If rules
ever gain a cross-account target, that is a new decision and not an accident of
reuse.

## Q2 — Who a rule may forward to: ADR 0008 Q5 stands, and #481's text is the error

#481 and ADR 0008 Q5 say opposite things, and only one of them is a decision.

| | target |
|---|---|
| ADR 0008 Q5 | anywhere, **except** an address of a configured account |
| #481's body | **only** an address of a configured account |

**Decision: the ADR.** #481's text is corrected rather than honoured.

The ADR's guard is about **loops**: forwarding to an address that lands back
in your own account re-triggers the rule set on arrival, which is the failure
that turns one message into a mailbox full of them. #481's guard is about
**exfiltration** — "never an arbitrary address a rule can be tricked into
typing" — and it is aimed at a threat that does not survive being written down:

- **Nothing tricks a rule into typing an address, because the target is not
  computed.** A `forward:` target is a literal in the rule. It is not
  interpolated from the message, not taken from `Reply-To`, not derived from
  anything the sender controls. Adding that constraint explicitly — **the
  target is a constant; there is no templating, now or later** — removes the
  entire class the guard was reaching for, at a cost of nothing, because
  nobody asked for a computed target.
- **The remaining attacker is one who can write `config.toml`, and they have
  already won.** Anything that can write the user's config runs as the user,
  and the store's key is in the same session's keyring — so it can read the
  mail directly. A guard that stops it forwarding mail it can already read is
  not a guard.

That second point has one honest asymmetry, and it is the reason this is
written down rather than assumed: a forward rule is a *persistence* primitive
where reading the store is a one-off. It buys future mail without maintaining
access. That is real, and it is answered by visibility rather than by
prohibition — ADR 0008 Q5 already says how, and the sentence deserves to be
read as a requirement rather than as reassurance:

> A forwarded message appears in Sent like any other, because the send goes
> through the ordinary operation queue. **It is not invisible.**

Two things make that true rather than hopeful, and both are acceptance
criteria: the forward is an ordinary queued send, so it appears in Sent; and
the rules view in the settings panel shows each rule's target **plainly**,
not elided and not behind a disclosure. A rule the user cannot see the target
of is the only version of this that is dangerous.

**Rejected: requiring a confirmation for a target outside the configured
accounts** (#481's third option). It is the option that sounds most careful
and it buys the least. Postio has exactly one modal, and this is not it; the
consent it asks for is the consent the user already gave by writing the rule;
and a rule that is written, valid, and silently inert until acknowledged
somewhere else is a dead end of the kind canvas 3d exists to forbid. If the
config file is not a sufficient expression of intent, the answer is not a
second dialog on top of it.

**Rejected: own-accounts-only, as #481's body has it.** Coherent, safe, and it
deletes the feature. Forwarding to yourself is what a second account's
`move:` already does better. If this were the policy, the honest conclusion is
the one #481's own commenter drew: drop `forward` and let `move:` and `label:`
carry the weight.

### The three guards, restated whole

Unchanged from ADR 0008 Q5 except where marked, and each one independently
tested per #481's acceptance list:

1. **No forwarding a message a rule already forwarded** — a Postio-set header,
   checked on arrival. This is the loop guard for mail that comes back by
   another path.
2. **The target is refused if it is an address of any configured account** —
   the loop guard for the direct case. **And the target is a literal**
   (new here): no interpolation from the message, ever.
3. **Rate-capped per rule per hour**, and hitting the cap raises `Attention`
   rather than dropping the mail (ADR 0008 Q6: errors never drop mail).

Guards 1 and 3 are about loops and volume, **not** about exfiltration. Saying
so is the point of writing this down: a future reader who believes they are
security guards will either weaken them carelessly or refuse a reasonable
change because they look load-bearing for something they were never holding.

## Consequences

- `postio-storage` gains the mutating half of the verbs, taking a
  `&Transaction` rather than opening one. `postio_session::actions` keeps the
  command-shaped half and calls it. No crate is added; no boundary check
  changes.
- `postio-sync` gains the rules pass ADR 0008's Consequences already promised,
  and it calls the storage verbs — there is no second implementation of any
  verb, which is what `trash` routing through the recoverable flow is proved
  by rather than tested for.
- A rule's action pushes no undo entry and resolves nothing against
  `SharedState`.
- `forward` ships with arbitrary targets and the three guards above. #481's
  body is corrected; ADR 0008 Q5 gains the literal-target sentence and a note
  that guards 1 and 3 are loop and volume guards.
- The settings panel's rules view shows each rule's forward target in full.
- Filed separately: a message should be able to say which rule touched it,
  which is what makes a misfiling discoverable rather than mysterious (#1118).
