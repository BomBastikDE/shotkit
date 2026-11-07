//! Alpha-correct pixelation of a bounded rectangle in a BGRA image.
//!
//! The source remains unchanged. Block averages can be written into an existing
//! target buffer or returned in an independent full-size copy of the image.

use crate::stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION};

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_BLOCK_SIZE: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelateRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelateError {
    InvalidImage,
    InvalidRegion,
    InvalidBlockSize,
    AllocationFailed,
}

/// Pixelate `region` in `source`, returning a new full-size BGRA image.
///
/// Each block's alpha is averaged, and its color is averaged in premultiplied
/// space before conversion back to straight alpha. Fully transparent pixels
/// therefore do not introduce hidden RGB color into partially transparent
/// block results.
pub fn pixelate_region(
    source: &Image,
    region: PixelateRect,
    block_size: usize,
) -> Result<Image, PixelateError> {
    let (right, bottom) = validate_request(source, region, block_size)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(source.pixels.len())
        .map_err(|_| PixelateError::AllocationFailed)?;
    pixels.extend_from_slice(&source.pixels);
    pixelate_validated(source, &mut pixels, region, block_size, right, bottom);

    Image::new(source.width, source.height, pixels).map_err(|_| PixelateError::InvalidImage)
}

/// Replace only `region` in a full-size BGRA `target`, sampling immutable `source`.
///
/// Uses the same alpha-correct averages as [`pixelate_region`], without allocating
/// a full-size intermediate image. Pixels outside `region` retain their current
/// target values. The target length must exactly match the source byte length;
/// a mismatch returns `InvalidImage`. All validation precedes any target write,
/// so every error leaves the target unchanged.
pub fn pixelate_into(
    source: &Image,
    target: &mut [u8],
    region: PixelateRect,
    block_size: usize,
) -> Result<(), PixelateError> {
    let (right, bottom) = validate_request(source, region, block_size)?;
    if target.len() != source.pixels.len() {
        return Err(PixelateError::InvalidImage);
    }
    pixelate_validated(source, target, region, block_size, right, bottom);
    Ok(())
}

fn validate_request(
    source: &Image,
    region: PixelateRect,
    block_size: usize,
) -> Result<(usize, usize), PixelateError> {
    validate_image(source)?;
    if block_size == 0 || block_size > MAX_BLOCK_SIZE {
        return Err(PixelateError::InvalidBlockSize);
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or(PixelateError::InvalidRegion)?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or(PixelateError::InvalidRegion)?;
    if region.width == 0 || region.height == 0 || right > source.width || bottom > source.height {
        return Err(PixelateError::InvalidRegion);
    }
    Ok((right, bottom))
}

// Call only after validating both buffers and the complete region. These bounds
// also cap each sum at 4096 * 4096 * 255 * 255, safely within u64.
fn pixelate_validated(
    source: &Image,
    target: &mut [u8],
    region: PixelateRect,
    block_size: usize,
    right: usize,
    bottom: usize,
) {
    let mut block_y = region.y;
    while block_y < bottom {
        let block_bottom = block_y + block_size.min(bottom - block_y);
        let mut block_x = region.x;
        while block_x < right {
            let block_right = block_x + block_size.min(right - block_x);
            let sample_count = (block_right - block_x) as u64 * (block_bottom - block_y) as u64;
            let mut alpha_sum = 0u64;
            let mut premultiplied_sum = [0u64; 3];

            for y in block_y..block_bottom {
                for x in block_x..block_right {
                    let offset = (y * source.width + x) * 4;
                    let alpha = u64::from(source.pixels[offset + 3]);
                    alpha_sum += alpha;
                    for (channel, sum) in premultiplied_sum.iter_mut().enumerate() {
                        *sum += u64::from(source.pixels[offset + channel]) * alpha;
                    }
                }
            }

            let alpha = div_round(alpha_sum, sample_count) as u8;
            let mut average = [0u8; 4];
            average[3] = alpha;
            if alpha_sum != 0 {
                for channel in 0..3 {
                    average[channel] = div_round(premultiplied_sum[channel], alpha_sum) as u8;
                }
            }

            for y in block_y..block_bottom {
                for x in block_x..block_right {
                    let offset = (y * source.width + x) * 4;
                    target[offset..offset + 4].copy_from_slice(&average);
                }
            }
            block_x = block_right;
        }
        block_y = block_bottom;
    }
}

fn validate_image(image: &Image) -> Result<(), PixelateError> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|count| count.checked_mul(4))
        .ok_or(PixelateError::InvalidImage)?;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_EDITABLE_IMAGE_DIMENSION
        || image.height > MAX_EDITABLE_IMAGE_DIMENSION
        || bytes != image.pixels.len()
        || bytes > MAX_IMAGE_BYTES
    {
        return Err(PixelateError::InvalidImage);
    }
    Ok(())
}

fn div_round(numerator: u64, denominator: u64) -> u64 {
    (numerator + denominator / 2) / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, pixels: Vec<u8>) -> Image {
        Image::new(width, height, pixels).unwrap()
    }

    #[test]
    fn writes_exact_premultiplied_averages_only_inside_target_region() {
        // Hand-calculated colors exercise hidden RGB, partial alpha, clipped
        // final blocks, and a target whose surrounding pixels differ from source.
        let samples = [
            [10, 20, 30, 0],
            [20, 40, 60, 255],
            [100, 90, 80, 128],
            [60, 80, 100, 255],
            [250, 10, 90, 0],
            [200, 190, 180, 128],
        ];
        for (width, height) in [(5, 4), (4, 3)] {
            let mut source = image(width, height, [3, 7, 11, 17].repeat(width * height));
            let region = PixelateRect {
                x: 1,
                y: 1,
                width: 3,
                height: 2,
            };
            for (index, sample) in samples.iter().enumerate() {
                let offset = ((1 + index / 3) * width + 1 + index % 3) * 4;
                source.pixels[offset..offset + 4].copy_from_slice(sample);
            }
            let original = source.clone();
            for block in [1, 2, 3, MAX_BLOCK_SIZE] {
                let mut target = [201, 202, 203, 204].repeat(width * height);
                let mut expected = target.clone();
                for (index, sample) in samples.iter().enumerate() {
                    let color = match block {
                        1 if sample[3] == 0 => [0, 0, 0, 0],
                        1 => *sample,
                        2 if index % 3 < 2 => [40, 60, 80, 128],
                        2 => [150, 140, 130, 128],
                        _ => [77, 87, 97, 128],
                    };
                    let offset = ((1 + index / 3) * width + 1 + index % 3) * 4;
                    expected[offset..offset + 4].copy_from_slice(&color);
                }
                pixelate_into(&source, &mut target, region, block).unwrap();
                assert_eq!(target, expected, "{width}x{height}, block {block}");
                assert_eq!(source, original);
                // The compatibility API still returns a complete source copy.
                let full = pixelate_region(&source, region, block).unwrap();
                for y in 0..height {
                    for x in 0..width {
                        let offset = (y * width + x) * 4;
                        let inside = (1..4).contains(&x) && (1..3).contains(&y);
                        let expected = if inside { &target } else { &source.pixels };
                        assert_eq!(
                            &full.pixels[offset..offset + 4],
                            &expected[offset..offset + 4]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_requests_never_mutate_target() {
        let source = image(2, 2, vec![79; 16]);
        let valid = PixelateRect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut target = vec![123; 16];
        for region in [
            PixelateRect { width: 0, ..valid },
            PixelateRect { height: 0, ..valid },
            PixelateRect { x: 1, ..valid },
            PixelateRect { y: 1, ..valid },
            PixelateRect {
                x: usize::MAX,
                ..valid
            },
            PixelateRect {
                y: usize::MAX,
                ..valid
            },
        ] {
            assert_eq!(
                pixelate_into(&source, &mut target, region, 1),
                Err(PixelateError::InvalidRegion)
            );
            assert_eq!(target, vec![123; 16]);
        }
        for block in [0, MAX_BLOCK_SIZE + 1, usize::MAX] {
            assert_eq!(
                pixelate_into(&source, &mut target, valid, block),
                Err(PixelateError::InvalidBlockSize)
            );
            assert_eq!(target, vec![123; 16]);
        }
        for len in [0, 4, 15, 17, 20] {
            let mut target = vec![123; len];
            assert_eq!(
                pixelate_into(&source, &mut target, valid, 1),
                Err(PixelateError::InvalidImage)
            );
            assert_eq!(target, vec![123; len]);
        }
        for invalid in [
            Image {
                width: 2,
                height: 2,
                pixels: vec![0; 15],
            },
            Image {
                width: 2,
                height: 2,
                pixels: vec![0; 17],
            },
            Image {
                width: 0,
                height: 2,
                pixels: vec![],
            },
            Image {
                width: usize::MAX,
                height: 2,
                pixels: vec![],
            },
            image(
                MAX_EDITABLE_IMAGE_DIMENSION + 1,
                1,
                vec![0; (MAX_EDITABLE_IMAGE_DIMENSION + 1) * 4],
            ),
        ] {
            assert_eq!(
                pixelate_into(&invalid, &mut target, valid, 1),
                Err(PixelateError::InvalidImage)
            );
            assert_eq!(target, vec![123; 16]);
        }
    }

    #[test]
    fn transparent_and_opaque_samples_average_in_premultiplied_space() {
        let source = image(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 0]);
        let result = pixelate_region(
            &source,
            PixelateRect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            2,
        )
        .unwrap();
        assert_eq!(result.pixels, vec![0, 0, 255, 128, 0, 0, 255, 128]);
        assert_eq!(source.pixels, vec![0, 0, 255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn transparent_samples_do_not_add_hidden_color() {
        let source = image(2, 1, vec![200, 150, 100, 0, 20, 40, 60, 0]);
        let result = pixelate_region(
            &source,
            PixelateRect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            2,
        )
        .unwrap();
        assert_eq!(result.pixels, vec![0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn clips_blocks_to_region_and_preserves_pixels_outside() {
        let source = image(3, 1, vec![0, 0, 10, 255, 0, 0, 20, 255, 0, 0, 30, 255]);
        let result = pixelate_region(
            &source,
            PixelateRect {
                x: 1,
                y: 0,
                width: 2,
                height: 1,
            },
            8,
        )
        .unwrap();
        assert_eq!(
            result.pixels,
            vec![0, 0, 10, 255, 0, 0, 25, 255, 0, 0, 25, 255]
        );
        assert_eq!(
            source.pixels,
            vec![0, 0, 10, 255, 0, 0, 20, 255, 0, 0, 30, 255]
        );
    }

    #[test]
    fn rejects_empty_out_of_bounds_and_overflowing_regions() {
        let source = image(2, 2, vec![0; 16]);
        let invalid = [
            PixelateRect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            PixelateRect {
                x: 1,
                y: 0,
                width: 2,
                height: 1,
            },
            PixelateRect {
                x: usize::MAX,
                y: 0,
                width: 1,
                height: 1,
            },
            PixelateRect {
                x: 0,
                y: 0,
                width: 1,
                height: usize::MAX,
            },
        ];
        for region in invalid {
            assert_eq!(
                pixelate_region(&source, region, 1),
                Err(PixelateError::InvalidRegion)
            );
        }
        assert_eq!(
            pixelate_region(
                &source,
                PixelateRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                0
            ),
            Err(PixelateError::InvalidBlockSize)
        );
        assert_eq!(
            pixelate_region(
                &source,
                PixelateRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                MAX_BLOCK_SIZE + 1
            ),
            Err(PixelateError::InvalidBlockSize)
        );
    }

    #[test]
    fn rejects_inconsistent_source_image() {
        let source = Image {
            width: 2,
            height: 1,
            pixels: vec![0; 4],
        };
        assert_eq!(
            pixelate_region(
                &source,
                PixelateRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                1
            ),
            Err(PixelateError::InvalidImage)
        );
    }
}
