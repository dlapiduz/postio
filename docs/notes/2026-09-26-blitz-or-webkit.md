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

### Arm A — WebKit (T022, `crates/postio-gtk/examples/eval_webkit.rs`)

**G1, legibility.** Run with research R10's rule: `postio_ui::reader::theme`'s
classification and repair, applied through an isolated-world script. The
count is over 95 text runs in 10 fixtures, drawn dark, with the pixels
sampled.

| | runs below the floor |
|---|---|
| as the reader draws them today (4.5:1) | **28** |
| with the rule (4.5:1) | **0** |
| with the rule, against the high-contrast floor (7:1) | **2** |

- **The two at 7:1** are white text on a sender's mid-blue button and card
  fill. No lightness of the text reaches 7:1 against those backgrounds, and
  FR-012 lets the rule change only the text. That is a gap in the spec, not
  a failure of either engine: at 7:1 the rule must be allowed to touch the
  background too, or high contrast accepts it. It is put to the
  maintainer with the scorecard.
- **Measured only after a fix arm A owes.** The reader's document does
  **not** follow libadwaita's dark mode. WebKit resolves
  `prefers-color-scheme` from GTK's own setting, and the only lever it
  follows, `gtk-application-prefer-dark-theme`, has been deprecated since
  GTK 4.20. This confirms at runtime the fourth cause in spec 006's
  Context. The fix arm A would owe is the editor's: pass the dark flag into
  the document itself.

**S1, fidelity.** 6 of 6 designed fixtures match their references, with
agreement between 98.3% and 100%. This was measured after the three
engine-neutral fixes below. Arm A's number is expected to be high, because
the references are WebKit; its question was only whether Postio's pipeline
loses anything.

### Arm B — Blitz (T023, `crates/postio-render/examples/eval_blitz.rs`)

Headless, with no display, no network crate, and fonts through `fontdb`
rather than fontconfig. The whole run takes about 3 s on a debug build:
font discovery, 6 fidelity renders, and 30 legibility layouts and paints.

**G1, legibility.** The same rule and the same pixel sampling as arm A.
Blitz's layout finds 88 text runs in the same 10 fixtures; arm A found 95,
because the two engines split runs differently.

| | runs below the floor |
|---|---|
| as drawn today (4.5:1) | **26** |
| with the rule (4.5:1) | **0** |
| with the rule, against the high-contrast floor (7:1) | **2** |

The two at 7:1 are the same two as arm A's: white on a sender's
mid-blue. So on legibility **the engines are at parity**, and the rule, not
the engine, is what fixes the bug. Two things arm B needed that arm A did
not:
- **An attribute mutation did not restyle.** In blitz-dom 0.3.0-beta.2, a
  `style` attribute set through `DocumentMutator` on a `<p>` reached
  neither computed style nor paint, through two resolves, while the same
  call on another element did. The harness applies overrides as a
  stylesheet on a fresh layout instead. A shipped renderer would re-lay
  out on a theme switch or a darken, which the render counts would have
  to budget for.
- **Its overrides need ID-level specificity.** A stylesheet rule loses to
  a sender's own `!important`, where arm A's inline `!important` wins. The
  harness uses `:is(#…, [stamp])` three times over.

**S1, fidelity.** **5 of 6** designed fixtures match: 83%, below SC-002's
95% bar.

| fixture | agreeing | match |
|---|---|---|
| `html-class-styled` | 97.6% | yes |
| `html-designed-three-column` | 99.1% | yes |
| `html-newsletter` | 88.8%, and the height drifts | **no** |
| `html-responsive-media` | 97.2% | yes |
| `html-transactional-receipt` | 95.3% | yes |
| `transactional-shipping-notice` | 98.4% | yes |

- **The failure is one known upstream defect.**
  `table { border-collapse: collapse }` is the standard email CSS reset,
  and Blitz paints a phantom dark grid on such a table when it has no
  borders at all. A one-cell probe paints 1,989 dark pixels with
  `collapse` and none with `separate`. Upstream has it as DioxusLabs/blitz
  #504, *"Collapsed table borders paint a phantom 3px grid for borderless
  tables"*, an open PR with a fix, not merged at the time of writing.
- **The counterfactual.** Forcing `separate`, which stands in for #504,
  gives **6 of 6** (97.2–99.1%). This was measured and labelled as such
  (`POSTIO_EVAL_COUNTERFACTUAL_504`), and it is not arm B's result.
- **Font fallback.** Without fontconfig, `Helvetica` and `Arial` resolve
  only because the harness aliases them to Liberation Sans. `Georgia` is
  not aliased, and falls to the bundled sans. Fontconfig's substitution
  table would have to be carried by Postio.

### Gates G2 and G3, both arms (T024)

Setup:
- every hostile fixture was rendered unconsented and consented;
- every remote URL was pointed at `postio_test_support::listener`, whose
  control connection was counted first;
- the harnesses were release builds;
- another session was idle-waiting on the machine, at a load average of
  about 2.

| | arm A (WebKit) | arm B (Blitz) |
|---|---|---|
| G2, unconsented: connections | **0** in every fixture | **0** in every fixture |
| G2, consented: connections | images only: 9 on `html-every-url-vector` (pixel, backgrounds, list marker, cursor, border image, generated content, the conditional rule's background, a cell `background`) and 4 on the tracking fixture. The web font and the `@import` were **not** fetched | **0**: the engine has no network code. Consented images would come through the app's fetcher (R12) |
| G3: slowest render | 374 ms on the first load, which includes starting the web process; 89 ms after that | 101 ms (`html-very-tall`); the rest are 1–10 ms |
| G3: crashes | none | **one panic class**, caught: see below |

- **Both arms pass G2 and G3.** Arm B passes only with one configuration
  its prototype lacked:
  - blitz-dom 0.3.0-beta.2 panics when a relative image URL (`<img
    src="x">`) is resolved against its default base, which is a `data:`
    URL and cannot be one;
  - the panic was contained by `catch_unwind`, as FR-023a requires, and
    would have fallen back to plain text;
  - but any message with a relative image, careless or hostile, would
    trigger it;
  - setting `DocumentConfig::base_url` removes it (re-measured: no panics).
- **SVG images did not paint in arm B at all.** Neither `cid:` nor
  `data:` did. The resource provider is asked and returns the bytes, and
  PNGs take the same route successfully, so the failure is inside Blitz's
  SVG parse or paint in this configuration. The cause was not found within
  the evaluation. It is recorded as an open risk against fidelity (mail
  uses SVG logos). It also makes research R5's local-file concern
  unobservable here: the magenta probe read 0 pixels because nothing drew.
- **Arm A's SVG local-file behaviour was not measured.** WebKit, like
  every browser, loads no external resources for SVG used as an image.

### S2 and S3, cost and blank frames (T026)

| | arm A (WebKit), `pane_comparison document N` | arm B (Blitz), `eval_blitz` cost |
|---|---|---|
| processes | the UI plus **1 web process** at 2, 10 and 50 messages (WebKit's network process is not counted by this instrument) | **1**: the app's own |
| memory | web Pss 83 / 96 / 97 MiB at 2 / 10 / 50 messages; UI RSS 150 MiB either way | render growth +12 / +9 / +47 MiB at 2 / 10 / 50. The prototype rasterises the whole document into one buffer; the plan's tiles cap that at 64 MiB. **Font discovery costs 198 MiB of Pss before anything renders**, because the prototype reads every installed font file into memory. A shipped renderer would have to memory-map or load lazily |
| time | handover 41–43 ms at every length (asynchronous: the UI waits for none of it) | first render 7.6 / 17.9 / 77.0 ms, warm 7.1 / 17.9 / 73.5 ms; the plan runs it off the UI thread |
| S3, blank frames | **not re-measured.** Reports stand open: a black frame between messages (#749, #947, still open) and on the reader↔composer swap (ADR 0034) | **not measurable at prototype depth**: there is no widget. By construction, one in-process surface with no second GL surface is what removes the black composite |

### S4 and S5, affordances and accessibility (T027)

- **S4, what exists today and what is still to build.** Counted from
  `tasks.md`:
  - **Arm A** already has selection and copy, find (`FindController`),
    zoom (`zoom-level`), printing, link handling and a screen-reader tree.
    What it owes is wiring: find and zoom to registry commands, the
    `[reader]` zoom setting, the document-level dark flag, and applying
    the rule through its isolated-world script. That is roughly **15–20
    tasks**.
  - **Arm B** has to build all of it: the Foundational phase (T030–T067,
    38 tasks), US4's selection, find, links, accessibility and script-free
    rail (T106–T119, 14), US6's zoom (T120–T131, 12), and the switch
    (T138–T153, 16). That is roughly **75 tasks**. At least 8 of them (the
    registry, config and zoom plumbing) are the same for both.
- **S5, accessibility.** Not walked: no AT-SPI client is installed on the
  machine. Instead, by construction:
  - **arm A:** WebKitGTK exposes the document's text, links, headings and
    tables to AT-SPI through GTK;
  - **arm B:** exposes nothing today. `AccessibleTextImpl` over the text
    index is T117–T118. `blitz-dom`'s own AccessKit tree would register a
    second AT-SPI root beside GTK's (the platform research), which is why
    the plan does not use it.

### S6–S8, security, maintenance and reach (T028)

- **S6, security posture.**
  - **Arm A:**
    - hostile HTML, CSS and images are parsed by WebKitGTK, a large C++
      engine, but in its **sandboxed** web process (bubblewrap and
      seccomp);
    - "no network" is a set of settings plus a CSP, **observed** at zero
      egress (G2) but not structural;
    - security fixes arrive with the distribution's WebKitGTK point
      releases, which ship CVE fixes regularly.
  - **Arm B:**
    - parsing is **memory-safe Rust** (html5ever, stylo, image, usvg,
      skrifa and harfrust), with fontconfig kept out;
    - "no network" is **structural**: there is no network crate in the
      graph;
    - but rendering runs in-process with no sandbox, so a panic or a hang
      is Postio's to contain. The evaluation found one panic class
      reachable from mail (a relative image URL); a configuration removes
      it.
- **S7, maintenance risk.**
  - **Arm A:** mature and stable API, distro-delivered.
  - **Arm B:** blitz-* 0.3.0-beta.2, with `main` pinning parley by git
    revision. The gaps that touched this corpus, all Postio's to carry
    until upstream lands them:
    - collapsed-border phantom grids (#504, an open PR);
    - `valign`, `cellpadding`/`cellspacing` and `<font>`, translated by
      Postio's hints;
    - SVG images not painting, cause unknown;
    - a style mutation that did not restyle;
    - a panic on a relative URL.
- **S8, platform reach.**
  - **Arm A** keeps one engine family on both platforms: the macOS
    frontend already renders with WKWebView (ADR 0019), and the
    isolated-world approach ports to WKWebView's user scripts.
  - **Arm B** could one day replace WKWebView too, which would give one
    renderer everywhere, but the macOS text, selection and accessibility
    integration would be built a second time.

## Scorecard

| | arm A — WebKit | arm B — Blitz |
|---|---|---|
| **G1** legible | **pass**, with the rule and a dark flag it owes: 28 → 0 at 4.5:1 | **pass**, with the rule: 26 → 0 at 4.5:1 |
| **G2** no egress | **pass**: 0 unconsented; images only when consented | **pass**: 0, structurally |
| **G3** survives | **pass**: ≤ 374 ms, no crash | **pass**, with a `base_url` configured; 1–101 ms |
| S1 fidelity | 6 of 6 (the references are WebKit's own) | 5 of 6; 6 of 6 with blitz#504 |
| S2 cost | ~85–100 MiB web process + UI; 42 ms asynchronous handover | no process; per-render growth bounded by tiles; font loading needs redesign |
| S3 blank frames | open reports (#749, #947) | not measurable yet; removed by construction |
| S4 affordance work | ~15–20 tasks | ~75 tasks |
| S5 accessibility | full, today | none, today |
| S6 security | sandboxed C++; network by setting | memory-safe in-process; network structurally absent |
| S7 maintenance | mature, distro-updated | beta; five gaps this corpus hit |
| S8 reach | same family as macOS today | one renderer everywhere, later |

## Recommendation

**WebKit, now; and keep Blitz measured.**

- **The engine was not the bug.** Every dark-on-dark run the user saw is
  fixed on both engines by the same engine-neutral work:
  - the canvas lifted, and classes kept;
  - Postio's type kept out of sender markup;
  - the R10 rule, which takes 26–28 failing runs to 0 on either arm.

  WebKit additionally owes one fix: tell the document it is dark.
- **Arm A wins on what a user feels and on cost to finish.** WebKit
  already has selection, find, accessibility, zoom and printing, matches
  every reference, and needs roughly a quarter of the remaining work.
- **Arm B's advantages are real, and not yet available.** Its structural
  "no network", its memory-safe parsing and its single surface arrive with
  a beta engine that failed to paint SVG, panicked on a relative URL,
  drew phantom table borders, and needs every reading affordance built from
  scratch.
- **Blitz stays cheap to reconsider.** `eval_blitz` re-runs in seconds.
  Re-evaluate when blitz 0.3 is final with #504 merged and SVG images
  painting.

**If the maintainer chooses WebKit**, T029 amends FR-001, FR-002, FR-023a
and SC-006 as research R0 lays out, and re-plans the engine-specific
phases. **If Blitz**, the plan stands, and the five arm B findings above
become its first tasks.

**A question for the maintainer either way.** At 7:1 (high contrast), two
runs on each arm cannot be repaired by changing the text colour alone:
white on a sender's mid-blue button. FR-012 lets the rule change only the
text. Should high contrast also be allowed to change a background, or does
it accept the residual?

### Engine-neutral defects the evaluation found

Found while measuring arm A, and fixed on the branch before either arm is
judged, because they cost both engines the same:

- **Embedded `data:` images were dropped, silently.** ammonia's default
  schemes exclude `data:`. `05987d24`.
- **The reader's own typography reached into sender markup**: border-box
  sizing, a 14 px size and 1.55 line height, paragraph margins, link and
  quote styles. It now stops at `.postio-original`. `aeab703a`.
- **`repair` met the floor before rounding and missed it after**: 4.48:1
  painted. `postio-ui` `theme.rs`.

## Amendments

*2026-09-26, before arm B has run and after arm A's first run:*

1. **S1 neutralizes the reader's container geometry in both arms.** The
   reader wraps a message in a padded, bordered box, and a reference is a
   bare page, so S1 measured Postio's chrome rather than the engine. Both
   harnesses now inject the same stylesheet. It paints the page white,
   removes the box's padding and border, and treats `.postio-canvas` as the
   sender's `<body>`: the browser's 8 px margin, which the sender's own
   inline style still overrides.
2. **References are drawn in standards mode.** A doctype is supplied where
   the sender wrote none. Every mail client embeds a message in a page of
   its own, so a sender never gets quirks mode where mail is read, and
   neither arm can reproduce it. Only `transactional-shipping-notice`
   changed (`aaee4fb9`).
3. **High contrast is measured as the dark render against 7:1.**
   libadwaita's high-contrast mode cannot be forced from a harness. Both
   arms do it the same way.
4. **The fidelity metric was amended before first use**, as its contract
   records: row alignment, and a no-lost-block rule.
5. **S5 was established by construction, not by an AT-SPI walk.** The
   machine has no AT-SPI client installed, and writing one was out of
   proportion to a criterion whose answer is not in doubt: one arm has a
   full tree and the other has none yet.
6. **S3 was not re-measured.** Arm B has no widget at prototype depth, and
   arm A's blank frames are already on record in open issues.
