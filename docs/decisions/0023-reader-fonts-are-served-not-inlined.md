# ADR 0023 — The reader's fonts are served over a scheme, not inlined into every document

- **Status:** Accepted (2026-09-01)
- **Date:** 2026-09-01
- **Decision by:** `/ux-architect`, on [#768](https://github.com/dlapiduz/postio/issues/768), which #749 split out because its fix changes a cross-frontend contract rather than a detail.
- **Issue:** [#768](https://github.com/dlapiduz/postio/issues/768)
- **Amends:** ADR 0019 Q6, whose list of what moved into `postio-ui` names "the `@font-face` data URIs". It is now the faces themselves and the handler that serves them; the data URIs are gone.
- **Related:** ADR 0019 (macOS frontend), ADR 0004 (composer document model), [#608](https://github.com/dlapiduz/postio/issues/608) (`BlobSource` moved into `postio-ui` for the same reason), [#749](https://github.com/dlapiduz/postio/issues/749) (stopped the pane reloading an unchanged document)
- **Decision:** **the vendored faces are served to the reader's web view over a `postio-font:` custom scheme, exactly as inline parts are served over `postio-cid:`.** The document keeps `@font-face` rules; it stops carrying the bytes. `font-src` narrows from `data:` to `postio-font:`. The font bytes get one owner — `postio-ui` — and the GTK build stops keeping a second copy.

---

## The problem

Every document the reader composes carries **~1.21 MB of base64 `@font-face` data** — eight TTFs, 909,640 bytes raw. `reader_css()` prepends `embedded_font_faces()` and `wrap_document` inlines the result in a `<style>` tag, so every render allocates and copies ~1.25 MB and hands it to the web engine to re-parse and base64-decode. Including an empty pane. Including every absent-state plate, which uses two of the eight faces.

The interaction budget is 16 ms. A megabyte of base64 per message change is not inside it, and #749 — which stopped the pane reloading a document that had not changed — removed the repeats without touching the cost of one genuine change.

## Why the obvious fix is wrong

`webkit6::UserContentManager::add_style_sheet` injects a stylesheet once per view instead of once per document. It works, the shape is already established in `postio-gtk` (`editor.rs:72-97` builds a `UserContentManager`), and it is what any GTK-only reader would do.

**It is GTK-only.** `WKWebView` has no user-stylesheet API; `WKUserScript` needs JavaScript, which the reader has off by policy. So the fix reaches one frontend and leaves the other re-feeding the megabyte forever.

That would not merely be an unfinished optimisation. `Session::reader_document()` hands this exact string across the FFI, and its own doc comment says the frontend's "entire job is to build a hardened web view, hand it this string, and refuse navigations". ADR 0019 Q6 answers *"how do the privacy invariants survive two frontends?"* with **"by being one implementation, not two that agree"**, and `crates/postio-ffi/tests/reader.rs:88` asserts the two documents byte-for-byte so that the answer stays true. A GTK-only stylesheet path makes the documents differ, and the test that exists to catch drift becomes the test that has to be relaxed to permit it.

## The decision

**A `postio-font:` scheme handler, sibling to `postio-cid:`.**

The document keeps its `@font-face` rules and references each face by URL:

```css
@font-face{font-family:'Barlow';font-weight:400;font-style:normal;font-display:block;
           src:url(postio-font:Barlow-Regular.ttf) format('truetype');}
```

The handler resolves a name against a **fixed compile-time table** of the eight vendored faces and returns not-found for anything else. It never takes a path, never touches the filesystem, and never reaches the network — the same contract `scheme.rs`'s `respond` already works under, for the same reason.

This is not a new pattern. It is [#608](https://github.com/dlapiduz/postio/issues/608)'s decision applied a second time: `BlobSource` moved into `postio_ui::reader::parts` because *"what a `Content-ID` may resolve to is a security property both frontends have to share, not a GTK detail"*. What a font URL may resolve to is the same kind of property, and it gets the same kind of home.

### What it buys

- **The document becomes proportional to the message.** The ~1.25 MB per-render allocation, the copy and the engine-side base64 decode all disappear. `reader_css()` should become a `&'static str` at the same time — nothing in it varies per render, since remote-image policy lives in the CSP and not the CSS.
- **The payload becomes proportional to what is actually drawn.** An engine fetches a `@font-face` src only when a rule matches text on the page. An absent-state plate pulls two faces instead of eight; a plain-text body pulls one. Under any inlining scheme it pulls all eight, always.
- **It works on both platforms with machinery both already have.** `WKURLSchemeHandler` is the direct counterpart of `webkit6`'s scheme registration, and the reader already depends on it for `postio-cid:`. `postio-font:` inherits whatever `postio-cid:` has proven on each platform: if inline images resolve there, fonts resolve there. It adds no new platform risk.
- **The CSP gets narrower.** `font-src data:` becomes `font-src postio-font:`. This is not a live hole today — the sanitizer removes `<style>` tag-and-contents and strips the `style` attribute, so a sender's CSS cannot declare a face — but the CSP exists precisely so that *"a sanitizer bug degrades to broken markup, not a live request"*, and font parsers are a classic memory-safety surface. `data:` lets any CSS that reaches the document ship arbitrary bytes into the web process's font parser; `postio-font:` lets it ship one of eight files the project vendored. Strictly narrower, for free.
- **One owner for the bytes.** Today the same TTFs are compiled in **twice** — the GResource bundle for Pango (`fonts.rs:53-76`) and `include_bytes!` in `postio-ui` for the base64 block — and `postio-ui`, the frontend-neutral crate, reads its data by relative path out of `postio-gtk/data/`, which is backwards. ADR 0019 moved the document into `postio-ui` to be frontend-neutral; the data it is made of did not follow. **The faces and the reader stylesheets move to `postio-ui`'s own data directory**, `postio-gtk` builds its GResource from there, and the second copy goes away — around 909 KB out of the binary.

### The details that would otherwise be guessed

- **`font-display: block`.** A `data:` face is present when the document parses; a scheme fetch is asynchronous, so without this there is a window where body text paints in fallback and then reflows — a flash of wrong font in the one pane whose whole job is to render text stably. `block` renders nothing for a short period and then swaps, which for bytes that are compiled into the process and served from memory is imperceptible. `swap` accepts the flash; `optional` would let a hiccup leave the reader in system sans permanently, which is the failure this ADR exists to avoid.
- **Register once per web context.** `WebKitWebContext` offers no way to unregister a scheme, and the font table is static for the process' life, so there is no per-message handle to manage — this is simpler than the `postio-cid:` case, not harder.
- **Names, not paths.** The URL carries a face's file name from the fixed table. Anything else is `NotFound`, like a `cid:` with no matching part.

### What must be measured, not assumed

Whether the engine caches a custom-scheme subresource across document loads is the one load-bearing unknown, and it decides how *large* the win is, not whether there is one. If it caches, a face is fetched once per web process. If it does not, each load fetches raw TTF bytes from an in-process memory stream — still less work than decoding a larger base64 string, and the document-side cost is gone either way. Measure it; do not assert it.

If the measurement comes back badly enough to matter, the fallback is the rejected option 2 below, and it should be taken deliberately with the number written down.

> **Measured 2026-09-02 (WebKitGTK 6, `crates/postio-gtk/tests/gtk_reader_fonts.rs`):**
> **it caches, and it is lazier than this ADR assumed.** Rendering a
> one-paragraph HTML message fetched **one** face — `Barlow-Regular.ttf`,
> the only one the page had text to draw with — not eight. Rendering a
> second, different message into the same view fetched **zero**: the engine
> served the face it already had.
>
> So the per-message font cost after the first render is nothing at all,
> against ~1.21 MB of base64 re-fed on every render before. "Proportional to
> what is actually drawn" turns out to understate it: it is proportional to
> what is drawn *and* not already fetched. The fallback is not needed. The
> test prints these counts on every run (`-- --nocapture`), so an engine
> that stops caching shows up as a number rather than as a mystery.

## What was rejected

**A GTK-only `UserStyleSheet`, keeping the FFI document as it is.** Cheapest to write and fixes the reported symptom on the reported platform. Rejected because it fixes one frontend and freezes the bug into the other, and because it trades away the single structural property ADR 0019 Q6 rests on. "Two readers that agree" is the failure mode Q6 was written to prevent; `reader.rs:88` is the machine that prevents it, and this option's first step is disabling that machine.

**`wrap_document(content, remote, Fonts::{Embedded, Injected})` — fonts as a parameter.** Better than the above: one function, and the divergence is explicit and greppable rather than implied. Still rejected as the primary answer. It makes the two frontends produce different documents by design, hands macOS the worse branch with no path off it, and creates a parameter a future frontend can be wrong about — a second place where a privacy-relevant document can differ, which is the shape ADR 0019 spent its Q6 removing. It is kept on file as the fallback if the scheme measures out badly, because it is honest about what it is.

**Shipping WOFF2 instead of TTF.** Would cut the vendored bytes to roughly a third and shrink the binary further. Rejected because Pango reads TTF and not WOFF2, so the project would carry both formats — reintroducing exactly the duplication this ADR removes, to save disk in a product whose stated budget is time. One copy of the bytes, serving both the toolkit and the web view, is worth more than the compression.

**Dropping the embedded faces and letting the reader use system fonts.** Rejected on the product: PRODUCT.md §19 names Barlow / Barlow Condensed / IBM Plex Mono as the identity, the design canvas sets message body text in Barlow, and the absent-state plates are Postio's own words in Postio's type. A web process never sees the fonts the host registered with Pango, so "no embedding" means "system sans", which is the outcome every option here exists to prevent.

## Consequences

- `crates/postio-ffi/tests/reader.rs:88` keeps asserting byte-identical documents, and keeps meaning something. Nothing about it is relaxed.
- The byte-exact CSP test moves to `font-src postio-font:` in both the blocked and allowed forms. It stays byte-exact: ADR 0019 Q6's proof is that there is exactly one string, not that the string never changes.
- `document.rs`'s *"every vendored face is embedded"* test becomes *"every vendored face is referenced and resolvable"* — the same guarantee against a face silently going missing, asserted against the table the handler serves rather than against a count of `data:` URIs.
- The macOS frontend gains one obligation it did not have: register a second scheme handler. It is a handful of lines beside the one it already has, and it is the reason this ADR is a decision rather than a patch.
- Nothing about the network changes. `postio-font:` is answered in-process from compiled-in bytes; `connect-src 'none'` and `default-src 'none'` are untouched, and the loopback test ADR 0019 Q6 specifies still asserts zero connections.
