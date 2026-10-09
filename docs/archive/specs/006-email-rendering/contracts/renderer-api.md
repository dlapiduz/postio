# Contract: `postio-render`'s public surface

This is the contract between the renderer crate and anything that shows its
output. It is GTK-free, network-free and C-free (research R2 and R3). Names
are indicative; the invariants are the contract.

## Entry points

```rust
/// One per reader. Owns a render thread (64 MiB stack) and every Blitz
/// document on it. Dropping it detaches the thread.
pub struct Renderer { /* … */ }

impl Renderer {
    pub fn new(fonts: &FontSet) -> Renderer;

    /// Non-blocking. The result arrives on the returned receiver, or never,
    /// if superseded. The caller runs the deadline (R6), with a length it is
    /// given: `DEFAULT_RENDER_DEADLINE` in production, and an injected one in
    /// tests. On expiry it calls `abandon(generation)` and shows `fallback()`.
    pub fn request(&self, req: RenderRequest) -> Receiver<RenderedDocument>;

    /// Marks the thread tainted if that generation is still running; the next
    /// request goes to a fresh thread.
    pub fn abandon(&self, generation: u64);

    /// Plain-text fallback, rendered synchronously and cheaply by the
    /// renderer's own minimal path (no sender markup, reader palette).
    pub fn fallback(&self, text: &str, theme: &Theme, viewport: Viewport,
                    reason: FallbackReason) -> RenderedDocument;
}

/// The production render bound (spec FR-023). Callers take their deadline
/// as a parameter and default to this; tests inject a scaled or a tiny one.
pub const DEFAULT_RENDER_DEADLINE: Duration = Duration::from_millis(400);

/// Process-wide, built once off the UI thread: bundled faces + fontdb
/// discovery, generic families and per-script fallbacks set explicitly.
pub struct FontSet { /* … */ }

/// Rasterise one tile from a snapshot. Pure; callable from any thread.
pub fn rasterize_tile(doc: &RenderedDocument, tile: TileSpec, out: &mut [u8]);
```

## Types

`RenderRequest`, `RenderedDocument`, `TextIndex`, `MessageBox`,
`Presentation`, `LinkBox`, `FoldBox` and `RenderCounts` are defined in
`data-model.md`. `RenderedDocument: Send + Sync + 'static`, asserted with
`static_assertions`.

```rust
pub enum LinkTarget {
    External(Url),           // http, https, mailto only; anything else is not a link
    Verb(MessageVerb),       // postio-allow / -reply / -forward / -continue
    Fragment { scope: Scope, id: String },
}

pub enum FallbackReason { Panicked, Deadline, OverCap(Cap), Undecodable }
```

## Pure functions over a snapshot (UI thread; no engine)

```rust
impl TextIndex {
    pub fn hit(&self, point: Point) -> Option<usize>;             // char offset
    pub fn word_at(&self, offset: usize) -> Range<usize>;
    pub fn line_at(&self, offset: usize) -> Range<usize>;
    pub fn rects(&self, range: Range<usize>) -> Vec<Rect>;       // selection/find highlight
    pub fn slice(&self, range: Range<usize>) -> &str;            // copy
    pub fn find(&self, query: &str) -> Vec<Range<usize>>;        // folded match
    pub fn char_at_top(&self, y: f64) -> usize;                  // zoom anchor
}

impl RenderedDocument {
    pub fn link_at(&self, point: Point) -> Option<&LinkBox>;
    pub fn fold_at(&self, point: Point) -> Option<&FoldBox>;
    pub fn current_message(&self, viewport: Rect) -> Option<Scope>;   // greatest visible area (001 FR-035)
    pub fn message_top(&self, scope: Scope) -> Option<f64>;           // scroll-to-message
}
```

## Guarantees (each is a test)

1. **Disconnected.** No request, whatever its content, causes a socket to
   open. This is proven by the graph checks (`renderer-graph-checks.md`) and
   observed with a loopback listener across the hostile corpus.
2. **Closed resources.** Only `RenderRequest.resources` is consulted. Every
   other lookup increments `counts.resources_unresolved` and draws nothing,
   or a placeholder of the declared size.
3. **Per-message resolution.** A `cid:` from scope A never resolves to a part
   of scope B.
4. **Contained failure.** A panic anywhere in parse, style, layout, record or
   index yields `outcome = FellBack { Panicked }`, never an unwind into the
   caller. The panicked document is dropped.
5. **Bounded input.** Any `over_cap` from `postio-body` yields
   `FellBack { OverCap }` without the markup reaching Blitz.
6. **Contrast floor.** For every cluster,
   `contrast(color, painted_ground) ≥ 4.5`, or `≥ 7` in high contrast,
   whatever the text size.
7. **No script.** No request executes script. Blitz has no script engine, and
   the snapshot carries no handler. `javascript:` never appears as a
   `LinkTarget`.
8. **Deterministic.** The same request produces the same display list and
   text index. Fidelity and contrast tests rely on this.
9. **Bounded passes.** `counts.style_passes ≤ 2` and `counts.renders == 1`
   per request.
