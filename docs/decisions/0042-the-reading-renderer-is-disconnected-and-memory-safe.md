# ADR 0042 — The reading renderer is disconnected and memory-safe

- **Status:** Accepted (2026-09-27), with `docs/archive/specs/006-email-rendering`. Built
- **Spec:** [`docs/archive/specs/006-email-rendering`](../archive/specs/006-email-rendering/spec.md)
  (FR-001, FR-023a, FR-025; research R18)
- **Related:** [ADR 0032](0032-the-conversation-is-one-document.md) (whose
  one-document conversation this keeps, drawn by a different engine),
  [ADR 0003](0003-rich-text-compose.md) (whose reader statements point here
  now)
- **Decision:** **`postio-render` draws hostile mail in Postio's own
  process, so it may contain nothing that runs C, reaches the network or
  asks the system for fonts. Every byte it draws is handed to it; a remote
  one arrives only through the application's fetcher, on the user's
  consent. A caught render panic must stay catchable.**

---

## Context

Spec 006 moved the reader from WebKit, a separate process with its own
sandbox, to `postio-render` (Blitz, parley, vello_cpu) inside the
application. The maintainer accepted that on one condition: everything that
parses or decodes message content is memory-safe code, and the renderer
cannot reach the network by construction rather than by a setting. The
feature's reasoning, the evaluation and the alternatives are the spec's
(`research.md` R0, R3–R6, R18). This records only the rules other work must
keep obeying after the feature has landed.

## Decision

1. **No C, no native bindings.** Nothing in `postio-render`'s product graph
   (normal and build edges, features as a product build resolves them) links
   native code, is a `-sys` crate, or compiles C or C++ at build time. The
   image decoders are the pure-Rust `png`, `jpeg`, `gif` and `webp` paths;
   nothing else is enabled.
2. **No network.** No crate in that graph can open a connection:
   `blitz-dom`'s `net` feature stays off, and so does any HTTP, TLS or
   socket crate. The renderer resolves a URL from the resource table it is
   handed -- a message's parts, Postio's faces, data URIs -- and nothing
   else.
3. **No system font stack.** Fonts come from Postio's `FontSet`, built with
   `fontdb`; `fontique`'s and `parley`'s `system` features, and
   `blitz-dom`'s `system-fonts`, stay off. Fontconfig is C and reads
   configuration the user never thinks of as mail.
4. **Remote bytes come only through the application.** A remote image is
   fetched by `postio-runtime`'s `RemoteImageFetcher`, for a message the
   user opened whose sender they allowed or that they chose to show once,
   and handed to the renderer's table. The renderer never learns a URL it
   was not given bytes for.
5. **`panic = "unwind"`.** The render thread catches a panic and shows the
   plain-text fallback. A profile that links the renderer may not abort on
   panic, or a hostile message would take the application down.

## Enforcement

- `scripts/checks/check-renderer-is-memory-safe.py` walks the graph for
  rules 1--3 and 5 and fails naming the crate and the feature that pulled
  it in.
- `scripts/checks/check-crate-boundaries.py`'s `RULES["postio-render"]`
  bans the network and storage crates by name, on product edges.
- `postio-render`'s `egress` suite holds rule 4 in both directions (the
  classic app's `gtk_reader` did too, until T256): blocked mail reaches no
  loopback listener, and the same mail reaches it once consent is given. Focus's
  `focus_suite::remote_images` holds it in the app: nothing is asked for
  before Show, Show fetches once, and Always holds for that sender alone.

## Rejected

- **Keep WebKit for the reader.** Its sandbox is real, but its process per
  view, its start-up cost and its dark-mode behaviour were what spec 006
  existed to fix, and the evaluation (`docs/notes/2026-09-26-blitz-or-webkit.md`)
  chose Blitz.
- **A sandboxed render process.** Memory safety plus no network removes
  what a sandbox is for, at the cost of a process per reader again.
- **A setting that turns the network off.** A setting can be turned back
  on; a crate that is not in the graph cannot.

## Would change this

A format the renderer must decode that has no memory-safe decoder, or a
need to render remote content the user did not consent to. Either is a new
decision, not an exception to this one.
