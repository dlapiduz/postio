//! Does a render look like its reference? (spec 006, SC-002)
//!
//! The rules are `specs/006-email-rendering/contracts/fidelity-metric.md`,
//! and the constants below are that contract's. They are fixed before any
//! engine is judged by them, and are not loosened to make a fixture pass.
//!
//! # What is compared, and what is not
//!
//! Two engines draw the same glyphs differently, and the question here is
//! not whose antialiasing is nicer: it is whether the columns, blocks,
//! backgrounds and images the sender built are where the sender put them.
//! So both images are cut into [`CELL`]-pixel cells, each reduced to its mean
//! colour in OKLab, and cells are compared — glyph shapes average out, a
//! missing block does not.
//!
//! Two failure shapes a plain cell-by-cell comparison gets wrong, both found
//! on synthetic images before any engine was compared:
//!
//! - **Drift.** One extra wrapped line moves everything below it by a line,
//!   and every later cell then disagrees with the cell beside it although
//!   nothing is missing. So rows are aligned first, as a text diff aligns
//!   lines ([`align_rows`]): an inserted or dropped row costs itself, not
//!   everything after it.
//! - **A lost block.** A whole card can go missing while 98% of cells still
//!   agree, so the percentage alone passes it. So a match also requires that
//!   no [`BLOCK`]×[`BLOCK`] square of aligned cells is entirely wrong.

use std::path::Path;

/// Cell edge, in pixels.
pub const CELL: usize = 16;
/// Largest OKLab distance at which two cell means still count as the same.
pub const MAX_DELTA_E: f64 = 0.08;
/// Share of aligned cells that must agree for a fixture to match.
pub const MIN_MATCHING: f64 = 0.92;
/// Largest relative difference in height between reference and candidate.
pub const MAX_HEIGHT_DRIFT: f64 = 0.08;
/// Edge of the square of cells that may not be wholly wrong: 3 cells, 48 px.
pub const BLOCK: usize = 3;

/// An RGBA image, tightly packed.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height` pixels, four bytes each.
    pub rgba: Vec<u8>,
}

/// What a comparison found.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Aligned cell pairs compared.
    pub cells: usize,
    /// How many of them agreed within [`MAX_DELTA_E`].
    pub agreeing: usize,
    /// Reference cells, as `(column, row)`, that disagreed with their aligned
    /// candidate cell — or had none to align with.
    pub mismatched: Vec<(usize, usize)>,
    /// Whether the heights are within [`MAX_HEIGHT_DRIFT`].
    pub height_ok: bool,
    /// Whether some [`BLOCK`]-square of aligned cells disagreed entirely.
    pub lost_block: bool,
}

impl Comparison {
    /// The contract's verdict: enough agreement, no lost block, similar height.
    pub fn matches(&self) -> bool {
        self.height_ok
            && !self.lost_block
            && self.cells > 0
            && self.agreeing as f64 / self.cells as f64 >= MIN_MATCHING
    }
}

impl Image {
    /// Build from tightly packed RGBA.
    pub fn from_rgba(width: usize, height: usize, rgba: Vec<u8>) -> Image {
        assert_eq!(rgba.len(), width * height * 4, "tightly packed RGBA");
        Image {
            width,
            height,
            rgba,
        }
    }

    /// Decode a PNG from disk into RGBA, whatever its colour type.
    pub fn load_png(path: &Path) -> Image {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut buf = vec![0; reader.output_buffer_size().expect("a bounded frame")];
        let info = reader
            .next_frame(&mut buf)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (width, height) = (info.width as usize, info.height as usize);
        let channels = info.color_type.samples();
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let row = &buf[y * info.line_size..y * info.line_size + width * channels];
            for px in row.chunks_exact(channels) {
                match channels {
                    1 => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
                    2 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
                    3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
                    _ => rgba.extend_from_slice(&px[..4]),
                }
            }
        }
        Image::from_rgba(width, height, rgba)
    }

    /// Encode as an RGBA PNG on disk.
    pub fn save_png(&self, path: &Path) {
        let file =
            std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            self.width as u32,
            self.height as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("a PNG header");
        writer.write_image_data(&self.rgba).expect("PNG data");
    }

    /// The pixel at `(x, y)` as OKLab, composited over white.
    fn oklab(&self, x: usize, y: usize) -> [f64; 3] {
        let at = (y * self.width + x) * 4;
        let alpha = f64::from(self.rgba[at + 3]) / 255.0;
        let channel = |i: usize| {
            let c = f64::from(self.rgba[at + i]) / 255.0 * alpha + (1.0 - alpha);
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (channel(0), channel(1), channel(2));
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        [
            0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
            1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
            0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
        ]
    }

    /// Each pixel row reduced to one mean OKLab colour per [`CELL`]-wide strip.
    fn row_signatures(&self, columns: usize) -> Vec<Vec<[f64; 3]>> {
        (0..self.height)
            .map(|y| {
                (0..columns)
                    .map(|column| {
                        let (x0, x1) = (column * CELL, ((column + 1) * CELL).min(self.width));
                        if x0 >= x1 {
                            return [1.0, 0.0, 0.0];
                        }
                        let mut sum = [0.0; 3];
                        for x in x0..x1 {
                            let lab = self.oklab(x, y);
                            for i in 0..3 {
                                sum[i] += lab[i];
                            }
                        }
                        let n = (x1 - x0) as f64;
                        [sum[0] / n, sum[1] / n, sum[2] / n]
                    })
                    .collect()
            })
            .collect()
    }
}

fn delta_e(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn row_cost(a: &[[f64; 3]], b: &[[f64; 3]]) -> f64 {
    a.iter().zip(b).map(|(x, y)| delta_e(*x, *y)).sum::<f64>() / a.len().max(1) as f64
}

/// Align pixel rows of a candidate to those of a reference, as a text diff
/// aligns lines: a banded global alignment in which substituting one row for
/// another costs their mean distance and skipping a row costs a mismatch.
///
/// Returns, for each reference row, the candidate row aligned with it, or
/// `None` where the reference row has no counterpart (content the candidate
/// lost). Candidate rows aligned with nothing (content the candidate added)
/// do not appear: extra content is answered by the height rule.
pub fn align_rows(reference: &[Vec<[f64; 3]>], candidate: &[Vec<[f64; 3]>]) -> Vec<Option<usize>> {
    let (n, m) = (reference.len(), candidate.len());
    let band = 32.max(n.max(m) / 10) + n.abs_diff(m);
    let gap = MAX_DELTA_E;
    let inf = f64::INFINITY;
    // cost[i][j]: best cost aligning reference[..i] with candidate[..j].
    let width = m + 1;
    let mut cost = vec![inf; (n + 1) * width];
    let mut step = vec![0u8; (n + 1) * width]; // 1 diagonal, 2 up (skip reference), 3 left (skip candidate)
    cost[0] = 0.0;
    for i in 0..=n {
        let lo = i.saturating_sub(band);
        let hi = (i + band).min(m);
        for j in lo..=hi {
            if i == 0 && j == 0 {
                continue;
            }
            let mut best = inf;
            let mut how = 0;
            if i > 0 && j > 0 && cost[(i - 1) * width + j - 1] < inf {
                let c =
                    cost[(i - 1) * width + j - 1] + row_cost(&reference[i - 1], &candidate[j - 1]);
                if c < best {
                    best = c;
                    how = 1;
                }
            }
            if i > 0 && cost[(i - 1) * width + j] < inf {
                let c = cost[(i - 1) * width + j] + gap;
                if c < best {
                    best = c;
                    how = 2;
                }
            }
            if j > 0 && cost[i * width + j - 1] < inf {
                let c = cost[i * width + j - 1] + gap;
                if c < best {
                    best = c;
                    how = 3;
                }
            }
            cost[i * width + j] = best;
            step[i * width + j] = how;
        }
    }
    let mut aligned = vec![None; n];
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match step[i * width + j] {
            1 => {
                aligned[i - 1] = Some(j - 1);
                i -= 1;
                j -= 1;
            }
            2 => i -= 1,
            3 => j -= 1,
            _ => unreachable!("the band always reaches the corner"),
        }
    }
    aligned
}

/// Compare `candidate` with `reference` under the contract.
pub fn compare(reference: &Image, candidate: &Image) -> Comparison {
    let columns = reference.width.div_ceil(CELL);
    let rows = reference.height.div_ceil(CELL);
    let reference_rows = reference.row_signatures(columns);
    let candidate_rows = candidate.row_signatures(columns);
    let aligned = align_rows(&reference_rows, &candidate_rows);

    let mut wrong = vec![false; columns * rows];
    let mut mismatched = Vec::new();
    for row in 0..rows {
        let (y0, y1) = (row * CELL, ((row + 1) * CELL).min(reference.height));
        for column in 0..columns {
            let mut sum_r = [0.0; 3];
            let mut sum_c = [0.0; 3];
            let mut present = 0usize;
            for y in y0..y1 {
                if let Some(cy) = aligned[y] {
                    for i in 0..3 {
                        sum_r[i] += reference_rows[y][column][i];
                        sum_c[i] += candidate_rows[cy][column][i];
                    }
                    present += 1;
                }
            }
            let lost = present * 4 < (y1 - y0) * 3; // more than a quarter of the cell has no counterpart
            let differs = present > 0 && {
                let n = present as f64;
                delta_e(
                    [sum_r[0] / n, sum_r[1] / n, sum_r[2] / n],
                    [sum_c[0] / n, sum_c[1] / n, sum_c[2] / n],
                ) > MAX_DELTA_E
            };
            if lost || differs {
                wrong[row * columns + column] = true;
                mismatched.push((column, row));
            }
        }
    }
    let lost_block = (0..rows.saturating_sub(BLOCK - 1)).any(|row| {
        (0..columns.saturating_sub(BLOCK - 1)).any(|column| {
            (0..BLOCK).all(|dy| (0..BLOCK).all(|dx| wrong[(row + dy) * columns + column + dx]))
        })
    });
    let cells = columns * rows;
    let drift = reference.height.abs_diff(candidate.height) as f64 / reference.height.max(1) as f64;
    Comparison {
        cells,
        agreeing: cells - mismatched.len(),
        mismatched,
        height_ok: drift <= MAX_HEIGHT_DRIFT,
        lost_block,
    }
}

/// The reference with every mismatched cell outlined in magenta.
pub fn diff_image(reference: &Image, comparison: &Comparison) -> Image {
    let mut out = reference.clone();
    for &(column, row) in &comparison.mismatched {
        let (x0, y0) = (column * CELL, row * CELL);
        let (x1, y1) = (
            ((column + 1) * CELL).min(out.width),
            ((row + 1) * CELL).min(out.height),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                if x == x0 || y == y0 || x == x1 - 1 || y == y1 - 1 {
                    let at = (y * out.width + x) * 4;
                    out.rgba[at..at + 4].copy_from_slice(&[255, 0, 255, 255]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reference(name: &str) -> Image {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data/reference")
            .join(format!("{name}.png"));
        Image::load_png(&path)
    }

    fn fill(image: &mut Image, x0: usize, y0: usize, w: usize, h: usize, rgba: [u8; 4]) {
        for y in y0..(y0 + h).min(image.height) {
            for x in x0..(x0 + w).min(image.width) {
                let at = (y * image.width + x) * 4;
                image.rgba[at..at + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn pixel(image: &Image, x: usize, y: usize) -> [u8; 4] {
        let at = (y * image.width + x) * 4;
        image.rgba[at..at + 4].try_into().unwrap()
    }

    /// A 3x3 box blur, one pixel each way: the scale of the difference two
    /// font rasterisers make, and nothing like a layout difference.
    fn blur(image: &Image) -> Image {
        let mut out = image.clone();
        for y in 0..image.height {
            for x in 0..image.width {
                let mut sum = [0u32; 4];
                let mut n = 0;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx >= 0
                            && ny >= 0
                            && (nx as usize) < image.width
                            && (ny as usize) < image.height
                        {
                            let p = pixel(image, nx as usize, ny as usize);
                            for (total, value) in sum.iter_mut().zip(p) {
                                *total += u32::from(value);
                            }
                            n += 1;
                        }
                    }
                }
                let at = (y * image.width + x) * 4;
                for (slot, total) in out.rgba[at..at + 4].iter_mut().zip(sum) {
                    *slot = (total / n) as u8;
                }
            }
        }
        out
    }

    /// Insert `rows` rows of `rgba` at `at`, as an extra wrapped line would.
    fn insert_band(image: &Image, at: usize, rows: usize, rgba: [u8; 4]) -> Image {
        let row_bytes = image.width * 4;
        let mut out = Vec::with_capacity(image.rgba.len() + rows * row_bytes);
        out.extend_from_slice(&image.rgba[..at * row_bytes]);
        for _ in 0..rows * image.width {
            out.extend_from_slice(&rgba);
        }
        out.extend_from_slice(&image.rgba[at * row_bytes..]);
        Image::from_rgba(image.width, image.height + rows, out)
    }

    #[test]
    fn an_image_matches_itself() {
        let image = reference("html-designed-three-column");
        let comparison = compare(&image, &image);
        assert!(comparison.matches(), "{comparison:?}");
        assert_eq!(comparison.agreeing, comparison.cells);
        assert!(comparison.mismatched.is_empty());
    }

    #[test]
    fn a_lost_card_does_not_match_although_most_cells_agree() {
        let image = reference("html-designed-three-column");
        let mut lost = image.clone();
        // The middle card ("Trail runner"), painted over with the page's white.
        fill(&mut lost, 313, 340, 175, 132, [255, 255, 255, 255]);
        let comparison = compare(&image, &lost);
        let share = comparison.agreeing as f64 / comparison.cells as f64;
        assert!(
            share >= MIN_MATCHING,
            "the percentage alone would pass this: {share}"
        );
        assert!(comparison.lost_block, "{comparison:?}");
        assert!(!comparison.matches());
    }

    #[test]
    fn glyph_scale_noise_still_matches() {
        let image = reference("html-newsletter");
        let comparison = compare(&image, &blur(&image));
        assert!(comparison.matches(), "{comparison:?}");
    }

    #[test]
    fn an_extra_wrapped_line_costs_itself_not_everything_below_it() {
        let image = reference("html-newsletter");
        // Twenty rows of the page ground, a third of the way down.
        let ground = pixel(&image, 2, 2);
        let drifted = insert_band(&image, image.height / 3, 20, ground);
        let comparison = compare(&image, &drifted);
        assert!(comparison.matches(), "{comparison:?}");
    }

    #[test]
    fn ten_percent_taller_does_not_match() {
        let image = reference("html-newsletter");
        let ground = pixel(&image, 2, 2);
        let taller = insert_band(&image, image.height, image.height / 10, ground);
        let comparison = compare(&image, &taller);
        assert!(!comparison.height_ok);
        assert!(!comparison.matches());
    }

    #[test]
    fn the_diff_outlines_exactly_the_cells_that_changed() {
        let image = reference("html-transactional-receipt");
        let mut changed = image.clone();
        // Exactly cells (10..13, 4..6): 48x32 px, aligned to the grid.
        fill(
            &mut changed,
            10 * CELL,
            4 * CELL,
            3 * CELL,
            2 * CELL,
            [255, 0, 0, 255],
        );
        let comparison = compare(&image, &changed);
        let mut expected: Vec<(usize, usize)> =
            (10..13).flat_map(|c| (4..6).map(move |r| (c, r))).collect();
        expected.sort_unstable();
        let mut found = comparison.mismatched.clone();
        found.sort_unstable();
        assert_eq!(found, expected);
        let diff = diff_image(&image, &comparison);
        // A changed cell's corner is outlined; an unchanged cell's is not.
        assert_eq!(pixel(&diff, 10 * CELL, 4 * CELL), [255, 0, 255, 255]);
        assert_ne!(pixel(&diff, 2 * CELL, 2 * CELL), [255, 0, 255, 255]);
    }
}
