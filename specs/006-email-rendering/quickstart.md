# Quickstart: validating faithful, readable rendering

These are runnable scenarios that prove the feature end to end, one per user
story plus the guarantees. The details live in the contracts; this file says
how to **see** each one. Run everything from the worktree
(`~/src/postio-worktrees/email-rendering`).

## Prerequisites

```bash
scripts/install-nextest.sh           # pinned test runner
scripts/install-shims.sh             # linker/cc shims (the claim and land scripts run it too)
```

The corpus fixtures and reference PNGs are checked in. Nothing here touches
the network except US5's loopback listener, which never leaves the machine.

## 0. The renderer is disconnected and memory-safe (FR-001, FR-023a)

```bash
python3 scripts/checks/check-crate-boundaries.py        # RULES["postio-render"]
python3 scripts/checks/check-renderer-is-memory-safe.py
cargo nextest run -p postio-render --test egress
```

**Expect:** both checks clean, and `egress` reports its control connection
counted and zero render connections. **To see a check bite:** add
`features = ["system-fonts"]` to `blitz-dom` in `crates/postio-render/Cargo.toml`
and re-run the second check. It must name `yeslogic-fontconfig-sys` and the
fix. Revert afterwards.

## 1. Legible in dark mode (US1, SC-001)

```bash
cargo nextest run -p postio-render --test contrast      # every cluster, every theme, pixels sampled
cargo nextest run -p postio-render --test presentation  # Paper / Adapted / SenderDark / Darkened per fixture
```

**Expect:** zero clusters below the floor in the light, dark and
high-contrast themes.

**By eye:** `scripts/run-isolated.sh --shot` with the app in dark mode on
three fixtures:
- `html-newsletter.eml`, which must show **paper**;
- a white-page reply fixture, which must be **dark, legible text**;
- the dark-aware fixture, which must show the **sender's dark design**.

Press `D` on the newsletter: it darkens. Press `D` again: it is back on paper.

## 2. As the sender built it (US2, SC-002)

```bash
cargo nextest run -p postio-body --lib sanitize          # class/id kept, canvas lifted, <font>/valign/cellpadding rewritten
cargo nextest run -p postio-render --test fidelity       # each designed fixture vs its WebKit reference
```

**Expect:** at least 95% of fixtures match under `contracts/fidelity-metric.md`,
and every mismatch is listed in `tests/reference/MISMATCHES.md` as cosmetic.
A failing fixture writes a diff image with the differing cells outlined.

## 3. Cannot betray the reader (US3, SC-003, SC-004)

```bash
cargo nextest run -p postio-render --test hostile        # beacons, script, overlay, 40k-px, malformed image, deep nesting
```

**Expect:**
- every hostile fixture either renders contained or reports
  `FellBack { .. }` within 400 ms;
- no panic escapes;
- no connection is made;
- no `LinkTarget` other than http, https, mailto, a verb or a fragment.

## 4. Read like everywhere else (US4, SC-007)

```bash
cargo nextest run -p postio-render --test text_index     # reading order, selection slices, find, link and fold hits
cargo nextest run -p postio-gtk --test gtk_suite body_view
```

**Expect** the widget cases to cover:
- a drag selecting across table cells copies tab- and newline-separated
  text;
- `mod+f` highlights every match, and `mod+g` walks them;
- link hover shows the target;
- the accessible text equals `TextIndex.text`;
- scrolling to the end of the 40,000 px fixture reaches its last line, with
  tile memory under 64 MiB.

## 5. Allowed senders' images (US5)

```bash
cargo nextest run -p postio-app --test app_suite remote_images_allowed
```

**Expect:**
- an allowed sender's image arrives from the loopback listener and is
  painted;
- the request carries no `Cookie`, `Referer` or `Origin`;
- with consent revoked, the listener's control counts and the render counts
  zero;
- moving the cursor across unopened messages makes zero connections.

## 6. Zoom (US6, SC-009)

```bash
cargo nextest run -p postio-render --test zoom           # every step 50–300%: nothing clipped, re-flow, anchor kept
cargo nextest run -p postio-gtk --test gtk_suite body_view_zoom
```

**By eye:** `mod+plus` twice on the newsletter shows 125% in the indicator,
the column design re-flows narrower, and the chrome is unchanged. Restart the
app: still 125%. `mod+0` resets it, and the indicator disappears.

## 7. The switch (FR-027, FR-030, SC-006)

```bash
grep -rn "webkit6" crates/postio-gtk/src/reader crates/postio-gtk/src/conversation.rs   # expect nothing
cargo nextest run -p postio-app --test app_suite          # wiring: open, theme switch, find, zoom reach the pane
```

**Expect:**
- no reader code imports WebKit;
- opening conversations starts no web process (the composer still may);
- the render counts per navigation hold.

## Before landing

Run the suites the diff touched (above), then
`scripts/issue-land.sh --detach`. The default tier is enough, and CI and the
nightly run the rest.
