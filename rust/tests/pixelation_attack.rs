//! Reproducible, synthetic candidate-matching benchmark for issue #5.
//!
//! This measures one known-font, known-position, six-digit attack. It is not a
//! security guarantee or a confidence bound on recovery of arbitrary text.
//! The attacker additionally knows the actual grid phase, but not the seed or
//! block noise. Fixed fixtures/seeds are specified below, never searched for a
//! passing result. Observations come from the real production byte renderer.
//! The likelihood attack knows the four output levels, uniform integer noise
//! in [-96,96], the shared noise across RGB, and the exactly-black exception.
//! It models independent block noise, not recovery of the deterministic PRNG
//! state or correlations between repeated exports or overlapping shapes.

use shotkit::{
    pixelate::{pixelate_phase, pixelate_region_seeded, PixelateRect},
    stitch::Image,
};

const CANDIDATES: usize = 400;
const TRIALS: usize = 4_000;
const WIDTHS: [usize; 8] = [1, 2, 3, 4, 6, 8, 12, 16];
// Original 5x7 pixel glyphs, nearest-neighbor scaled to 12/16 pixel heights.
// No font download, host font selection, antialiasing or external RNG is used.
const DIGITS: [[u8; 7]; 10] = [
    [31, 17, 17, 17, 17, 17, 31],
    [4, 12, 4, 4, 4, 4, 14],
    [30, 1, 1, 14, 16, 16, 31],
    [30, 1, 1, 14, 1, 1, 30],
    [18, 18, 18, 31, 2, 2, 2],
    [31, 16, 16, 30, 1, 1, 30],
    [15, 16, 16, 30, 17, 17, 14],
    [31, 1, 2, 4, 8, 8, 8],
    [14, 17, 17, 14, 17, 17, 14],
    [14, 17, 17, 15, 1, 1, 30],
];

fn render(number: usize, height: usize) -> Image {
    let width = height * 6;
    let canvas_height = height * 8 / 5;
    let mut pixels = vec![255; width * canvas_height * 4];
    for (index, digit) in format!("{number:06}").bytes().enumerate() {
        for y in 0..height {
            for x in 0..height * 5 / 7 {
                if DIGITS[usize::from(digit - b'0')][y * 7 / height] & (1 << (4 - x * 7 / height))
                    != 0
                {
                    let offset = ((3 + y) * width + 4 + index * height * 6 / 7 + x) * 4;
                    pixels[offset..offset + 3].fill(0);
                }
            }
        }
    }
    Image::new(width, canvas_height, pixels).unwrap()
}

struct Integral {
    stride: usize,
    sums: Vec<u64>,
}

impl Integral {
    fn new(image: &Image) -> Self {
        let stride = image.width + 1;
        let mut sums = vec![0; stride * (image.height + 1)];
        for y in 0..image.height {
            let mut row = 0;
            for x in 0..image.width {
                row += u64::from(image.pixels[(y * image.width + x) * 4]);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row;
            }
        }
        Self { stride, sums }
    }

    fn sum(&self, cell: Cell) -> u64 {
        let at = |x, y| self.sums[y * self.stride + x];
        at(cell.right, cell.bottom) + at(cell.x, cell.y)
            - at(cell.x, cell.bottom)
            - at(cell.right, cell.y)
    }

    fn mean(&self, cell: Cell) -> u8 {
        let sum = self.sum(cell);
        // Historical pixelate.rs used div_round(sum * 255, count * 255)
        // for an opaque grayscale image, exactly equivalent to this rounding.
        ((sum + cell.area() / 2) / cell.area()) as u8
    }
}

#[derive(Clone, Copy)]
struct Cell {
    x: usize,
    y: usize,
    right: usize,
    bottom: usize,
}

impl Cell {
    fn area(self) -> u64 {
        ((self.right - self.x) * (self.bottom - self.y)) as u64
    }
}

fn edges(length: usize, block: usize, phase: usize) -> Vec<usize> {
    let mut result = vec![0];
    let mut edge = block - phase;
    while edge < length {
        result.push(edge);
        edge += block;
    }
    result.push(length);
    result
}

fn cells(width: usize, height: usize, block: usize, phase: (usize, usize)) -> Vec<Cell> {
    let xs = edges(width, block, phase.0);
    let ys = edges(height, block, phase.1);
    ys.windows(2)
        .flat_map(|y| {
            xs.windows(2).map(move |x| Cell {
                x: x[0],
                y: y[0],
                right: x[1],
                bottom: y[1],
            })
        })
        .collect()
}

fn legacy_bytes(source: &Image, block: usize) -> Image {
    // Exact historical behavior for these opaque grayscale fixtures: grid
    // anchored at (0,0), clipped final cells, rounded u8 average, alpha 255.
    legacy_bytes_with_phase(source, block, (0, 0))
}

fn legacy_bytes_with_phase(source: &Image, block: usize, phase: (usize, usize)) -> Image {
    // Controlled ablation: historical rounded byte averages with an explicit
    // grid/phase. For each actual hardened grid this isolates the value of
    // noise/quantization rather than crediting larger blocks.
    let integral = Integral::new(source);
    let mut result = source.clone();
    for cell in cells(source.width, source.height, block, phase) {
        let value = integral.mean(cell);
        for y in cell.y..cell.bottom {
            for x in cell.x..cell.right {
                let offset = (y * source.width + x) * 4;
                result.pixels[offset..offset + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
    }
    result
}

fn likelihood_table() -> [[f64; 4]; 256] {
    let mut result = [[f64::NEG_INFINITY; 4]; 256];
    for (mean, row) in result.iter_mut().enumerate() {
        let mut counts = [0u32; 4];
        // Exact finite distribution after clipping and integer quantization.
        // The production modulo introduces a negligible (<193/2^64) bias;
        // this attack assumes the requested uniform integer noise model.
        for noise in -96..=96 {
            let level = (mean as i32 + noise).clamp(0, 255) as usize / 64;
            counts[level] += 1;
        }
        for (log_probability, count) in row.iter_mut().zip(counts) {
            if count != 0 {
                *log_probability = (f64::from(count) / 193.0).ln();
            }
        }
    }
    result
}

fn guess_likelihood(
    observation: &Image,
    candidates: &[Integral],
    grid: &[Cell],
    table: &[[f64; 4]; 256],
) -> usize {
    let levels: Vec<_> = grid
        .iter()
        .map(|cell| {
            let offset = (cell.y * observation.width + cell.x) * 4;
            let color = &observation.pixels[offset..offset + 4];
            // Grayscale source plus one shared noise value gives equal RGB.
            assert_eq!(color[0], color[1]);
            assert_eq!(color[1], color[2]);
            assert_eq!(color[3], 255);
            match color[0] {
                0 => None, // Only a truly black source block bypasses noise.
                32 | 96 | 160 | 224 => Some(usize::from(color[0] / 64)),
                value => panic!("unexpected hardened output level {value}"),
            }
        })
        .collect();
    let mut best = 0;
    let mut best_score = f64::NEG_INFINITY;
    for (index, candidate) in candidates.iter().enumerate() {
        let mut score = 0.0;
        for (&cell, &level) in grid.iter().zip(&levels) {
            let sum = candidate.sum(cell);
            match (sum == 0, level) {
                (true, None) => {}
                (false, Some(level)) => {
                    // A rounded mean of zero is not the exactly-black exception.
                    let mean = ((sum + cell.area() / 2) / cell.area()) as usize;
                    score += table[mean][level];
                }
                _ => {
                    score = f64::NEG_INFINITY;
                    break;
                }
            }
        }
        // Each block contributes once: the pixels and RGB channels within a
        // block share the same noise and are not independent observations.
        if score > best_score {
            best = index;
            best_score = score;
        }
    }
    assert!(
        best_score.is_finite(),
        "all candidates have zero likelihood"
    );
    best
}

fn guess(observation: &Image, candidates: &[Integral], grid: &[Cell]) -> usize {
    // Full-image squared byte error, compressed without loss because both
    // images are constant within each known grid cell. RGB noise channels are
    // evaluated separately; area weighting includes clipped boundary cells.
    let observed: Vec<_> = grid
        .iter()
        .map(|cell| {
            let offset = (cell.y * observation.width + cell.x) * 4;
            let value = &observation.pixels[offset..offset + 3];
            [value[0], value[1], value[2]]
        })
        .collect();
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let distance = grid
                .iter()
                .zip(&observed)
                .map(|(&cell, color)| {
                    let expected = i64::from(candidate.mean(cell));
                    color
                        .iter()
                        .map(|&channel| (i64::from(channel) - expected).unsigned_abs().pow(2))
                        .sum::<u64>()
                        * cell.area()
                })
                .sum::<u64>();
            (distance, index)
        })
        // Explicit deterministic first-index tie break, independent of target.
        .min()
        .unwrap()
        .1
}

fn fixture_seed(trial: usize, size: usize) -> u64 {
    // SplitMix64 finalizer is only a reproducible test-fixture generator. These
    // public constants derive from hexadecimal digits of pi; no seed fitting.
    let mut value = 0x243f_6a88_85a3_08d3u64
        .wrapping_add((trial as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(size as u64);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[test]
fn synthetic_digit_candidate_attack_and_historical_counterexample() {
    let mut failures = Vec::new();
    let likelihood = likelihood_table();
    for height in [12, 16] {
        // Coprime multiplier modulo 1,000,000 gives 400 unique numbers, including
        // leading-zero strings. Candidate order is fixed before any observations.
        let candidates: Vec<_> = (0..CANDIDATES)
            .map(|index| render((index * 7_919 + 123_457) % 1_000_000, height))
            .collect();
        let integrals: Vec<_> = candidates.iter().map(Integral::new).collect();
        let first = &candidates[0];
        let region = PixelateRect {
            x: 0,
            y: 0,
            width: first.width,
            height: first.height,
        };
        let mut exact_24_hits = 0;
        for trial in 0..TRIALS {
            let target = (trial * 137 + 83) % CANDIDATES;
            let phase = pixelate_phase(24, fixture_seed(trial, height));
            let grid = cells(first.width, first.height, 24, phase);
            let observed = legacy_bytes_with_phase(&candidates[target], 24, phase);
            exact_24_hits += usize::from(guess(&observed, &integrals, &grid) == target);
        }
        eprintln!(
            "height={height}px historical counterexample: exact-byte-mean block24, known phase={exact_24_hits}/{TRIALS}; chance=1/{CANDIDATES}"
        );
        // Require the stronger original >90% counterexample criterion at block
        // 24 too; do not accept an uninformative near-random ablation.
        if exact_24_hits * 10 <= TRIALS * 9 {
            failures.push(format!(
                "same-grid block24 counterexample <=90%: {height}px: {exact_24_hits}/{TRIALS}"
            ));
        }
        for width in WIDTHS {
            let legacy_block = (width * 3).clamp(12, 4_096);
            let hardened_block = (width * 6).clamp(48, 4_096);
            let legacy_grid = cells(first.width, first.height, legacy_block, (0, 0));
            // All 400 targets occur equally often. This also exposes legacy
            // collisions at large block sizes instead of concealing them.
            let legacy_guesses: Vec<_> = candidates
                .iter()
                .map(|image| guess(&legacy_bytes(image, legacy_block), &integrals, &legacy_grid))
                .collect();
            let mut legacy_hits = 0;
            let mut hardened_hits = 0;
            let mut likelihood_hits = 0;
            let mut exact_same_grid_hits = 0;
            for trial in 0..TRIALS {
                let target = (trial * 137 + 83) % CANDIDATES;
                legacy_hits += usize::from(legacy_guesses[target] == target);
                let seed = fixture_seed(trial, height);
                let phase = pixelate_phase(hardened_block, seed);
                let grid = cells(first.width, first.height, hardened_block, phase);
                let exact = legacy_bytes_with_phase(&candidates[target], hardened_block, phase);
                exact_same_grid_hits += usize::from(guess(&exact, &integrals, &grid) == target);
                let observed =
                    pixelate_region_seeded(&candidates[target], region, hardened_block, seed)
                        .unwrap();
                hardened_hits += usize::from(guess(&observed, &integrals, &grid) == target);
                likelihood_hits += usize::from(
                    guess_likelihood(&observed, &integrals, &grid, &likelihood) == target,
                );
            }
            eprintln!(
                "height={height}px width={width}: legacy={legacy_hits}/{TRIALS} (block {legacy_block}), exact-same-grid={exact_same_grid_hits}/{TRIALS}, hardened-MSE={hardened_hits}/{TRIALS}, hardened-ML={likelihood_hits}/{TRIALS} (block {hardened_block}); chance=1/{CANDIDATES}"
            );
            // The old minimum is the demonstrated vulnerable counterexample.
            // Larger legacy grids are reported, not falsely asserted >90%:
            // small text produces indistinguishable byte averages there.
            if legacy_block == 12 && legacy_hits * 10 <= TRIALS * 9 {
                failures.push(format!(
                    "legacy counterexample failed: {height}px width {width}"
                ));
            }
            // Same-size, same-phase exact averages must discriminate beyond
            // the hardened acceptance threshold. Report their full rate;
            // this does not assert >90% recovery at arbitrarily large blocks.
            if exact_same_grid_hits * CANDIDATES <= 2 * TRIALS {
                failures.push(format!(
                    "same-grid exact mean fails to discriminate beyond twice chance: {height}px width {width}: {exact_same_grid_hits}/{TRIALS}"
                ));
            }
            if hardened_hits * CANDIDATES > 2 * TRIALS {
                failures.push(format!(
                    "hardened exceeds twice chance: {height}px width {width}: {hardened_hits}/{TRIALS}"
                ));
            }
            if likelihood_hits * CANDIDATES > 2 * TRIALS {
                failures.push(format!(
                    "maximum-likelihood exceeds twice chance: {height}px width {width}: {likelihood_hits}/{TRIALS}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
