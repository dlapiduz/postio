# ADR 0011 — The docs site

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#25 Docs site (GitHub Pages)](https://github.com/dlapiduz/postio/issues/25)
- **Related:** `docs/ARCHITECTURE.md` §2 (one registry, every surface) and §11
  (privacy), `crates/postio-core/tests/keybindings_doc.rs`
- **Decision:** **mdBook**, deployed by its own workflow on push to `main`.
  Generated pages are produced by **tests that fail when the file on disk is
  stale** — the mechanism `docs/keybindings.md` already uses — so the site
  build compiles nothing and CI, not the deploy, is what keeps the references
  honest. The narrative is **written for users and is not a copy of
  `spec.md`.**

---

## What is already built

The hard half. `docs/keybindings.md` is generated from
`postio-core::registry::all()` by `crates/postio-core/tests/keybindings_doc.rs`,
which **fails when the checked-in file no longer matches the registry** and
regenerates it under `POSTIO_UPDATE_DOCS=1`. It carries its own "do not edit by
hand" banner. That is exactly what the issue's first acceptance criterion asks
for, one output format short.

So the shortcut reference is not a thing to build. It is a thing to **publish**.

---

## Q1 — Which static site generator?

The decision the issue leaves open. The candidates, judged against what this
repository actually is:

| | Fit |
|---|---|
| **mdBook** | Rust; one `cargo install` in CI; input is Markdown this project already writes; built-in client-side search; the reference-manual shape the content is |
| Jekyll (Pages default) | Requires Ruby in the toolchain of a Rust project, for no capability mdBook lacks here |
| Hugo / Zola | Both good and both general-purpose. Their advantage is theming for marketing pages, which is the *landing page's* problem, not this one |
| Hand-written HTML | What the landing page should be; wrong for forty pages of reference |

**Decision: mdBook.** Its input is the Markdown the repository already produces
— including the generated files, unchanged — and it adds one tool to CI rather
than one language.

**One Pages deployment, two surfaces.** GitHub Pages serves one site per
repository, and the landing page (its own issue) is deliberately *not*
mdBook-shaped: it wants real screenshots, the north-star line as a headline, and
prose. So:

```
  /            hand-written landing page
  /docs/       mdBook output
```

One workflow builds both and deploys once. The landing page links into `/docs/`;
the book links back. Keeping them in one deployment avoids a second domain, a
second workflow, and the split-brain where one is updated and the other is not.

---

## Q2 — How the generated references get into the book

**The generator is a test, and the output is checked in.** This is already the
idiom; the ADR's job is to make it the rule rather than a thing one file does.

```
  postio-core::registry  ──►  tests/keybindings_doc.rs  ──►  docs/keybindings.md
  postio-config schema   ──►  tests/config_doc.rs       ──►  docs/config.md
                                                             │
                                                    mdBook includes them
```

Three properties, and the third is why this shape rather than a build step:

1. **CI catches drift, not the deploy.** `cargo test -p postio-core` already
   fails on a stale keybindings file. Adding the config reference the same way
   means a PR that changes the schema and not the docs is red *in review*,
   which is where a stale doc is cheap to fix.
2. **The site build compiles nothing.** No GTK, no WebKit, no `cargo build` —
   the deploy job is mdBook over checked-in Markdown on `ubuntu-latest`. The
   repository's build cost is already the binding constraint, and a docs deploy
   that rebuilt the workspace would be the most expensive job in the project for
   the least reason.
3. **The generated file is readable in the repository.** Someone reading
   `docs/keybindings.md` on GitHub gets the same reference as someone reading
   the site, which is the reason it was written that way in the first place.

---

## Q3 — Generating the config reference without reflection

`postio-config` is a set of `serde` structs with doc comments, and Rust has no
reflection to walk them at runtime. Two tempting answers are both wrong here:

- **`schemars` as a dependency.** It would make doc comments into schema
  descriptions for free. But `postio-config` is depended on by `postio-core`,
  and `ARCHITECTURE.md` §9's feature-unification argument applies: a `schema`
  feature would union into every crate that touches config the moment anything
  enabled it, and an unconditional dependency puts a schema library in the
  graph of the whole workspace to render one Markdown page.
- **Parsing the source with `syn` in a build script.** A second, fragile model
  of the schema, which drifts in a way nothing detects.

**Decision: `tests/config_doc.rs` owns a table of `(path, type, default,
description)`, renders the Markdown from it, and asserts that the table's paths
are exactly the keys a default `Config` serialises to.**

The second half is the whole design. `postio-config` already round-trips
through `toml`, so serialising a `Config::default()` yields every key the schema
has. Compare that key set to the documented key set, and **adding a field
without documenting it fails the test with the field's name in the message.**
No reflection, no new dependency, and the failure mode the issue is worried
about — a reference that drifts — is the one that is caught.

The `[keys]` section is the exception: it is not a fixed key set but one entry
per command id, so it renders from `registry::all()` and points at
`docs/keybindings.md` rather than repeating it.

---

## Q4 — The narrative, and what "kept in sync with `spec.md`" has to mean

The issue asks that narrative content be *"drawn from `spec.md`, kept in sync
rather than forked into a second copy"*. Taken literally that produces two
documents that drift, because "kept in sync" is a habit and habits are the thing
generation exists to replace.

**Decision: they are different documents for different readers, and the
overlap is removed rather than synchronised.**

- `spec.md` is the **contributor's** document: what to build and why, decisions
  and their reasons, open questions. It stays.
- The docs site is the **user's** document: what Postio does, how to install it,
  what the keys are, what the config options mean, what the privacy posture
  is — written in the second person, with screenshots.

Where they would overlap on *behaviour* — what `a` does, what gets blocked in
the reader, how sync behaves offline — the site is authoritative and `spec.md`
links to it. That is one source per fact, which is the same rule §2 applies to
the command surfaces.

Sections, matching the issue: what Postio is · install · keyboard reference
(generated) · `config.toml` reference (generated) · how sync works · privacy and
security · FAQ.

---

## Q5 — The site holds itself to Postio's own rules

A privacy-first mail client whose documentation loads a third-party font and an
analytics beacon is making a claim its own website contradicts.

- **No analytics.** Not Google's, not a self-hosted one, not "privacy-friendly"
  ones. `ARCHITECTURE.md` §11 says no telemetry; a page-view counter on the
  docs is telemetry about the people reading them.
- **No CDN, no external fonts.** Barlow, Barlow Condensed and IBM Plex Mono are
  the design system's faces and are self-hosted from the site's own origin, so
  no request leaves the reader's browser for a third party.
- **No embedded video, no third-party search, no comment widget.** mdBook's
  search is client-side and ships in the bundle.
- **The privacy page says all of the above** and is the page most worth having,
  because it is the claim a prospective user is deciding whether to believe.

---

## Q6 — Deployment

A workflow of its own, not a job in `ci.yml`, for the same reason `release.yml`
is separate: it says nothing new about a commit and it should not be in the
critical path of a PR.

- Triggers on push to `main` and on `workflow_dispatch`.
- Builds the landing page and the book, uploads one Pages artifact, deploys.
- **Does not regenerate anything.** If the checked-in Markdown is stale, `ci.yml`
  already failed on the PR that made it stale. A deploy that regenerated would
  quietly paper over exactly the drift the tests exist to surface.
- `ci.yml` gains a fast job that runs `mdbook build` to catch a broken link or
  a missing `SUMMARY.md` entry in review rather than after merge.

---

## Alternatives

**Jekyll, because it is what Pages does by default.** Adds Ruby to a Rust
project's toolchain for no capability gained here.

**Generate the whole site from `spec.md`.** Would produce a spec with a
stylesheet. The reader who needs this site is not the reader `spec.md` was
written for, and Q4's separation is the point rather than a compromise.

**Generate the references at deploy time by running the tests.** Removes the
checked-in copies, and with them the review-time drift check and the readable
`docs/keybindings.md` on GitHub. It also puts a `cargo build` of this workspace
in the deploy path.

**Hand-write the shortcut and config references.** What the issue exists to
prevent, and what §2 already rules out for every other command surface.

---

## Consequences

- New: `docs/book/` (mdBook sources and `SUMMARY.md`), `site/` (landing page),
  `.github/workflows/pages.yml`.
- New: `crates/postio-config/tests/config_doc.rs` and `docs/config.md`,
  following `keybindings_doc.rs` exactly, including `POSTIO_UPDATE_DOCS=1`.
- `docs/keybindings.md` is included into the book unchanged; it keeps its
  banner and its generator.
- `spec.md` loses its behavioural duplication and gains links.
- Every future user-visible surface inherits an obligation: if it has a
  reference, the reference is generated and its generator is a test.
