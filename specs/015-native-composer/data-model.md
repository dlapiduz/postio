# Data model: the algebra and the projection

All types live in `postio-body` (a pure leaf, no new dependencies). Names
are proposals; the shape is the contract.

## Position and selection

```rust
/// Where in a document. Characters, not bytes or UTF-16 (research R1).
pub struct DocPosition {
    /// Block index, then one index per list item or quote child entered.
    pub path: Vec<u32>,
    /// Characters from the start of that block's flattened inline text.
    pub offset: u32,
}

pub struct Selection { pub anchor: DocPosition, pub head: DocPosition }
```

A collapsed selection has `anchor == head`. A valid position names a block
that has inline text, and an offset no greater than its length. No valid
position is inside `Block::Quoted`, `Block::Rule` or an image's alt text.

## Intents

```rust
pub enum EditIntent {
    // text
    Insert(String),                 // a commit, typed text
    DeleteBackward(Unit),           // Unit: Char, Word, Line
    DeleteForward(Unit),
    SplitBlock,                     // Enter
    InsertBreak,                    // Shift+Enter, Inline::Break
    // marks, over the selection
    ToggleMark(Mark),               // Strong, Emphasis, Code (main); more in spec 014
    SetLink(Option<Href>),
    InsertImage { content_id: ContentId, alt: String },
    // blocks, at the caret's block
    ToggleBlock(BlockKind),         // Heading(level), BulletList, NumberedList, Quote
    InsertRule,
    Indent, Outdent,                // list items
    // composed
    Convert(Conversion),            // a Markdown trigger (FR-006)
    Paste(Document),                // already parsed (FR-007)
    // history and IME
    CommitPreedit(String),
    CancelPreedit,
}
```

Spec 014's arms (Phase 7) are added to this enum and are not in the table
above: `ToggleTask`, `SetListStart`, `SetCodeLanguage`, `InsertTable`,
`TableRow(Add|Delete)`, `TableColumn(Add|Delete|Align)`, and the new `Mark`
and `BlockKind` variants. The compiler's exhaustive `match` finds every
site.

`Mark` and `BlockKind` are closed sets that mirror `Document`'s inline and
block kinds that a person can author. `Conversion` is
`{ trigger_len: u32, kind: ConversionKind }`: postio-ui's matcher reads the
text before the caret (`block_text_before`), finds the sequence, and says how
many characters the trigger took and what it becomes. `apply` knows no
Markdown.

## The result

```rust
pub struct Edited {
    pub document: Document,
    pub selection: Selection,
    /// Top-level blocks [start, old_end) replaced by [start, new_end).
    pub changed: BlockRange,
    pub join: Join,               // Run | New (research R6)
}
```

`Edited::unchanged(document, selection)` has an empty range. The surface
patches `changed` and nothing else.

## The projection

```rust
pub fn project(document: &Document, preedit: Option<&Preedit>) -> Projection;

pub struct Projection {
    pub text: String,             // what the buffer holds
    pub runs: Vec<Run>,           // contiguous, covering text
}
pub enum Run {
    Text { doc: DocPosition, len: u32, marks: MarkSet },
    Prefix { kind: PrefixKind, len: u32 },   // bullet, number: no doc position
    Atomic { kind: AtomicKind, len: u32 },   // reply quote, image anchor
    Preedit { len: u32 },                    // uncommitted: no doc position
}
impl Projection {
    pub fn position_of(&self, buffer_offset: u32) -> DocPosition;
    pub fn offset_of(&self, p: &DocPosition) -> u32;
    pub fn utf16_of(&self, buffer_offset: u32) -> u32;
    pub fn patch(&self, changed: BlockRange, new: &Document) -> Patch; // minimal buffer edits
}
```

**Invariants** (each a property test):

1. `position_of(offset_of(p)) == p` for every valid `p`.
2. `offset_of(position_of(o))` is the nearest text offset for every `o`; a
   `Prefix` or `Atomic` offset lands at the following text or the whole
   unit.
3. `project(apply(d, s, i).document)` equals `project(d)` with
   `patch(changed)` applied, for every intent.
4. A `Preedit` run is never in `position_of`'s range, and setting it does not
   change `d`.

## What is not stored

Bullets, numbers, margins and the reply quote's drawing are projection
output. They are not in `Document`, so they cannot be lost, and they are
never sent.
