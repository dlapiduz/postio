# ADR 0023 — The reader's fonts are served over a scheme, not inlined into every document

- **Status:** Accepted (2026-09-01), built (#768)
- **Decision:** the composed reader document names Postio's vendored faces by `postio-font:` URLs and never carries their bytes, as inline parts are named by `postio-cid:`.

## Rules
- The document keeps `@font-face` rules with `src:url(postio-font:<Face>.ttf) format('truetype')` and `font-display: block`; the CSP has `font-src postio-font:`, never `data:`.
- A name resolves against a fixed compile-time table of the eight vendored faces (`postio_ui::reader::document::font_bytes`). Anything else is not-found. It never takes a path or touches the filesystem or network.
- Linux: `postio-render` registers a `FontSet` once per process and answers `postio-font:<name>` in-process ([ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md)). macOS: a `WKURLSchemeHandler` beside the `postio-cid:` one. The composer's editor registers the scheme once per web context (`composer/scheme.rs`), because a `WebKitWebContext` cannot unregister one.
- Faces and reader stylesheets live in `postio-ui/data/`, the one owner of the bytes.
- Tests: `crates/postio-ffi/tests/ffi_suite/reader.rs` asserts byte-identical documents across frontends; the CSP test is byte-exact on `font-src postio-font:` in blocked and allowed forms; `document.rs` asserts every vendored face is referenced and resolvable.
- Body text is set in the vendored faces (Barlow, Barlow Condensed, IBM Plex Mono; PRODUCT §19), never system fonts. TTF only.

## Why
- Inlining costs about 1.21 MB of base64 on every render, including an empty pane, re-parsed against a 16 ms budget.
- `data:` would let CSS ship arbitrary bytes into a font parser, a memory-safety surface; the scheme names one of eight files.
- A GTK-only user stylesheet fixes one frontend and splits the two documents, and `WKWebView` has no equivalent. A fonts parameter on `wrap_document` lets a frontend choose the wrong branch.
- `block` avoids a fallback flash that `swap` accepts and `optional` risks permanently.

## Revisit when
- A frontend cannot register a custom scheme.
