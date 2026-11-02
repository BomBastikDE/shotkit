//! Pure image analysis and scroll-offset matching ported from ShotKit's
//! `ScrollCapture.cs`. Pixels are tightly packed, top-down row-major BGRA. The module deliberately
//! excludes screen capture, scrolling, stitching, and UI concerns.
//!
//! [`Frame::analyze`] builds an exact per-row hash and a coarse horizontal
//! luminance profile. [`find_offset`] returns the downward displacement of
//! content between equal-sized frames, plus stationary top/bottom margins.
//! `None` means no unambiguous evidence; `Some(dy=0)` means no active columns
//! were found (the C# caller treats this as a stationary/end condition).
//!
//! Limitations inherited from the source algorithm: grayscale bins can confuse
//! repeated patterns, matching expects equal frame dimensions, and very flat
//! or animated regions may abstain. Hashes are exact and are only suitable for
//! quick same-position / settled checks, not approximate equality.

const BAND: usize = 40;
const MIN_OVERLAP: usize = 24;
const MIN_BAND_ROWS: usize = 6;
const BIN_TOLERANCE: u8 = 10;
const ROW_SHARE: f32 = 0.75;
const MIN_ROW_CONTRAST: u8 = 6;
const STATIC_TOLERANCE: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Number of coarse horizontal profile bins per row.
    pub bins: usize,
    /// Exact hash for each row.
    pub row_hashes: Vec<u64>,
    /// Row-major coarse luminance values (`height * bins`).
    pub signature: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchResult {
    /// Downward content displacement, in pixels; zero represents stationary.
    pub dy: usize,
    /// Stationary top and bottom edge extents.
    pub top: usize,
    pub bottom: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalyzeError {
    EmptyImage,
    InvalidPixelLength,
}

impl Frame {
    /// Analyze tightly packed, top-down row-major BGRA data. Alpha is included in the
    /// exact hash and ignored in luminance, matching the C# ARGB behavior.
    pub fn analyze(width: usize, height: usize, bgra: &[u8]) -> Result<Self, AnalyzeError> {
        if width == 0 || height == 0 {
            return Err(AnalyzeError::EmptyImage);
        }
        if width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(bgra.len()) {
            return Err(AnalyzeError::InvalidPixelLength);
        }
        let margin = if width > 60 { 24.min(width / 10) } else { 0 };
        let cols = width - 2 * margin;
        // Preserve the source's 8..64 target except for exceptionally narrow
        // inputs, where fewer bins avoid empty-bin division by zero.
        let bins = (cols / 12).clamp(8, 64).min(cols);
        let mut row_hashes = Vec::with_capacity(height);
        let mut signature = vec![0; height * bins];
        let mut counts = vec![0usize; bins];
        for x in 0..cols {
            counts[x * bins / cols] += 1;
        }
        for y in 0..height {
            let mut hash = 14695981039346656037u64;
            let mut sums = vec![0u32; bins];
            for x in 0..cols {
                let p = ((y * width) + x + margin) * 4;
                // Construct the packed ARGB integer represented by BGRA bytes.
                let packed = ((bgra[p + 3] as u32) << 24)
                    | ((bgra[p + 2] as u32) << 16)
                    | ((bgra[p + 1] as u32) << 8)
                    | bgra[p] as u32;
                hash = (hash ^ packed as u64).wrapping_mul(1099511628211);
                let lum =
                    (bgra[p + 2] as u32 * 77 + bgra[p + 1] as u32 * 150 + bgra[p] as u32 * 29) >> 8;
                sums[x * bins / cols] += lum;
            }
            row_hashes.push(hash);
            for b in 0..bins {
                signature[y * bins + b] = (sums[b] / counts[b] as u32) as u8;
            }
        }
        Ok(Self {
            width,
            height,
            bins,
            row_hashes,
            signature,
        })
    }

    /// Exact whole-frame equality, corresponding to C# `Same`.
    pub fn same(&self, other: &Self) -> bool {
        self.row_hashes == other.row_hashes
    }

    /// Whether at least 90% of this frame's rows are pixel-identical.
    pub fn settled_with(&self, other: &Self) -> bool {
        self.height == other.height
            && self
                .row_hashes
                .iter()
                .zip(&other.row_hashes)
                .filter(|(a, b)| a == b)
                .count() as f32
                >= self.height as f32 * 0.9
    }
}

fn diff(a: u8, b: u8) -> u8 {
    a.abs_diff(b)
}

/// Infer content displacement in equal-sized frames. `expected` breaks ties
/// between nearby equally supported offsets, as in the source implementation.
pub fn find_offset(a: &Frame, b: &Frame, expected: usize) -> Option<MatchResult> {
    if a.height != b.height
        || a.width != b.width
        || a.bins != b.bins
        || a.signature.len() != a.height * a.bins
        || b.signature.len() != b.height * b.bins
    {
        return None;
    }
    let h = a.height;
    let active: Vec<usize> = (0..a.bins)
        .filter(|&c| {
            let eq = (0..h)
                .filter(|&y| {
                    diff(a.signature[y * a.bins + c], b.signature[y * b.bins + c])
                        <= STATIC_TOLERANCE
                })
                .count();
            (eq as f32) < h as f32 * 0.97
        })
        .collect();
    if active.len() < 4 {
        return Some(MatchResult {
            dy: 0,
            top: 0,
            bottom: 0,
        });
    }
    let structured = |f: &Frame, row: usize| -> bool {
        let (mut lo, mut hi) = (u8::MAX, 0);
        for &c in &active {
            let v = f.signature[row * f.bins + c];
            lo = lo.min(v);
            hi = hi.max(v);
        }
        hi - lo >= MIN_ROW_CONTRAST
    };
    let static_row = |row: usize| {
        active.iter().all(|&c| {
            diff(a.signature[row * a.bins + c], b.signature[row * b.bins + c]) <= STATIC_TOLERANCE
        })
    };
    let mut top = 0;
    for row in 0..h / 3 {
        if structured(b, row) {
            if static_row(row) {
                top = row + 1;
            } else {
                break;
            }
        }
    }
    let mut bottom = 0;
    for i in 0..h / 3 {
        let row = h - 1 - i;
        if structured(b, row) {
            if static_row(row) {
                bottom = i + 1;
            } else {
                break;
            }
        }
    }
    let max_dy = h.saturating_sub(top + bottom + MIN_OVERLAP);
    let mut votes = vec![0usize; max_dy + 1];
    let mut band_offsets = Vec::new();
    let row_match = |ia: usize, ib: usize| -> Option<f32> {
        let allowed_miss = active.len() - (active.len() as f32 * ROW_SHARE).ceil() as usize;
        let (mut ok, mut sum, mut miss) = (0usize, 0u32, 0usize);
        for &c in &active {
            let d = diff(a.signature[ia * a.bins + c], b.signature[ib * b.bins + c]);
            if d <= BIN_TOLERANCE {
                ok += 1;
                sum += d as u32;
            } else {
                miss += 1;
                if miss > allowed_miss {
                    return None;
                }
            }
        }
        Some(if ok > 0 { sum as f32 / ok as f32 } else { 0.0 })
    };
    let mut b0 = top;
    while b0 < h - bottom {
        let end = (b0 + BAND).min(h - bottom);
        let rows: Vec<usize> = (b0..end).filter(|&i| structured(b, i)).collect();
        if rows.len() >= MIN_BAND_ROWS {
            let last = max_dy.min(h - 1 - rows[rows.len() - 1]);
            let mut cv = vec![0usize; last + 1];
            let mut ce = vec![f32::MAX; last + 1];
            let mut vmax = 0;
            for dy in 0..=last {
                let mut err = 0.0;
                for &i in &rows {
                    if let Some(e) = row_match(i + dy, i) {
                        cv[dy] += 1;
                        err += e;
                    }
                }
                if cv[dy] > 0 {
                    ce[dy] = err / cv[dy] as f32;
                }
                vmax = vmax.max(cv[dy]);
            }
            if (vmax as f32) >= rows.len() as f32 * 0.6 {
                let mut best = None;
                for dy in 0..=last {
                    if (cv[dy] as f32) >= vmax as f32 * 0.9
                        && best.map_or(true, |bd| ce[dy] < ce[bd])
                    {
                        best = Some(dy);
                    }
                }
                if let Some(bd) = best {
                    let ambiguous = (0..=last).any(|dy| {
                        dy.abs_diff(bd) > 2
                            && (cv[dy] as f32) >= vmax as f32 * 0.9
                            && ce[dy] <= ce[bd] + 1.0
                    });
                    if !ambiguous {
                        votes[bd] += cv[bd];
                        band_offsets.push(bd);
                    }
                }
            }
        }
        b0 += BAND;
    }
    if band_offsets.is_empty() {
        return None;
    }
    let winner = *band_offsets.iter().max_by(|&&x, &&y| {
        let score = |d: usize| {
            votes.get(d.wrapping_sub(1)).copied().unwrap_or(0)
                + votes[d]
                + votes.get(d + 1).copied().unwrap_or(0)
        };
        score(x)
            .cmp(&score(y))
            .then_with(|| votes[x].cmp(&votes[y]))
            .then_with(|| expected.abs_diff(y).cmp(&expected.abs_diff(x)))
    })?;
    Some(MatchResult {
        dy: winner,
        top,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Deterministic, non-repeating rows with distinct horizontal profiles.
    fn image(height: usize, shift: usize) -> Vec<u8> {
        let w = 120;
        let mut p = vec![0; w * height * 4];
        for y in 0..height {
            let src = (y + shift) % height;
            for x in 0..w {
                let v = ((src * 73 + x * 41 + src * x * 19 + (src * src) % 251) % 256) as u8;
                let i = (y * w + x) * 4;
                p[i] = v;
                p[i + 1] = v;
                p[i + 2] = v;
                p[i + 3] = 255;
            }
        }
        p
    }
    #[test]
    fn analysis_and_exact_settled_checks() {
        let a = Frame::analyze(120, 100, &image(100, 0)).unwrap();
        let b = Frame::analyze(120, 100, &image(100, 0)).unwrap();
        assert!(a.same(&b));
        assert!(a.settled_with(&b));
        assert_eq!(
            Frame::analyze(2, 2, &[]),
            Err(AnalyzeError::InvalidPixelLength)
        );
    }
    #[test]
    fn finds_synthetic_offset() {
        let a = Frame::analyze(120, 160, &image(160, 0)).unwrap();
        let b = Frame::analyze(120, 160, &image(160, 5)).unwrap();
        assert_eq!(find_offset(&a, &b, 5).unwrap().dy, 5);
    }
    #[test]
    fn dimension_mismatch_is_rejected() {
        let a = Frame::analyze(120, 80, &image(80, 0)).unwrap();
        let b = Frame::analyze(120, 81, &image(81, 0)).unwrap();
        assert_eq!(find_offset(&a, &b, 1), None);
    }
    #[test]
    fn very_narrow_frames_do_not_have_empty_bins() {
        for width in 1..8 {
            let bytes = vec![127; width * 3 * 4];
            let f = Frame::analyze(width, 3, &bytes).unwrap();
            assert!(f.bins >= 1 && f.bins <= width);
            assert_eq!(f.signature.len(), 3 * f.bins);
        }
    }
}
