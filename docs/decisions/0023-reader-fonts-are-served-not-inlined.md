# ADR 0023 — The reader's fonts are served over a scheme, not inlined into every document

- **Status:** Accepted (2026-09-01). Built
- **Date:** 2026-09-01
- **Decision by:** `/ux-architect`, on [#768](https://github.com/dlapiduz/postio/issues/768), which #749 split out because its fix changes a cross-frontend contract rather than a detail.
- **Issue:** [#768](https://github.com/dlapiduz/postio/issues/768)
- **Related:** ADR 0019 (macOS frontend; its Q6 lists the faces and the handler that serves them among what lives in `postio-ui`), [ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md) (the renderer that draws the document on Linux), ADR 0004 (composer document model), [#608](https://github.com/dlapiduz/postio/issues/608) (`BlobSource` moved into `postio-ui` for the same reason)
- **Decision:** **the composed document names Postio's vendored faces by `postio-font:` URLs and never carries their bytes, exactly as inline parts are named by `postio-cid:`.** The document keeps `@font-face` rules; `font-src` is `postio-font:`. The font bytes have one owner — `postio-ui` — and whatever draws the document resolves the names from that one table.

---

## The problem

A document that inlines its faces carries **~1.21 MB of base64 `@font-face` data** — eight TTFs, 909,640 bytes raw — on every render, including an empty pane, and hands it to the engine to re-parse and decode. The interaction budget is 16 ms; a megabyte of base64 per message change is not inside it.

## Why the document, not the view, carries the fix

A per-view stylesheet (WebKitGTK's `UserContentManager::add_style_sheet`) is GTK-only: `WKWebView` has no user-stylesheet API, and `WKUserScript` needs JavaScript, which the reader has off by policy. `Session::reader_document()` hands the same composed string across the FFI, and ADR 0019 Q6 answers *"how do the privacy invariants survive two frontends?"* with **"by being one implementation, not two that agree"** — `crates/postio-ffi/tests/ffi_suite/reader.rs` asserts the two documents byte-for-byte. A fix that made the frontends' documents differ would have to relax that test first.

## The decision

The document references each face by URL:

```css
@font-face{font-family:'Barlow';font-weight:400;font-style:normal;font-display:block;
           src:url(postio-font:Barlow-Regular.ttf) format('truetype');}
```

A name resolves against a **fixed compile-time table** of the eight vendored faces (`postio_ui::reader::document::font_bytes`) and returns not-found for anything else. It never takes a path, never touches the filesystem, and never reaches the network. What a font URL may resolve to is a security property both frontends share, so it lives where #608 put `BlobSource`: in `postio-ui`.

Who answers the names:

- **On Linux, `postio-render`** (ADR 0042). Postio's faces are its own `FontSet`, registered once per process and handed to every render; `Resources` answers `postio-font:<name>` in-process, and nothing is fetched.
- **On macOS, a `WKURLSchemeHandler`** for `postio-font:`, beside the one for `postio-cid:`.
- **The composer's editor**, still a WebKitGTK view (ADR 0039), registers the same scheme once per web context (`postio-widgets`' `composer/scheme.rs`). `WebKitWebContext` offers no way to unregister a scheme, and the table is static, so there is no per-message handle.

What it buys:

- **The document is proportional to the message.** No per-render allocation, copy or decode of the faces.
- **The CSP is narrower.** `font-src postio-font:` lets CSS that reaches the document name one of eight files the project vendored, where `data:` would let it ship arbitrary bytes into a font parser — a classic memory-safety surface. The sanitizer already strips a sender's `<style>` and `style`; the CSP exists so that *"a sanitizer bug degrades to broken markup, not a live request"*.
- **One owner for the bytes.** The faces and the reader stylesheets are in `postio-ui/data/`, and the GTK build reads them from there.

**`font-display: block`.** In a web view a scheme fetch is asynchronous; without `block` there is a window where body text paints in fallback and reflows. `swap` accepts the flash; `optional` would let a hiccup leave the reader in system sans.

## What was rejected

- **A GTK-only user stylesheet**, keeping the FFI document as it is: it fixes one frontend and disables the test that keeps the two documents one.
- **Fonts as a `wrap_document` parameter** (`Embedded` or `Injected`): the two frontends produce different documents by design, and a future frontend can choose the wrong branch.
- **WOFF2 instead of TTF:** the toolkit reads TTF, so the project would carry both formats.
- **System fonts in the reader:** PRODUCT.md §19 names Barlow, Barlow Condensed and IBM Plex Mono as the identity, and the design sets message bodies in Barlow.

## Consequences

- `crates/postio-ffi/tests/ffi_suite/reader.rs` asserts byte-identical documents, and nothing about it is relaxed.
- The CSP test is byte-exact on `font-src postio-font:` in both the blocked and allowed forms.
- `document.rs` asserts every vendored face is referenced and resolvable, against the table a handler serves.
- `crates/postio-gtk/tests/gtk_suite/gtk_reader_fonts.rs` holds that the faces are the reader's own and not carried by the document.
- Nothing about the network changes. `postio-font:` is answered in-process from compiled-in bytes; `connect-src 'none'` and `default-src 'none'` are untouched.
