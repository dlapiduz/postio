# Mismatches against the reference renders

What `crates/postio-render/tests/fidelity.rs` found when it compared the
renderer with these WebKit references, and why. A fixture that does not match
must be listed here as `cosmetic`, with its cause, or the test fails
(`specs/006-email-rendering/contracts/fidelity-metric.md`).

## First run: 2026-09-26

**6 of 6 match.** Nothing is listed, and the metric's constants are unchanged
from the contract.

| Fixture | Verdict |
|---|---|
| `html-class-styled` | match |
| `html-designed-three-column` | match |
| `html-newsletter` | match |
| `html-responsive-media` | match |
| `html-transactional-receipt` | match |
| `transactional-shipping-notice` | match |

The one mismatch the engine evaluation found (`html-newsletter`, 88.8%) was
Blitz's collapsed-border grid, upstream #504. It is patched in-tree
(`patches/blitz/0001-collapsed-borders-only-where-they-are-drawn.patch`,
research R1), and with it the newsletter reaches 98.4%.
