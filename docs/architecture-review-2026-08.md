# Postio architecture review — August 2026

Reviewer's note: this is an assessment, not a work order. Nothing here is
implemented. Each finding ends with a suggested shape and a rough cost so the
maintainer can decide what is worth doing and when.

---

## Verdict

The load-bearing decisions are right, and several of them are better than they
need to be for a v1. Specifically:

- **`postio-core` is genuinely UI-agnostic.** Commands in, events out, no GTK
  type anywhere, and CI proves the closure is clean rather than trusting a
  grep. The tokio↔glib bridge is confined to one module that names no GTK type.
- **The registry-as-single-source-of-truth is the best idea in the codebase.**
  Keymap, `Ctrl+K` palette, `?` cheat sheet, context menu and row key hints are
  all *derived* from one table. Most mail clients have four drifting lists.
- **Local-first is structural, not aspirational.** Handlers write SQLite,
  enqueue an operation and return; the bus awaits each handler so undo sees a
  total order without a lock. The UI genuinely never awaits the network.
- **The `MailBackend` seam plus the feature-gated `postio-imap` is a real
  achievement.** `postio-sync` builds with `default-features = false` and the
  pre-1.0 protocol crate is not in its graph at all.

The gaps below are almost all of one kind: **the code is portable, but the
crate graph does not let anyone else use it.** Logic that has no GTK in it sits
inside crates that link GTK. That costs nothing today, on a Linux-only v1. It
is the entire cost of the second platform.

---

## 1. The `postio-search` question is already answered — the doc is stale

The tree in `CLAUDE.md` and the README shows `postio-search` indented under
`postio-gtk` with the description "FTS5 index, query-operator parser". **That
is no longer what the code does.** The split already happened:

| Crate | Lines | Depends on | What it is |
|---|---|---|---|
| `postio-search` | 1,990 | `postio-model`, `chrono` | Parser, query AST, highlighter, facets, date/size grammar, result types |
| `postio-index` | 1,280 | `postio-search`, `rusqlite` | The FTS5 index and the executor |

`postio-search` has **no `rusqlite`, no `gtk`, no I/O of any kind**. It is a
pure leaf on `postio-model`. And it is not a child of anything — four crates
depend on it: `postio-gtk`, `postio-index`, `postio-runtime`, `postio-app`.

So the instinct behind the question is correct, and the fix is already in the
tree. What is wrong is the **drawing**. The ASCII tree uses indentation to mean
"depends on", which forces every shared crate to be drawn under exactly one
parent and makes shared leaves look like private children. `postio-search`
looks like a GTK detail; `postio-index` does not appear at all.

**Suggested fix (documentation only):** redraw as layers rather than a tree, so
a crate's position states its rank and shared leaves are visibly shared. See
§8 for a proposed diagram.

**Cost:** an afternoon of doc editing. No code moves.

---

## 2. Portable presentation logic is trapped inside `postio-gtk`

`postio-gtk` is 24,764 lines — by a wide margin the largest crate. Most of that
is legitimately GTK: widgets, subclasses, CSS, `glib::SourceId` timers. But a
meaningful slice has no GTK in it at all and is exactly what a second frontend
would need first:

| File | Lines | GTK references |
|---|---|---|
| `keymap.rs` | 1,413 | **one function** — `Chord::from_key_event(gdk::Key, gdk::ModifierType)` at `keymap.rs:298` |
| `tokens.rs` | 878 | zero — `std` only, by design |
| `selection.rs` | 352 | zero |
| `reader/sanitize.rs` | 319 | zero |
| `reader/quote.rs` | 238 | zero |
| `reader/allowlist.rs` | 204 | ten, all peripheral |
| **Total** | **~3,400** | |

This is the keyboard model, the design system, the selection semantics, the
HTML sanitizer and the quote folder — the parts of a mail client that are hard
to get right and have nothing to do with a toolkit. `keymap.rs` even documents
that "nothing here touches a widget, which is what lets it be unit-tested with
no display and no GTK main loop." That is true of the module and false of the
crate it ships in.

A macOS or Windows frontend has three options today, and all three are bad:
reimplement 3,400 lines of subtle logic, depend on `postio-gtk` and link GTK on
macOS, or fork. The `Selection::Everything { except }` predicate in particular
is a correctness invariant (never materialise a 100k mailbox) that must not be
re-derived by a second frontend from scratch.

`tokens.rs` deserves separate mention. It parses the Industry design system and
emits **GTK CSS**. The parse half is universal; the emit half is per-toolkit. A
second frontend wants the same tokens as, say, an AppKit colour table. Those
two halves are already separate functions (`parse()` / `generate()`) — they are
just not separately consumable.

**Suggested shape:** a new `postio-ui` crate holding the platform-neutral
presentation layer, depending on `postio-core` + `postio-model` and *nothing
toolkit-shaped*. Move `selection`, `sanitize`, `quote`, `allowlist`, `tokens`
(parse; keep `generate_gtk_css` in `postio-gtk`), and `keymap`.

The keymap needs one small piece of work to move: a neutral key type. Today
`Chord::from_key_event` takes `gdk::Key` and `gdk::ModifierType`. Define
`postio_ui::Key` / `Modifiers` and have `postio-gtk` provide
`impl From<gdk::Key> for postio_ui::Key`. The keymap already parses bindings
from **strings** (`"a"`, `"ctrl+k"`, `"g g"`) because `[keys]` in `config.toml`
is a string format — so the neutral representation already exists and is
already the canonical one. This is a genuinely small change for how much it
unlocks.

**Cost:** moderate and mostly mechanical. The one design decision is the key
type. Add a `postio-ui` rule to `check-crate-boundaries.py` at the same time,
banning `gtk4`/`libadwaita`/`rusqlite`, so it cannot silently regress.

---

## 3. The composition root is GTK-bound, and it holds the message verbs

`postio-app` is described as "the composition root: opens the store, starts the
engine, runs the UI." Those are two jobs, and the crate does not separate them.
It depends on `gtk4` and `libadwaita` directly, so *everything* in it links GTK.

Measured by GTK references:

| GTK-free (the engine assembly) | Lines | | GTK-bound (the frontend) | Lines |
|---|---|---|---|---|
| `actions.rs` | 1,452 | | `lib.rs` | 549 |
| `refresh.rs` | 229 | | `search.rs` | 567 |
| `engine.rs` | 105 | | `onboarding.rs` | 460 |
| `paths.rs` | 93 | | `compose.rs` | 428 |
| | | | `notifications.rs` | 300 |
| **~1,880** | | | `commands.rs`, `feed.rs`, `main.rs` | ~500 |

`actions.rs` is the important one. It is **the entire message verb vocabulary**
— archive, delete, move, flag, mark-unread, and the undo replay that inverts
them — written against `postio-storage` and `postio-core` with zero GTK
references. Its own module docs explain why it lives here: "a handler needs the
store, and `postio-core` is not allowed to know what SQLite is. `postio-gtk` is
not allowed to either. This crate is the one that knows both halves exist."

That reasoning is correct. The conclusion is one crate too coarse. "Knows both
halves exist" and "runs GTK" are different privileges, and only the second one
needs to link a toolkit.

The consequence is the same as §2 but worse, because these are the verbs: a
second frontend must either duplicate the archive/undo semantics or link GTK to
borrow them. And undo correctness — inverses replayed with `Recording::Replay`
so `u u` walks back rather than toggling — is precisely the kind of thing that
must exist once.

**Suggested shape:** split `postio-app` in two.

- **`postio-session`** — headless. Opens the store, builds the `Dispatcher`,
  wires `actions`/`refresh`, starts the engine, owns paths. Depends on
  `postio-runtime` + `postio-core`. No toolkit. Roughly today's `actions.rs`,
  `engine.rs`, `refresh.rs`, `paths.rs`.
- **`postio-app`** — the GTK binary. Builds a session, hands its
  `CommandSender`/`EventStream` to `postio-gtk`, runs the main loop.

This split pays for itself three times over: it is the second-platform seam,
it is the MCP/AI seam (§5), and it is the CLI/test seam. The integration tests
in `crates/postio-app/tests/` that `postio-bl2` built — proving the composition
root is testable without a GUI — would move to `postio-session` and stop
needing a GTK link to run.

**Cost:** moderate. Mostly moving files and adjusting `use` paths; the code
being moved is already GTK-free, which is what makes it tractable.

---

## 4. The command vocabulary is closed — this is the extensibility wall

`CommandId` is generated by the `command_ids!` macro in `command.rs` as a
closed enum. `Dispatcher` is a `HashMap<CommandId, Handler>`. `registry::all()`
returns `&'static [CommandSpec]` with `&'static str` titles.

For built-in commands this is **excellent** and should not be given up:

- exhaustive `match` means a new command cannot be silently unhandled;
- `CommandId` serialises as a stable string, so `[keys]` in `config.toml` is a
  real file format with a test holding it to `DEFAULT_BINDINGS`;
- `destructive` + `recovery` are machine-checked — a destructive command with
  no `Recovery` fails the suite.

But it means **no command can exist without recompiling `postio-core`.** For
the stated roadmap — MCP, AI support, extensibility — that is the wall. An MCP
tool, a user script, a plugin-contributed action and an AI-proposed operation
are all "a thing that can be done", and the registry is the only place in this
architecture where that concept lives. Today none of them are representable.

Worth being precise about what breaks: it is not just dispatch. The palette,
the cheat sheet, the context menu and the key hints are *generated from the
registry*. A command outside the registry is not merely unbound — per the docs,
it "does not exist". So an extension mechanism that bypasses the registry
inherits none of the discoverability that makes this app good, and users would
find extension commands second-class in exactly the surfaces that matter.

**Suggested shape:** keep the closed enum, add a namespaced escape hatch beside
it rather than replacing it.

- `CommandId::Ext(ExtId)` where `ExtId` is a namespaced interned string
  (`"mcp:summarise-thread"`, `"user:file-to-receipts"`). Namespacing keeps
  built-in ids collision-free forever and makes provenance visible in the
  palette and in logs.
- `registry::all()` becomes static ∪ dynamic, so extension commands appear in
  the palette and cheat sheet on the same footing — and are bindable from
  `[keys]` with no new syntax, since bindings already refer to commands by
  string.
- `CommandSpec` grows an owned-string variant (`Cow<'static, str>`) for title.
  This is also what unblocks **i18n**, which `&'static str` titles currently
  make impossible — worth noting as a second reason to do it once.
- Extension handlers register through the existing `DispatcherBuilder`; the
  fallthrough for an unregistered `Ext` is the `CommandError::rejected` path
  that already exists.

**Design constraint to hold:** `destructive` and `recovery` must be
**mandatory** on extension specs too. The invariant that a destructive command
has a recovery is currently enforced by a test over a static table; for dynamic
specs it has to move into the registration call so it cannot be skipped. An AI-
or plugin-invoked destructive action with no undo is a much worse failure than
a built-in one, because the user did not type it.

**Cost:** substantial, and the highest-leverage item on this list. Best done
*before* MCP/AI work starts, not alongside it — retrofitting a vocabulary is
far more expensive than widening it while there is one consumer.

---

## 5. There is no way to await the result of a command

`CommandSender::send` returns immediately and results arrive as `Event`s on a
broadcast stream. `Invocation` carries `command` and an `EventSink`, and
**neither `Command` nor `Event` carries a correlation id**.

For a GTK frontend this is exactly right — it is what keeps the UI off the
network, and a repaint does not care which keystroke caused it.

For a programmatic caller it does not work. An MCP tool call, an AI agent step
and a CLI subcommand all need request/response: *did my archive succeed?* Today
a caller can only watch the global event stream and guess by shape and timing,
which is unreliable the moment two commands are in flight, or the sync engine
emits a `MessagesChanged` for unrelated reasons — which it does constantly.

**Suggested shape:** an optional correlation token threaded through the
existing path — `send_tracked(cmd) -> InvocationId`, `Invocation::id()`, and an
`origin: Option<InvocationId>` field on the events a handler emits. The GTK
frontend ignores it and nothing changes for it. A programmatic caller filters
the stream by its own id. This is deliberately additive: no existing call site
has to change.

Worth doing at the same time as §4 — they are the same consumer's requirements,
and both touch the `Command`/`Event` vocabulary.

**Cost:** small if done before there are many handlers. Grows with every
handler added.

---

## 6. The rich text composer is the least portable thing you will build

Today `composer.rs` builds the body from a `gtk::TextView` (`composer.rs:279`)
— plain text. `MessageBody { text: Option<String>, html: Option<String> }` in
the model is already the right shape to grow into, which is good.

Two cautions, both about where the document lives:

**Do not let the toolkit's text buffer become the source of truth.** A
`GtkTextBuffer`, an `NSTextStorage` and a `contenteditable` DOM have genuinely
different models of what a rich text document *is* — they disagree about
attribute runs versus nested spans, about what an undo step is, and about list
and blockquote nesting. If the composer's state is "whatever is in the
`TextBuffer`", the macOS composer is a rewrite rather than a port, and the two
will produce subtly different HTML from the same user gestures. Model the
document in `postio-model` (or `postio-ui`), let each platform's editor be a
*view* over it, and serialise to HTML from the neutral model.

**Sanitisation is bidirectional.** The reader is well defended — `ammonia`,
JavaScript off, network off, `cid:` from the local blob store. Outgoing HTML
needs its own discipline for a different reason: a reply quoting a hostile
message will re-emit that message's markup into the world. The `reader/quote.rs`
and `reader/sanitize.rs` modules are the natural home for the shared half,
which is another reason to make them reachable from outside `postio-gtk` (§2).

**Cost:** the document model is a real design task. Cheap now; expensive after
a GTK composer has grown features.

---

## 7. Smaller notes

**Single account is hardcoded above the model.** `postio-app/src/lib.rs:538`
is `first_account()`. `postio-model`, `postio-storage` and the engine are all
account-aware (`Engine::spawn` takes an `account`), so the constraint lives
only in the composition root — which is the right place for it and an
appropriate MVP cut. One thing to watch: `AppState` should become
account-scoped before much more state accumulates in it, because "which
account is this selection in" is the kind of question that is cheap to answer
early and expensive to retrofit through every handler.

**Boundary rules cover two crates, not the graph.** `check-crate-boundaries.py`
guards `postio-core` and `postio-gtk`. It has no rule preventing
`postio-search` from re-acquiring `rusqlite` (undoing the §1 split), nothing
keeping `postio-model` pure, and — if §2/§3 happen — nothing would guard
`postio-ui` or `postio-session`. The script's mechanism is good; it is the rule
table that is thin. Adding rules is cheap and each one is a regression that
cannot happen again.

**`&'static str` in `CommandSpec` blocks localisation.** Noted in §4; flagging
separately because it is a product constraint, not only an extensibility one.

---

## 8. Suggested diagram

Layers rather than a tree, so shared leaves stop looking like private children.
Everything below the line is what a second frontend reuses unchanged.

```text
                    ┌──────────────┐   ┌──────────────┐
  frontends         │  postio-app  │   │ postio-mcp   │   (future: -cli, -macos)
                    │    (GTK)     │   │   (headless) │
                    └──────┬───────┘   └──────┬───────┘
                           │                  │
                    ┌──────┴───────┐          │
                    │  postio-gtk  │          │   widgets, CSS, GTK key events
                    └──────┬───────┘          │
  ─────────────────────────┼──────────────────┼───────────────────────────────
                    ┌──────┴──────────────────┴───┐
  session           │      postio-session         │   store + dispatcher +
                    │  (verbs, undo, engine boot) │   engine, no toolkit
                    └──────┬──────────────────────┘
                    ┌──────┴───────┐
  runtime           │postio-runtime│   the database half: queue drainer,
                    └──────┬───────┘   body backfill, reconnect
                    ┌──────┴───────┐
                    │ postio-sync  │──── postio-imap ── postio-smtp
                    └──────┬───────┘     (MailBackend seam)
                    ┌──────┴───────┐
                    │postio-storage│──── postio-index (FTS5)
                    └──────────────┘
  ─────────────────────────────────────────────────────────────────────────────
  contract          postio-core     commands, events, registry, undo, bridge
                    postio-ui       keymap, selection, sanitize, quote, tokens
                    postio-config   TOML schema, validation, live reload
  domain            postio-model    types + JWZ threading
                    postio-search   query parser, highlighter, facets
```

---

## 9. Suggested sequencing

Ordered by *what gets more expensive if deferred*, not by size.

1. **Redraw the docs (§1).** Free, and the current tree actively misleads.
2. **Widen the command vocabulary (§4) and add correlation ids (§5).** Do these
   first among the code changes. Both touch the `Command`/`Event` contract, and
   that contract gets more expensive to change with every handler and every
   frontend added. Doing them before MCP/AI work means the extension path is
   the path, rather than a parallel one that bypasses the registry.
3. **Split `postio-session` out of `postio-app` (§3).** Unblocks a headless
   frontend, which is what MCP actually needs to exist. Mostly file moves.
4. **Extract `postio-ui` (§2).** The second-platform prerequisite. Can wait
   until a second platform is real, *except* for the sanitize/quote modules, which
   the composer (§6) will want sooner.
5. **Design the composer document model (§6) before building the rich editor.**
6. **Broaden the boundary rules (§7)** alongside 3 and 4, so each new crate
   arrives with its invariant already enforced.

Nothing here is urgent for the MVP, and none of it should displace the current
`mvp`-labelled work. Items 2 and 5 are the two that get materially more
expensive the longer they wait.
