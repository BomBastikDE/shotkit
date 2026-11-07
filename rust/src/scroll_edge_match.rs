//! Conservative exact translation matching, including the outermost pixels.
//!
//! A match proves a unique image translation only within the supported search
//! range (at least 24 overlap rows). It does not identify document ownership:
//! an independently animated overlay can produce the same pixels as content.
//! No scrollbar is cropped or removed, and sticky/changed pixels cause abstention.
use crate::scroll_match::MatchResult;
use crate::stitch::Image;

const MIN_OVERLAP: usize = 24;
const MIN_TRANSITIONS: usize = 6;

/// Find a unique positive, exact full-width translation. The caller must keep
/// an uncertain result out of estimated-scroll fallbacks and handle stationary
/// frames separately. There is deliberately no expected-offset tie breaker.
pub fn find_edge_offset(before: &Image, after: &Image) -> Option<MatchResult> {
    before.validate().ok()?;
    after.validate().ok()?;
    if before.width != after.width || before.height != after.height {
        return None;
    }
    let height = before.height;
    if height <= MIN_OVERLAP {
        return None;
    }
    let stride = before.width * 4;
    let hashes = |image: &Image| {
        image
            .pixels
            .chunks_exact(stride)
            .map(|row| {
                row.iter().fold(14695981039346656037u64, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
                })
            })
            .collect::<Vec<_>>()
    };
    let old_rows = hashes(before);
    let new_rows = hashes(after);

    // KMP finds every suffix of the old rows equal to a prefix of the new
    // rows in linear row work. Hashes only reject candidates; bytes certify
    // every accepted candidate, so a hash collision cannot accept a seam.
    let mut prefix = vec![0; height];
    for i in 1..height {
        let mut matched = prefix[i - 1];
        while matched > 0 && new_rows[i] != new_rows[matched] {
            matched = prefix[matched - 1];
        }
        if new_rows[i] == new_rows[matched] {
            matched += 1;
        }
        prefix[i] = matched;
    }
    let mut overlap = 0;
    for row in old_rows {
        while overlap > 0 && (overlap == height || row != new_rows[overlap]) {
            overlap = prefix[overlap - 1];
        }
        if row == new_rows[overlap] {
            overlap += 1;
        }
    }
    if overlap == height {
        overlap = prefix[overlap - 1];
    }
    let mut winner = None;
    while overlap >= MIN_OVERLAP {
        let dy = height - overlap;
        if before.pixels[dy * stride..] == after.pixels[..overlap * stride] {
            // Count every exact candidate before filtering its structure. A
            // weak alternative is still ambiguity, never a tie to discard.
            if winner.is_some() {
                return None;
            }
            winner = Some((dy, overlap));
        }
        overlap = prefix[overlap - 1];
    }
    let (dy, overlap) = winner?;
    // A very short accidental suffix can be unique yet insufficient evidence
    // for appending almost a whole viewport. Retry with a smaller scroll step.
    if overlap < height.div_ceil(2) {
        return None;
    }
    // A flat background or a simple translating scrollbar thumb supplies too
    // little evidence. Binary irregular artwork is supported: variation need
    // not consist of many different colors or horizontal luminance bins.
    let transitions = |start: usize, end: usize| {
        (start + 1..end)
            .filter(|&row| new_rows[row - 1] != new_rows[row])
            .count()
    };
    let middle = overlap / 2;
    if transitions(0, overlap) < MIN_TRANSITIONS
        || transitions(0, middle) < 2
        || transitions(middle, overlap) < 2
    {
        return None;
    }
    Some(MatchResult {
        dy,
        top: 0,
        bottom: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 120;
    const HEIGHT: usize = 220;

    fn image(shift: usize, edge: usize, pattern: impl Fn(usize) -> u8) -> Image {
        let mut pixels = vec![255; WIDTH * HEIGHT * 4];
        for y in 0..HEIGHT {
            for x in edge..edge + 8 {
                let p = (y * WIDTH + x) * 4;
                pixels[p..p + 3].fill(pattern(y + shift));
            }
        }
        Image::new(WIDTH, HEIGHT, pixels).unwrap()
    }

    fn irregular(row: usize) -> u8 {
        if (row.wrapping_mul(73) ^ (row >> 2)) % 7 < 3 {
            0
        } else {
            255
        }
    }

    #[test]
    fn finds_exact_content_in_either_outermost_margin() {
        for edge in [0, WIDTH - 8] {
            for dy in [1, 30, 50, 100] {
                let a = image(0, edge, irregular);
                let b = image(dy, edge, irregular);
                assert_eq!(
                    find_edge_offset(&a, &b),
                    Some(MatchResult {
                        dy,
                        top: 0,
                        bottom: 0
                    }),
                    "edge={edge}, dy={dy}"
                );
            }
        }
    }

    #[test]
    fn repeated_structure_is_ambiguous_even_with_strong_variation() {
        for edge in [0, WIDTH - 8] {
            let pattern = |row: usize| if row % 7 < 3 { 0 } else { 255 };
            assert_eq!(
                find_edge_offset(&image(0, edge, pattern), &image(30, edge, pattern)),
                None
            );
        }
    }

    #[test]
    fn changing_edge_animation_cannot_certify_a_seam() {
        let a = image(0, 0, irregular);
        let mut b = image(50, 0, irregular);
        for y in 0..HEIGHT {
            b.pixels[(y * WIDTH + WIDTH - 1) * 4] = 127;
        }
        assert_eq!(find_edge_offset(&a, &b), None);
    }

    #[test]
    fn moving_simple_scrollbar_and_flat_frames_abstain() {
        let thumb = |row: usize| if (100..140).contains(&row) { 0 } else { 255 };
        for edge in [0, WIDTH - 8] {
            assert_eq!(
                find_edge_offset(&image(0, edge, thumb), &image(30, edge, thumb)),
                None
            );
        }
        let flat = image(0, 0, |_| 255);
        assert_eq!(find_edge_offset(&flat, &flat), None);
    }

    #[test]
    fn rejects_wrong_direction_and_changed_interior() {
        let a = image(0, 0, irregular);
        let b = image(30, 0, irregular);
        assert_eq!(find_edge_offset(&b, &a), None);
        let mut changed = b;
        for y in 0..HEIGHT {
            changed.pixels[(y * WIDTH + WIDTH / 2) * 4] = 0;
        }
        assert_eq!(find_edge_offset(&a, &changed), None);
    }

    #[test]
    fn invalid_dimensions_and_pixels_abstain() {
        let valid = image(0, 0, irregular);
        let invalid = Image {
            width: WIDTH,
            height: HEIGHT,
            pixels: vec![0],
        };
        assert_eq!(find_edge_offset(&invalid, &valid), None);
        assert_eq!(find_edge_offset(&valid, &invalid), None);
        let different = Image::new(1, HEIGHT, vec![255; HEIGHT * 4]).unwrap();
        assert_eq!(find_edge_offset(&valid, &different), None);
        let short = Image::new(1, MIN_OVERLAP, vec![255; MIN_OVERLAP * 4]).unwrap();
        assert_eq!(find_edge_offset(&short, &short), None);
    }
}
