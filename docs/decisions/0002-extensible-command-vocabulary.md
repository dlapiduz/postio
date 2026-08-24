# ADR 0002 — An extensible command vocabulary

- **Status:** Accepted and implemented — `3e8102f` (core), `20a1424` (gtk)
- **Date:** 2026-08-24, implemented 2026-08-24
- **Bead:** `postio-plp4`, which blocks `postio-z3b.2` (MCP) and `postio-sgi` (AI)
- **Decision:** keep `CommandId` closed and exactly as it is. Open the layer
  *above* it, which is already string-keyed.

---

## Context and method

`postio-plp4` was filed because no command can exist without recompiling
`postio-core`, and that is the wall MCP, AI and any plugin surface hit. The
bead sketched `CommandId::Ext(ExtId)` — a new variant on the existing enum.

This ADR exists because the sketch was written from reading the code and never
tested against it. `docs/architecture-review-2026-08.md` §4 asserted a large
blast radius on the strength of a raw grep — **368 `CommandId::` occurrences
across 40 files in four crates** — and the handoff prompt turned that into
"measure before you design". This is that measurement.

Method: count the *kinds* of reference rather than the references, and read
every place the vocabulary is actually closed. Everything below is from the
tree at `95eb685`.

**The headline: the sketch is wrong, and the raw count was misleading in both
directions.** Adding a variant to `CommandId` is cheaper than 368 suggests and
more expensive than it looks, for a reason the grep could not show. And the
seam the feature actually needs already exists one layer up.

---

## Q1 — Where is the vocabulary actually closed?

Of the 368 references, **almost all are constructions** — `CommandId::Archive`
used as a value in a registry row, a wiring table or a test assertion. Those do
not break when an enum gains a variant.

Matches with a `CommandId` as the scrutinee, in the entire tree:

| Site | What it is |
|---|---|
| `command.rs:49` | `as_str()`, generated inside the `command_ids!` macro |
| `command.rs:453` | `Command::default_for(id)` — 38 arms |

That is all. `FromStr` (`command.rs:165`) is a linear scan over `CommandId::ALL`,
not a match. `Command::id()` (`command.rs:405`) matches over `Command`, a
different enum. Outside `postio-core` there is exactly **one** match involving a
`CommandId`, at `window.rs:950`, and it is a match on a `Result`, not on the
enum.

**So "adding a variant breaks 368 sites" is false.** It breaks two.

## Q2 — What does adding a variant to `CommandId` actually cost?

Two things the grep could not see, and the second is decisive.

**`CommandId` is `Copy`** (`command.rs:37`) and is passed by value throughout.
An `Ext(String)` variant would make it non-`Copy`, and *that* is what would
touch a large fraction of the 368 sites. Avoidable — intern to a `&'static str`
or a `u32` handle and `Copy` survives.

**`registry::get` is an array index, not a lookup:**

```rust
pub fn get(id: CommandId) -> &'static CommandSpec {
    let spec = &SPECS[id as usize];
    debug_assert_eq!(spec.id, id, "the registry table is out of order");
```

`id as usize` is an enum-to-integer cast, and **Rust permits that only for a
fieldless enum.** A single data-carrying variant makes the cast stop compiling,
and with it the O(1) property that `get` was written for — every one of the 38
rows would move to a scan or a map.

Verified rather than assumed, because the decision turns on it: `rustc` rejects
the cast with `E0605`, *"an `as` expression can be used to convert enum types to
numeric types only if the enum type is unit-only or field-less"*. Note this
fires even when the payload is `Copy` — an `Ext(&'static str)` variant still
derives `Copy` cleanly and still breaks the cast. So interning solves the `Copy`
problem and does **not** solve this one. They are separate costs.

This is the real finding. It is not that the change is large; it is that the
change buys nothing and costs the registry's shape. Which raises the question
the sketch never asked:

## Q3 — Does the extension seam need to be in `CommandId` at all?

No. **The binding layer is already string-keyed, and already has the fallback.**

`keymap::Outcome::Command` carries a `String`, not a `CommandId`. It is parsed
at the window boundary, and the failure arm is already written:

```rust
Outcome::Command(id) => match id.parse::<CommandId>() {
    Ok(id) => { self.run(id); glib::Propagation::Stop }
    // A binding for a command this build does not know: leave the
    // key alone rather than swallowing it.
    Err(_) => glib::Propagation::Proceed,
},
```

Everything from `[keys]` through the resolver to that line already deals in
strings and already tolerates an id it does not know. The keymap does not need
opening; it is open. Only the parse is closed, and only at one site.

The same is true of `[keys]` itself, which refers to commands by string because
command ids are a file format (`ARCHITECTURE.md` §3). An extension command
needs *no new configuration syntax at all*.

## Q4 — Then where is the cost?

In the registry's return types. All four accessors hand out `&'static`:

```rust
pub fn all()            -> impl Iterator<Item = &'static CommandSpec>
pub fn for_context(..)  -> impl Iterator<Item = &'static CommandSpec>
pub fn get(..)          -> &'static CommandSpec
pub fn lookup_binding(..) -> Option<&'static CommandSpec>
```

**48 call sites** consume them — 22 in `src` across `postio-core` and
`postio-gtk`, 26 in tests. A specification registered at runtime cannot be
`&'static`, so naively making `all()` return static ∪ dynamic changes that
lifetime at all 48.

Most of that is avoidable. Many call sites *mean* "the built-in table" —
`keybindings_doc.rs` renders shipped documentation, `command_registry.rs`
asserts invariants over what shipped. They should keep seeing a static table
and keep compiling untouched.

But the sites that must change are **not** only view surfaces, and an earlier
draft of this ADR was wrong to say so. The centre of the work is a type:

```rust
pub struct Keymap {
    bindings: BTreeMap<CommandId, Vec<String>>,   // config.rs:51
    problems: Vec<String>,
}
```

`postio_core::Keymap` — the resolved `[keys]` table — **is keyed on
`CommandId`**. So is `bindings()`, `command_for()` (which returns
`Option<CommandId>`) and `holder_of()`. "Extension commands are bindable from
`[keys]` with no new syntax" is therefore not free: it means `Keymap` becomes
keyed on the wider id. That is the real cost of this feature, it lives in
`postio-core`, and no amount of care at the view layer avoids it.

The honest inventory of `src` sites that must move:

| Where | What | Why |
|---|---|---|
| `core/config.rs:51` | `Keymap.bindings` map key | the binding table must hold extension ids |
| `core/config.rs:64,77,99,120,379` | `resolve()` | builds that table |
| `core/config.rs:143,150,159,175,179` | `bindings`, `command_for`, `holder_of` | accessors + conflict detection |
| `core/dispatch.rs:290,310` | `registry::get(id).title` | error text for an extension command |
| `core/command.rs:515` | `is_destructive()` | merged lookup |
| `gtk/keymap.rs:637` | `from_commands` | resolver built from specs + bindings |
| `gtk/window.rs:950` | `id.parse::<CommandId>()` | the one parse that is actually closed |
| `gtk/palette.rs:178` | `Ctrl+K` rows | must list extensions |
| `gtk/cheatsheet.rs:99` | `?` rows | must list extensions |
| `gtk/list_view.rs:984,1011` | context menu + dispatch-by-name | menu is filtered by `is_message_action(CommandId)` |

Roughly a dozen `src` sites across two crates, plus their tests — not three,
and not 48.

---

## Decision

**1. `CommandId` does not change.** It stays closed, fieldless, `Copy`, and
index-addressable. `registry::get` keeps its array index and its O(1).

**2. Add `ExtId` — a namespaced, interned, `Copy` identifier.**
`"mcp:summarise-thread"`, `"user:file-to-receipts"`. The namespace keeps
built-in ids collision-free forever and makes provenance visible in the palette
and in logs. Interned so it is `Copy`; the interner is append-only and bounded
by the number of registrations.

**3. Add `ActionId` above both:**

```rust
pub enum ActionId { Builtin(CommandId), Ext(ExtId) }
```

This is what the registry, the palette, the cheat sheet and the key hints deal
in. Dispatch keeps `HashMap<CommandId, Handler>` for built-ins and gains a
parallel map for extensions, so the built-in path does not slow down or change
shape.

**4. Leave `registry::all()` alone; add a second accessor.** `all()` and
`get()` keep meaning "the built-in table" and keep their `&'static`, so every
call site that documents or asserts over what *shipped* is untouched — which
is most of the 26 in tests. Add `registry::reachable(context)` yielding merged
built-in + registered specs, and move the consumers in the Q4 table onto it.

**4a. `Keymap` becomes keyed on `ActionId`.** This is the load-bearing part and
should be done first, because everything else in the gtk column follows from
it. `command_for()` returns `Option<ActionId>`. `Keymap::resolve` already
warns-and-continues on "a command this build does not know" (`config.rs:58`),
which is the behaviour an unregistered extension id needs anyway — so the
failure mode is already designed, and the ordering problem below is why it
matters.

**5. `CommandSpec::title` becomes `Cow<'static, str>`.** Required for owned
titles; also the thing that unblocks i18n, which `&'static str` makes
impossible today.

**6. Registration validates the invariant that a test currently guards.**
`destructive: true` with `recovery: Recovery::None` must be **rejected at
registration**, returning an error. Today "a destructive command is
recoverable" is a test over a static table (`command_registry.rs:190`); a table
that grows at runtime cannot be checked by a test over its literal, so the
check moves into the door. An AI- or plugin-invoked destructive action with no
undo is worse than a built-in one, because the user did not type it.

### Why this over the sketch

The sketch put the seam in the innermost, most-constrained type in the
contract. The measurement says the layer that needs to be open — binding
resolution — is already open and already string-keyed, and that the type that
would have to give up `Copy` and O(1) indexing gains nothing from the change.

Concretely: the sketch's version touches `CommandId`'s two matches, breaks the
`as usize` index, risks `Copy` across 368 sites, and still has to solve the
registry lifetime problem. This version touches one parse site, three registry
consumers, and adds two types.

### What this deliberately does not do

It does not make extension commands *equal* to built-ins in `postio-core`'s
type system, and that is on purpose. A built-in is exhaustively matched and
statically known to have a handler. An extension is neither. Erasing that
distinction inside the contract would trade away the property that a command
cannot be silently unhandled — which `ARCHITECTURE.md` §2 names as the reason
the registry is worth having. They are equal where it matters to the *user* —
the palette, the cheat sheet, `[keys]` — and distinguishable where it matters
to the *compiler*.

---

## Correlation ids — separable, do it second

The bead's other half. `send` is fire-and-forget and events are broadcast; a
programmatic caller cannot tell which events answer its own command, and the
sync engine emits `MessagesChanged` constantly for unrelated reasons.

This shares no code with the vocabulary work. It should land as its own bead
and its own commit, **after** the above is green.

Shape, unchanged from the bead: `send_tracked(cmd) -> InvocationId`,
`Invocation::id()`, `origin: Option<InvocationId>` on events a handler emits.
Purely additive — the GTK frontend ignores it and no existing call site changes.

One constraint worth stating now: `Event` is already `Clone` and broadcast to a
single consumer today. If a second consumer arrives (an MCP server alongside
the window), the fan-out story has to be decided then, not assumed. That is not
this bead.

> **Settled:** [ADR 0013](0013-event-fanout.md) (2026-08-24) decides the
> fan-out story — an event hub at the composition root, subscribed to by name.

### Implemented — issue #33

Built as sketched, with `Invocation::id()` becoming `invocation_id()` because
`id()` was already taken by the `CommandId` accessor. The shape landed in
`crates/postio-core/src/invocation.rs`:

- `CommandSender::send_tracked(cmd) -> InvocationId`, alongside an unchanged
  `send`.
- The *sink* carries the origin, not the command and not the handler. The
  pump tags the `EventSink` it hands to a handler, so a handler that never
  heard of correlation still emits attributable events, and a task it spawns
  keeps the attribution after that handler has returned. `CommandHandler` did
  not change; neither did any call site outside `postio-core`, which the
  "purely additive" claim required and a `cargo build --workspace` confirmed.
- The channel carries an `EventEnvelope { event, origin }`.
  `EventStream::next`/`try_next`/`next_blocking` still yield a bare `Event`
  and discard the envelope; the `*_tracked` accessors hand it over.

One thing the sketch did not have, added because the acceptance criterion
needs it. A caller told to "observe the outcome of THAT invocation" must have
an outcome to observe, and a handler that succeeds silently emits nothing at
all — so correlation alone leaves a caller unable to tell success from *still
running*. `Event::InvocationFinished { invocation, outcome }` is therefore
emitted **once per tracked send, and only for a tracked send**: completed,
rejected, or failed, including when the handler panicked and when no handler
was registered. A programmatic caller awaiting an answer that never arrives
is a hang, which is a worse failure than the one being reported. The
untracked path emits nothing new, so the GTK frontend's stream is unchanged
event for event.

The fan-out constraint above is untouched and still the next decision: there
is still exactly one `EventStream`, so a tracked caller and the window cannot
both read it. That holds while the only tracked callers are tests and a
future headless consumer, and it is the first thing to settle when an MCP
server wants to sit beside a running window.

> **Settled:** that decision is now [ADR 0013](0013-event-fanout.md) — every
> consumer subscribes to one hub and receives its own `EventStream`.

---

## Consequences

- `postio-core` gains no dependencies. The interner is `std`.
  `ARCHITECTURE.md` §9 stands: no optional dependencies, ever.
- `postio-gtk` changes in five files — `keymap.rs`, `window.rs`,
  `palette.rs`, `cheatsheet.rs`, `list_view.rs` — mostly replacing
  `registry::all()` with `registry::reachable(context)` and widening the id
  type. `postio-app` is expected to need no change at all: it registers
  handlers through `DispatcherBuilder` keyed on built-in `CommandId`s, and
  extensions register through their own door.
- **An ordering problem this ADR does not solve:** `config.toml` is parsed and
  `[keys]` resolved at startup, but extensions register later. A `[keys]` entry
  naming an extension command will not find it at resolve time. `Keymap`'s
  existing warn-and-continue path stops that being a crash, but "the binding
  silently does nothing" is not acceptable either. Decide this during
  implementation: either re-resolve the keymap when an extension registers, or
  keep unresolved bindings and bind them late. Prefer the former — the config
  watcher already re-resolves on file change (`ARCHITECTURE.md` § live reload),
  so the machinery exists.
- `[keys]` gains extension commands with **no syntax change** — bindings
  already name commands by string.
- The `docs/keybindings.md` generator keeps rendering the built-in table only,
  which is correct: it documents what ships.
- `window.rs:950`'s "command this build does not know" arm becomes "not a
  built-in — try the extension registry", and keeps its fallback for genuinely
  unknown ids.

## Follow-ups to file if this is accepted

- Update `postio-plp4`'s design notes: the `CommandId::Ext(ExtId)` sketch is
  superseded by `ActionId`. The acceptance criteria are unchanged and still
  correct.
- Split the correlation-id half into its own bead, blocked on the vocabulary.
- `ARCHITECTURE.md` §2 gains a paragraph on how an extension command reaches
  the palette, once this is real rather than proposed.

## What implementation changed

Built as decided, with the measurement holding: `postio-app` needed **no
change at all**, and `postio-gtk` compiled against the widened `postio-core`
untouched — the keymap accessors take `impl Into<ActionId>`, so the many call
sites passing a built-in read exactly as before.

Three things this ADR got wrong or left open, corrected here rather than
silently:

**Titles are leaked, not `Cow`.** Decision 5 said `CommandSpec::title` becomes
`Cow<'static, str>`, "required for owned titles". It is not required, and it
has a cost this ADR did not price: `CommandSpec` is `Copy` and `Cow` is not, so
that change would take `Copy` off the spec type to buy something the interner
already pays for. Registration leaks the title instead — the same argument this
ADR already accepts for `ExtId` ("append-only and bounded by the number of
registrations"). `CommandSpec` is untouched, and so are the 26 test call sites
over it. i18n is unaffected: a translated title is resolved at registration.

**The `[keys]` ordering problem is solved by parsing, not by re-resolving.**
This ADR preferred re-resolving the keymap on registration and asked for that
to be confirmed workable. It *is* workable — `ConfigService` retains the
overrides and re-resolves on `apply` — but it is the wrong mechanism, because
`register` is a free function on a global with no access to `ConfigService`.
Re-resolution would need either interior mutability behind `keymap()`, or an
explicit call the application must remember; a forgotten call is a silently
dead binding, which is the outcome the consequence section calls unacceptable.

Interning does not depend on registration, so `Keymap::resolve` binds a
namespaced id whether or not a command exists for it, and it starts reaching
one the moment it registers. The binding was never lost — it pointed at
something not yet there. Consequence to be deliberate about: an unregistered id
has unknown contexts, so conflict detection treats it as `ContextSet::ANY`,
protecting the user's explicit override from a built-in default, per the
existing "a default is a suggestion; an override is not" rule.

**The Q4 inventory missed `gtk/finder.rs`.** The palette's *widget* lives
there, not in `palette.rs`, and its command channel was `CommandId`-typed end
to end. Six gtk files, not five.

Two things added that this ADR did not specify, both because a discoverable
command that cannot run is the failure mode `ARCHITECTURE.md` §2 exists to
prevent:

- `Dispatcher::dispatch_ext` and `DispatcherBuilder::on_ext`, a parallel path
  keyed on `ExtId`. Deliberately not a `Command` variant: `ExtInvocation`
  carries an id and a sink and no payload, because nothing in this build knows
  what payload it would have.
- `Window::connect_ext_command`, the seam an application subscribes to in order
  to route a registered command to that dispatcher. `Event::CommandRejected`
  widened to `ActionId` so both halves are refused through one event;
  `postio-app` destructures it with `..` and did not notice.

## What would falsify this

If a real MCP or AI consumer turns out to need an extension command to be
dispatchable through `Command::default_for` — that is, to carry a *payload*
shaped like a built-in's rather than an opaque one — then `ActionId` is the
wrong split and the extension path needs its own `Command` variant too. Nothing
here has been built against a real consumer; that is the main weakness of this
ADR and the reason its status is Proposed.
