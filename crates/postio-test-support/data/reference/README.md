# Reference renders (spec 006, SC-002)

WebKitGTK renders of each `designed` corpus fixture, **unsanitized**, at 800 CSS px,
scale 1, light, with network and script off and the bundled faces as the default
families. Written by `cargo run -p postio-gtk --example capture_reference`; compared
with `postio_test_support::fidelity` under
`specs/006-email-rendering/contracts/fidelity-metric.md`. Do not regenerate to make
a comparison pass: a new capture is a new baseline, and says why in its commit.

| Fixture | Size | WebKitGTK | Captured |
|---|---|---|---|
| `html-class-styled` | 800x262 | 2.54.0 | 2026-09-26 |
| `html-designed-three-column` | 800x656 | 2.54.0 | 2026-09-26 |
| `html-newsletter` | 800x516 | 2.54.0 | 2026-09-26 |
| `html-responsive-media` | 800x178 | 2.54.0 | 2026-09-26 |
| `html-transactional-receipt` | 800x319 | 2.54.0 | 2026-09-26 |
| `transactional-shipping-notice` | 800x201 | 2.54.0 | 2026-09-26 |
