# Contract: `postio-focus`

This is the public surface both frontends drive. Names are indicative. The
contract is the shape, the invariants and the test obligations, not the
spelling.

## API

```rust
pub struct FocusController { /* Send; plain data */ }

impl FocusController {
    pub fn new(policy: Policy, keymap: &Keymap, config: FocusConfig) -> Self;

    /// Synchronous so the host can swallow or pass the key within one frame.
    pub fn press(&mut self, chord: &Chord, facts: KeyFacts, now: Instant, rows: &dyn Rows) -> Press;

    pub fn handle(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect>;

    /// A neutral snapshot for observe(), storyboards and tests (ADR 0044).
    pub fn view(&self) -> FocusView;
}

pub struct Press { pub handled: bool, pub pending: Option<String>, pub effects: Vec<Effect> }

pub enum Effect {
    Show(Intent),
    Ask(Ticket, Request),
    Timer { token: u64, after: Duration },
}

/// The one mapping from a request to the client. Runs on any executor.
pub async fn perform(client: &Client, request: Request) -> Reply;

pub trait Rows {
    fn len(&self) -> u32;
    fn row(&self, position: u32) -> Option<FocusRowRef<'_>>;
    fn position_of(&self, id: MessageId) -> Option<u32>;
}
```

## Inputs

- `Command(CommandId, Origin)`, where `Origin` is Key, Bar, RowMenu,
  RowAction, Chrome, Bulk, Banner or Empty.
- `Event(postio_core::Event)`
- `Reply(Ticket, Reply)`
- `TimerFired(u64)`
- `WindowSize { w, h }`
- `CursorPlaced { position, by }`
- `RowPick { id, Toggle | Range }`
- `ScrolledToTop(bool)`
- `SurfaceClosed(SurfaceKind)`
- `ReaderState { more_open, finding }`
- `Config(FocusConfig)`
- `Keymap(Keymap)`
- `SavedSearches(..)`
- `Accounts(..)`
- `Typed { surface, text }`: bar and picker text.

## Intents

Grouped as in research R1. Each intent is drawable without asking the
controller anything else.

| Group | Intents |
|---|---|
| List | `Cursor{position, id, reveal}`, `Selection{ids, summary}`, `DeliverPage{generation, offset, rows, total}`, `RefreshList`, `SingleHeading` |
| Chrome | `Place{name}`, `Strip{…}`, `EmptyOrList`, `Banner`, `SyncLabel` |
| Feedback | `Toast{kind, text, undoable, seconds}`, `Notify`, `KeyboardHome(List \| CursorRow)` |
| Message | `OpenMessage{message, row, position, origin, host}`, `CloseMessage`, `Reader(verb)` |
| Digest | `OpenDigest`, `Digest(verb)`, `CloseDigest`, `RuleDialog`, `ShowRules`, `Rules(verb)` |
| Filtered | `ShowFiltered`, `LeaveFiltered`, `Filtered(verb)` |
| Capture | `OpenCapture{source, mode}`, `Capture(verb)` |
| Bar | `OpenBar(mode)`, `BarText`, `Bar(verb)`, `CloseBar`, `OpenPlaces`, `ClosePlaces` |
| Pickers | `OpenPicker{kind, anchor, aim, now}`, `OpenRowMenu` |
| Other | `OpenKeyMap`, `CloseTop`, `Composer(..)`, `OpenDraft`, `Settings(..)`, `CredentialDialog`, `OfferAddAccount`, `Confirm(..)`, `Placement`, `Quit` |

## Requests

- `Send{command, aims}`
- `Post(command)`
- `OpenScope{scope}`
- `Page{scope, offset, limit}`
- `FocusCounts`, `Places`, `Mailboxes`, `Accounts`, `DraftBehind`,
  `SweepPreview`, `Unsubscribe`
- Filtered, digest and vault reads
- `UndoTop`

`Aim::Everything{accounts, except}` is resolved inside `perform`. This is
the one copy; GTK and the FFI each have their own today.

## Invariants (each a unit test in `postio-focus/tests/`)

1. The cursor and the selection are distinct. `x` never moves the cursor, and
   `a` acts on the selection when there is one, otherwise on the cursor's
   row.
2. `!` clears the selection and keeps the cursor on the same message when it
   is still shown, otherwise on the first row.
3. Every list opens with the cursor on its first row (C30).
4. After a removal, the cursor goes to the survivor below. After an undo, it
   goes to the restored row.
5. A key the top surface does not own falls through to the list's table
   (fixes #1754).
6. With `stacking = false`, at most one of Message, Digest, Composer and
   Capture is on the stack (M4).
7. Back closes the top surface, then follows the list ladder.
8. A stale reply never changes state.
9. `press` with `in_text_entry` and a printable single key returns
   `handled: false` and no effects.
10. Every `Intent` the controller can emit has a matching arm in both
    drivers. A test enumerates intents against a list each driver exports
    (the registry-parity idea, applied to intents).

## Boundary

The `RULES["postio-focus"]` entry in `check-crate-boundaries.py` sets:

- **Banned:** GTK, GDK, libadwaita and webkit (with their `-sys` crates);
  `turso*`; `rusqlite`; `libsqlite3-sys`; `io-imap`; `postio-host`;
  `-session`; `-storage`; `-runtime`; `-widgets`; `-gtk`; `uniffi`.
- **Not allowed as direct dependencies:** `tokio`, `glib`, `async-std`.
