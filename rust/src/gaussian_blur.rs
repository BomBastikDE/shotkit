//! Bounded Gaussian-like blur for straight-alpha BGRA images.
//!
//! Three separable box filters approximate a Gaussian in linear time. Sampling
//! clamps to the selected rectangle: no pixels outside it can enter the result.
//! The caller MUST supply a source with all document redactions already applied,
//! regardless of annotation order, and reapply them to the final document.

use crate::{
    pixelate::PixelateRect,
    stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION},
};

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_WORK_BYTES: usize = 256 * 1024 * 1024;
const MAX_RADIUS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlurError {
    InvalidImage,
    InvalidRegion,
    InvalidRadius,
    RegionTooLarge,
    AllocationFailed,
}

/// Returns only the blurred rectangle, in straight-alpha BGRA format.
/// `radius` is the radius of each of three box filters (1..=64). The source
/// and pixels outside the selection are untouched. Hidden colors under zero
/// alpha are discarded; interpolation is performed in premultiplied space.
pub fn gaussian_blur_region(
    source: &Image,
    region: PixelateRect,
    radius: usize,
) -> Result<Image, BlurError> {
    let bytes = source
        .width
        .checked_mul(source.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(BlurError::InvalidImage)?;
    if source.width == 0
        || source.height == 0
        || source.width > MAX_EDITABLE_IMAGE_DIMENSION
        || source.height > MAX_EDITABLE_IMAGE_DIMENSION
        || bytes > MAX_IMAGE_BYTES
        || bytes != source.pixels.len()
    {
        return Err(BlurError::InvalidImage);
    }
    if !(1..=MAX_RADIUS).contains(&radius) {
        return Err(BlurError::InvalidRadius);
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or(BlurError::InvalidRegion)?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or(BlurError::InvalidRegion)?;
    if region.width == 0 || region.height == 0 || right > source.width || bottom > source.height {
        return Err(BlurError::InvalidRegion);
    }
    let count = region
        .width
        .checked_mul(region.height)
        .ok_or(BlurError::RegionTooLarge)?;
    // Two [u32; 4] buffers and the resulting four-byte pixel buffer.
    if count
        .checked_mul(36)
        .filter(|&n| n <= MAX_WORK_BYTES)
        .is_none()
    {
        return Err(BlurError::RegionTooLarge);
    }
    let mut a = Vec::new();
    a.try_reserve_exact(count)
        .map_err(|_| BlurError::AllocationFailed)?;
    for y in region.y..bottom {
        for x in region.x..right {
            let i = (y * source.width + x) * 4;
            let p = &source.pixels[i..i + 4];
            let alpha = u32::from(p[3]);
            a.push([
                u32::from(p[0]) * alpha,
                u32::from(p[1]) * alpha,
                u32::from(p[2]) * alpha,
                alpha * 255,
            ]);
        }
    }
    let mut b = Vec::new();
    b.try_reserve_exact(count)
        .map_err(|_| BlurError::AllocationFailed)?;
    b.resize(count, [0u32; 4]);
    for _ in 0..3 {
        for y in 0..region.height {
            box_line(&a, &mut b, y * region.width, 1, region.width, radius);
        }
        for x in 0..region.width {
            box_line(&b, &mut a, x, region.width, region.height, radius);
        }
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count * 4)
        .map_err(|_| BlurError::AllocationFailed)?;
    for p in a {
        let alpha = (p[3] + 127) / 255;
        if alpha == 0 || p[3] == 0 {
            pixels.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for &channel in &p[..3] {
            pixels.push(
                ((u64::from(channel) * 255 + u64::from(p[3]) / 2) / u64::from(p[3])).min(255) as u8,
            );
        }
        pixels.push(alpha.min(255) as u8);
    }
    Image::new(region.width, region.height, pixels).map_err(|_| BlurError::InvalidRegion)
}

// Sliding window with repeated boundary pixels, so a uniform region remains
// uniform even when the radius exceeds the dimensions of the selection.
fn box_line(
    input: &[[u32; 4]],
    output: &mut [[u32; 4]],
    start: usize,
    stride: usize,
    length: usize,
    radius: usize,
) {
    let divisor = (radius * 2 + 1) as u64;
    let mut sums = [0u64; 4];
    for offset in 0..=radius * 2 {
        let index = offset.saturating_sub(radius).min(length - 1);
        for (sum, value) in sums.iter_mut().zip(input[start + index * stride]) {
            *sum += u64::from(value);
        }
    }
    for pos in 0..length {
        let out = &mut output[start + pos * stride];
        for c in 0..4 {
            out[c] = ((sums[c] + divisor / 2) / divisor) as u32;
        }
        if pos + 1 < length {
            let remove = pos.saturating_sub(radius);
            let add = (pos + radius + 1).min(length - 1);
            for c in 0..4 {
                sums[c] = sums[c] - u64::from(input[start + remove * stride][c])
                    + u64::from(input[start + add * stride][c]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: usize, y: usize, width: usize, height: usize) -> PixelateRect {
        PixelateRect {
            x,
            y,
            width,
            height,
        }
    }
    fn image(width: usize, height: usize, pixels: Vec<u8>) -> Image {
        Image::new(width, height, pixels).unwrap()
    }

    #[test]
    fn impulse_spreads_smoothly_in_both_axes() {
        let mut pixels = vec![0; 9 * 9 * 4];
        for p in pixels.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        pixels[(4 * 9 + 4) * 4] = 255;
        let source = image(9, 9, pixels);
        let result = gaussian_blur_region(&source, rect(0, 0, 9, 9), 1).unwrap();
        let at = |x: usize, y: usize| result.pixels[(y * 9 + x) * 4];
        assert!(at(4, 4) > at(5, 4) && at(5, 4) > at(6, 4) && at(6, 4) > 0);
        assert_eq!(at(5, 4), at(4, 5));
        assert_eq!(source.pixels[(4 * 9 + 4) * 4], 255);
    }

    #[test]
    fn transparent_colors_do_not_bleed() {
        let source = image(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 0]);
        let out = gaussian_blur_region(&source, rect(0, 0, 2, 1), 1).unwrap();
        for p in out.pixels.as_chunks::<4>().0 {
            assert_eq!(&p[..3], &[0, 0, 255]);
            assert!(p[3] > 0 && p[3] < 255);
        }
        let clear = image(1, 1, vec![123, 45, 67, 0]);
        assert_eq!(
            gaussian_blur_region(&clear, rect(0, 0, 1, 1), 64)
                .unwrap()
                .pixels,
            vec![0; 4]
        );
    }

    #[test]
    fn selection_clamps_sampling_and_preserves_uniform_edges() {
        let source = image(
            3,
            1,
            vec![255, 255, 255, 255, 12, 34, 56, 127, 255, 255, 255, 255],
        );
        assert_eq!(
            gaussian_blur_region(&source, rect(1, 0, 1, 1), 64)
                .unwrap()
                .pixels,
            vec![12, 34, 56, 127]
        );
    }

    #[test]
    fn validates_radius_region_and_image() {
        let source = image(1, 1, vec![0; 4]);
        assert_eq!(
            gaussian_blur_region(&source, rect(0, 0, 1, 1), 0),
            Err(BlurError::InvalidRadius)
        );
        assert_eq!(
            gaussian_blur_region(&source, rect(usize::MAX, 0, 2, 1), 1),
            Err(BlurError::InvalidRegion)
        );
        assert_eq!(
            gaussian_blur_region(&source, rect(0, 0, 0, 1), 1),
            Err(BlurError::InvalidRegion)
        );
        let malformed = Image {
            width: 2,
            height: 1,
            pixels: vec![0; 4],
        };
        assert_eq!(
            gaussian_blur_region(&malformed, rect(0, 0, 1, 1), 1),
            Err(BlurError::InvalidImage)
        );
    }
}
