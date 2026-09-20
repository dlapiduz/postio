# Phase 1 Data Model: The Compose Editor

**Plan**: [plan.md](./plan.md) | **Spec**: [spec.md](./spec.md) | **Date**: 2026-09-10

Most of this exists in `postio-model`. What follows records the shape the spec
depends on, marks what changes, and states the rules a test can be written
against. Fields are named as the domain names them, not as a table column.

---

## Draft

The unsent message. Persisted, so it survives a restart (FR-063).

| Field | Meaning | Source |
|---|---|---|
| `to`, `cc`, `bcc` | Recipients, several each | exists |
| `subject` | Free text; a reply prefills it | exists |
| body (formatted) | The authoring document — the restricted HTML subset of ADR 0003 | exists |
| body (plain) | Derived alternative, sent alongside (FR-037) | exists |
| `in_reply_to` | The local message this answers, or none | exists |
| identity | Which of the account's identities it sends as | exists |
| attachments | Files and inline images, see below | exists |
| state | being edited → queued → sent, or failed | exists |

**Rules**

- A draft is editable in exactly one surface at a time (FR-013). The reading
  pane holds at most one; every other open draft is in a detached window
  (FR-010, FR-011).
- Unsent work is saved without being asked, and survives navigating away,
  closing the window, and an unexpected stop (FR-063).
- Reopening restores text, formatting, recipients and attachments (FR-064).
- **Changed by FR-030**: changing the identity does not touch the body. The
  signature already in a draft stays, even if it belongs to the previous
  identity.

**State transitions**

```text
                 save (automatic, FR-047)
   [editing] ─────────────────────────────► [editing, saved]
       │                                            │
       │ send (FR-049: local-first, never awaits)   │ reopen (FR-064)
       ▼                                            ▼
   [queued] ──── grace period, FR-050 ────► [sent]
       │
       │ failure (FR-066)
       ▼
   [failed] ── editable, findable, names what went wrong ──► [editing]
```

Discarding is the one transition that does not reverse, which is why it asks
first (FR-061).

---

## Attachment

A part of the message that is not the body. Two kinds, and the difference is
what the recipient sees (FR-056).

| Field | Meaning |
|---|---|
| name | Filename shown to the user and the recipient |
| size | Bytes, shown in the row (FR-056) |
| type | Media type |
| disposition | **attached** — listed alongside the message; or **inline** — rendered in the body at a point, carried as a `cid:` part |

**Rules**

- Both kinds count against one size total, because the receiving server counts
  both (FR-056). The refusal names the largest items (FR-056).
- An inline image is a real part, so a recipient who blocks remote content still
  sees it (FR-052).
- Removing an attachment removes it from the draft (FR-056).

---

## Quote

**The entity this feature changes.** Today the quote is a `Document` — the
closed authoring type — built by `postio_body::replying::quoted_reply`. Under
FR-044 it becomes the sanitised rendering of the original.

| Field | Meaning |
|---|---|
| attribution | The "On <date>, <sender> wrote:" line (FR-041) |
| content | The original as the reader would render it: sanitised HTML with the sender's styles scoped (FR-044, FR-045) |
| fallback | The text alternative, used when the original has no renderable HTML (FR-045) |
| folded | Whether it is collapsed on open — it is (FR-041) |

**Rules**

- The content may carry only what the reader would render: no script, no
  remote-loading content, no tracking pixels (FR-047).
- The sender's styling is confined to the quote and may not reach the user's
  own text, a nested earlier quote, or Postio's chrome (FR-078).
- What is in the editor is what is sent (FR-046).

**Nesting**: a quote may contain an earlier quote, each with its own styling.
The scoping rule applies at every level, which is the case `styles.rs` was
already built for.

---

## Identity

| Field | Meaning |
|---|---|
| address | What the recipient sees as the sender |
| display name | Shown with it |
| signature | Plain text in v1; HTML signatures are deferred |

**Rules**

- A draft opens carrying the signature of the identity it starts as (FR-030).
- Changing identity updates the sender and **not** the body (FR-016, FR-031).
- No action produces two signatures in one draft (FR-032).

---

## Threading

Not a stored entity so much as a relationship the draft carries, but the spec
depends on it and it has rules.

- A reply carries the threading of what it answers, so it lands in the same
  conversation for the recipient (FR-029) and in Postio's own list without
  waiting for the server (FR-029).
- Preserved across save/reopen and across a failed-and-retried send (FR-029).
- A fresh message starts its own conversation; a forward is not threaded into
  the one it came from (FR-029).

On the wire this is `In-Reply-To` and `References`, already computed in
`postio-model/src/outgoing.rs` from the parent's chain.

---

## What is new

| Change | Where | Requirement |
|---|---|---|
| Quote content becomes sanitised HTML, not a `Document` | `postio-body/src/replying.rs` | FR-042, FR-043 |
| Identity change stops touching the body | `postio-model/src/draft.rs` | FR-030 |
| One size total across attachments and inline images | composer + model | FR-054, FR-055 |
| `undisclosed-recipients:;` for Bcc-only | `postio-model/src/outgoing.rs` | FR-022 |
| Editor document gains a stylesheet | `postio-ui/src/editor/document.rs` | FR-073 to FR-078 |

Everything else in this model exists and is unchanged; it is written down so the
conformance pass has something to assert against.
