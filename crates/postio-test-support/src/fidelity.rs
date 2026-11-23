//! Does a render look like its reference? The rules are the fidelity contract's (spec 006), fixed
//! before any engine is judged and not loosened to make a fixture pass.
//! Both images are cut into [`CELL`]-pixel cells reduced to mean OKLab colour, so glyph shapes
//! average out and a missing block does not. Rows are aligned first ([`align_rows`]) so one extra
//! wrapped line does not shift every later cell, and a match also requires that no
//! [`BLOCK`]×[`BLOCK`] square of aligned cells is entirely wrong, so a lost card cannot pass on the
//! percentage.

use std::path::Path;

pub const CELL: usize = 16;
pub const MAX_DELTA_E: f64 = 0.08;
pub const MIN_MATCHING: f64 = 0.92;
pub const MAX_HEIGHT_DRIFT: f64 = 0.08;
pub const BLOCK: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub cells: usize,
    pub agreeing: usize,
    pub mismatched: Vec<(usize, usize)>,
    pub height_ok: bool,
    pub lost_block: bool,
}

impl Comparison {
    pub fn matches(&self) -> bool {
        self.height_ok
            && !self.lost_block
            && self.cells > 0
            && self.agreeing as f64 / self.cells as f64 >= MIN_MATCHING
    }
}

impl Image {
    pub fn from_rgba(width: usize, height: usize, rgba: Vec<u8>) -> Image {
        assert_eq!(rgba.len(), width * height * 4, "tightly packed RGBA");
        Image {
            width,
            height,
            rgba,
        }
    }

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

/// Banded global alignment as a text diff aligns lines. Returns for each reference row the
/// candidate row aligned with it, or `None` where the candidate lost it; content the candidate
/// added is answered by the height rule.
pub fn align_rows(reference: &[Vec<[f64; 3]>], candidate: &[Vec<[f64; 3]>]) -> Vec<Option<usize>> {
    let (n, m) = (reference.len(), candidate.len());
    let band = 32.max(n.max(m) / 10) + n.abs_diff(m);
    let gap = MAX_DELTA_E;
    let inf = f64::INFINITY;
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

/// Glyphs cover a minority of their box and a flat ground is one exact colour while antialiasing
/// spreads a glyph across many, so the most frequent exact colour is what is painted behind the
/// text. Not quantized: dark text on a dark ground lands in one bucket at any useful coarseness.
/// Clipped to the image; `None` if nothing is left.
pub fn ground_behind(
    image: &Image,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Option<[u8; 3]> {
    let (x1, y1) = ((x + width).min(image.width), (y + height).min(image.height));
    if x >= x1 || y >= y1 {
        return None;
    }
    let mut votes: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for row in y..y1 {
        for column in x..x1 {
            let at = (row * image.width + column) * 4;
            *votes
                .entry([image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]])
                .or_insert(0) += 1;
        }
    }
    votes
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(colour, _)| colour)
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
    fn the_ground_outvotes_the_glyphs_on_it() {
        let mut image = Image::from_rgba(40, 20, vec![0; 40 * 20 * 4]);
        fill(&mut image, 0, 0, 40, 20, [43, 43, 45, 255]);
        fill(&mut image, 5, 5, 10, 10, [34, 34, 34, 255]);
        fill(&mut image, 15, 5, 2, 10, [38, 38, 40, 255]);
        assert_eq!(ground_behind(&image, 0, 0, 40, 20), Some([43, 43, 45]));
        assert_eq!(
            ground_behind(&image, 50, 50, 5, 5),
            None,
            "clipped away entirely"
        );
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
        assert_eq!(pixel(&diff, 10 * CELL, 4 * CELL), [255, 0, 255, 255]);
        assert_ne!(pixel(&diff, 2 * CELL, 2 * CELL), [255, 0, 255, 255]);
    }
}
