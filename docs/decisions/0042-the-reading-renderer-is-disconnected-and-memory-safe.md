# ADR 0042 — The reading renderer is disconnected and memory-safe

- **Status:** Accepted (2026-09-27). Built, with `specs/006-email-rendering` (archived).
- **Decision:** `postio-render` draws hostile mail in Postio's own process, so it contains nothing that runs C, reaches the network or asks the system for fonts. Every byte it draws is handed to it. A remote one arrives only through the application's fetcher, on the user's consent. A caught render panic stays catchable.

## Rules
- No C and no native bindings. Nothing in the renderer's product graph links native code, is a `-sys` crate, or compiles C or C++. The graph is what its code can call: normal and build edges, minus the edge into `postio-workspace-hack` (ADR 0047), with features as built including any the hack switches on. Decoders are the pure-Rust `png`, `jpeg`, `gif` and `webp` paths, and `image` stays out of the workspace-hack.
- No network. `blitz-dom`'s `net` feature stays off, and so does any HTTP, TLS or socket crate. The renderer resolves URLs only from the resource table it is given (message parts, Postio's faces, data URIs).
- No system font stack. Fonts come from Postio's `FontSet` (`fontdb`). `fontique` and `parley` `system` features and `blitz-dom` `system-fonts` stay off.
- Remote bytes arrive through `postio-runtime`'s `RemoteImageFetcher`, for a message the user opened whose sender they allowed or that they chose to show once. The renderer never learns a URL it has no bytes for.
- `panic = "unwind"`: the render thread catches a panic and shows the plain-text fallback. No profile that links the renderer may abort on panic.
- Enforcement: `check-renderer-is-memory-safe.py` (rules 1-3 and 5; prunes the workspace-hack but keeps its features); `check-crate-boundaries.py` `RULES["postio-render"]`; `postio-render`'s `egress` suite and `focus_suite::remote_images` for the consent rule (nothing before Show, Show fetches once, Always holds for that sender alone).

## Why
- The reader's process-per-view, start-up cost and dark-mode behaviour were what spec 006 fixed (`docs/notes/2026-09-26-blitz-or-webkit.md`).
- Rejected: WebKit for the reader; a sandboxed render process (memory safety plus no network removes what a sandbox is for); a setting that turns the network off (a crate not in the graph cannot be turned back on).

## Revisit when
- A needed format has no memory-safe decoder, or remote content must render without consent. Either is a new decision.
