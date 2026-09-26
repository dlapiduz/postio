# Contract: the fidelity metric (SC-002)

**These constants are fixed by this document before the first comparison
runs, and they are not loosened to make a fixture pass** (spec SC-002). A
change to them is a spec change and needs a commit that says why.

## Reference renders

- **Tool.** `crates/postio-gtk/examples/capture_reference.rs`. It is run by
  hand, needs a display, and must be run **while WebKit is still the reader's
  engine** (Phase 1).
- **Input.** Each *designed* fixture in the rendering corpus, as the
  **unsanitized** HTML body. Its `cid:` parts are served from the fixture.
  Network, JavaScript and remote loads are off.
- **Viewport.** 800 CSS px wide, scale 1, light scheme. The page height is
  the document height.
- **Fonts.** The bundled faces are installed as the default sans, serif and
  monospace families for the capture.
- **Output.** `crates/postio-test-support/data/reference/<fixture>.png`, plus a
  line in `reference/README.md` naming the fixture, the WebKitGTK version and
  the capture date.

## Comparison

The candidate is the chosen engine's render of the same fixture through the
**production pipeline**: sanitize, compose, render. It uses the same
viewport, light theme and zoom 100.

1. Both images are cut into **16 × 16 px cells**. A partial cell at an edge
   is averaged over its real pixels.
2. Each cell becomes its mean colour in **OKLab**.
3. The cell difference is the Euclidean distance **ΔE_OK**.
4. A fixture **matches** when both of these hold:
   - **≥ 92%** of cells have **ΔE_OK ≤ 0.08**;
   - the heights agree within **±8%**, computed over the shorter height's
     cells. The extra rows of the taller image count as mismatched cells.
5. **SC-002 passes** when **≥ 95%** of designed fixtures match.

Every fixture that does not match is listed in
`crates/postio-test-support/data/reference/MISMATCHES.md`, with its cause and
with *cosmetic* or *content-losing* stated. A content-losing mismatch fails
SC-002 regardless of the 95%.

## Failure output

The test writes a diff image to the test's scratch directory, with mismatched
cells outlined, and prints the cell coordinates. "These cells differ" is the
message, not a similarity score.

## Why these numbers

- **16 px cells** are small enough that a collapsed 3 × 70 px column changes
  whole cells, and large enough that glyph shapes from two different font
  rasterisers average out.
- **ΔE_OK 0.08** is roughly a just-noticeable difference in flat colour. It
  admits anti-aliasing noise and rejects a wrong background.
- **92% / ±8%** admit line-wrapping drift from differing font metrics, which
  shifts some rows, but not a missing block.

If the first honest run shows the constants mismatch reality, the change is
made **once**, recorded in `research.md` R13 with the evidence, and **before**
any renderer change is judged by it.
