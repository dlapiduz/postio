# ADR 0011 — The docs site

- **Status:** Accepted (2026-08-24)
- **Decision:** mdBook, deployed by its own workflow on push to `main`. Generated references are checked-in files that a test fails on when stale, so the deploy compiles nothing. `spec.md` is retired into `docs/PRODUCT.md`, which keeps its section numbers.

## Rules
- One Pages deployment, two surfaces: `/` is the hand-written landing page, `/docs/` the mdBook output. One workflow builds both.
- Generated references (`docs/keybindings.md` from the registry, `docs/config.md` from the config schema) are produced by tests that fail when the file on disk is stale. `POSTIO_UPDATE_DOCS=1` regenerates. The deploy does not regenerate.
- `config_doc.rs` owns a `(path, type, default, description)` table and asserts its paths equal the keys `Config::default()` serialises to; an undocumented field fails with its name. No `schemars`, no `syn`. `[keys]` renders from `registry::all()` and points at `keybindings.md`.
- `PRODUCT.md` keeps `spec.md`'s section numbers, because about eighty comments and tests cite them. A section whose content moved becomes a one-paragraph pointer. The site draws narrative from `PRODUCT.md`; second-person prose (install, FAQ) is written once, on the site.
- The site obeys the product's privacy rules (constitution): no analytics, no CDN, fonts self-hosted, no embedded video, no third-party search or comments. The privacy page says so.
- Deployment is a workflow of its own (`pages.yml`), on push to `main` and `workflow_dispatch`. `ci.yml` runs `mdbook build` to catch broken links and missing `SUMMARY.md` entries in review.

## Why
- Jekyll adds Ruby to a Rust toolchain; Hugo and Zola's theming serves the landing page, not forty reference pages.
- Review-time drift checks and readable files on GitHub are lost if references generate at deploy, which would also put a workspace build in the deploy path.
- Two documents describing one product, "kept in sync", drift.

## Revisit when
- The landing page outgrows hand-written HTML.
