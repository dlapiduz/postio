# Blitz or WebKit: the reading engine, evaluated

*2026-09-26, spec 006 (`specs/006-email-rendering`), research R0.*

The maintainer's question: *"we still need to evaluate whether we should use
blitz or webkit"*. Spec 006's plan had taken Blitz from the spike (#1543), and
the spike never measured WebKit on the same terms. This note is the
evaluation.

**The protocol below was committed before either arm produced a result**,
and its gates, criteria, fixtures and decision rule do not move to fit one.
The results section is filled in by T024–T028 and was empty when the
protocol was committed. A change to the protocol after that point is its own
dated paragraph under *Amendments*, saying why.

## The question

Which engine should draw message bodies in the GTK reading pane?

- **A — WebKit, improved.** WebKitGTK 6 as today: the hardened `Reader`, one
  document per conversation (ADR 0032). It renders the engine-neutral work
  spec 006 has already landed.
- **B — Blitz.** A new `postio-render` crate, in-process: `blitz-*`
  `=0.3.0-beta.2`, CPU raster, no network crate, no C.

Both arms get the **same input**. That input is the production pipeline as
of the engine-neutral phase:
- the sanitizer keeps classes and ids, lifts the canvas, carries the
  `color-scheme`, translates presentational hints and lists its refusals;
- `postio-ui` composes the document;
- every message opens in its original layout.

Neither arm is judged on work that would help both.

## The machine

- Intel Core i7-8559U, 8 threads, 31 GiB RAM.
- Fedora 44, kernel 7.2.7, Wayland.
- GTK 4.22.5, libadwaita 1.9.4, WebKitGTK 2.54.0.
- rustc 1.98.0.

Both arms are measured on this machine, one after the other, never
concurrently with each other or with another session's build. That is
checked with `scripts/lanes` before each timing run. Timings are taken on
release builds of the harnesses; correctness measures run in any profile.

## Fixtures

All come from `crates/postio-model/tests/corpus/`, selected by category:

| Used for | Fixtures |
|---|---|
| Fidelity (S1) | every `designed` fixture that has a reference in `crates/postio-test-support/data/reference/` (six at the time of writing) |
| Legibility (G1) | every `theme-contrast` fixture, plus `html-newsletter`, `html-designed-three-column`, `html-legacy-font-center`, `plain-text-simple`, `multipart-alternative` |
| Hostility (G2, G3) | every `hostile` fixture |
| Cost (S2, S3) | the #1348 thread from `postio-app`'s `pane_comparison` example, at 2, 10 and 50 messages |
| Accessibility (S5) | `html-transactional-receipt` |

## Gates

An arm that fails a gate cannot be recommended. It can still be chosen if a
fix is shown to fit within spec 006's branch; the note then says what the
fix is.

- **G1 — Legible (spec 006 SC-001).** With the arm's prototype of research
  R10's rule, meaning the classification plus the OKLCH contrast repair:
  - render each legibility fixture in dark and in high contrast;
  - for every text run, sample the rendered pixels behind it, excluding
    glyph pixels, and take its colour;
  - **pass when zero runs are below 4.5:1 in dark, and zero below 7:1 in
    high contrast**, for text of every size. There is no large-text
    allowance (FR-012).

  How each arm finds its text runs:
  - arm A reads `{rect, color}` with Postio's isolated-world script
    (`Range.getClientRects`, `getComputedStyle`);
  - arm B reads them from its layout.

  Each arm also reports its count **before** repair: today's bug, measured.
- **G2 — No egress (SC-003).** Each hostile fixture is rendered in a live
  reader, arm A, or through the renderer, arm B, twice: unconsented, and
  consented. Every remote URL is rewritten to a loopback listener, and the
  listener's control connection is counted first. **Pass when renders
  cause zero accepted connections.** In the consented case, arm A may fetch
  images, because that is today's consent path. The note records what it
  fetched.
- **G3 — Survives (SC-004).** Each hostile fixture is opened. **Pass on no
  crash, and no render still running at 400 ms.** A render that falls back
  in time is a pass, and is counted.

## Scored criteria

These are reported side by side. None of them alone decides.

| | Criterion | Measure |
|---|---|---|
| S1 | Fidelity | `postio_test_support::fidelity` against the references, per fixture, with the contract's verdict. **Known bias:** the references are WebKit, so arm A's score measures sanitizer loss only, and arm B's measures sanitizer plus engine loss. For arm B the question is "≥ 95% match?", not "as good as A?". |
| S2 | Cost | Pss of every process the reader keeps (web and network processes for A, the app's own growth for B), process count, first and warm render time, and handover at 2, 10 and 50 messages |
| S3 | Blank frames | frames showing only the ground colour across 50 message-to-message navigations, sampled from the widget |
| S4 | Affordance parity | for each of US4 (select, copy, find, links, screen reader, long messages) and US6 (zoom): built in, or the `tasks.md` tasks still needed |
| S5 | Accessibility | an AT-SPI walk of one message body: the text, links and headings a screen reader receives |
| S6 | Security posture | which code parses hostile input (C/C++ or memory-safe), process isolation, and whether "no network" is structural or a setting, with the evidence cited |
| S7 | Maintenance risk | upstream maturity, release cadence, how security fixes reach users, and the open gaps that matter to mail |
| S8 | Platform reach | what each arm means for the macOS frontend, which renders with WKWebView today (ADR 0019) |

## Decision rule

- **The maintainer decides.** This note ends in a scorecard and a
  recommendation. The decision is recorded in `spec.md`'s Clarifications,
  and the spec, plan and tasks are brought into line with it (T029).
- **If only one arm passes all three gates,** it is recommended.
- **If both pass,** the recommendation weighs fidelity (S1), blank frames
  (S3) and security posture (S6) against cost (S2), affordance work (S4) and
  maintenance risk (S7). Every trade is stated, not netted into a score.
- **If neither passes,** the note says which gate each fails, what the
  cheapest fix is, and recommends the arm whose fix is smaller.

## Results

*Empty when the protocol was committed. Filled in by T024 (gates), T025
(S1), T026 (S2, S3), T027 (S4, S5) and T028 (S6–S8, the scorecard and the
recommendation).*

## Amendments

*None.*
