//! Alpha-correct pixelation of a bounded rectangle in a BGRA image.
//!
//! The source remains unchanged. The returned image is a complete independent
//! copy with the requested region replaced by block averages, ready to replace
//! the source image in a document.

use crate::stitch::Image;

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_DIMENSION: usize = 30_000;
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

    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(source.pixels.len())
        .map_err(|_| PixelateError::AllocationFailed)?;
    pixels.extend_from_slice(&source.pixels);

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
                    for channel in 0..3 {
                        premultiplied_sum[channel] +=
                            u64::from(source.pixels[offset + channel]) * alpha;
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
                    pixels[offset..offset + 4].copy_from_slice(&average);
                }
            }
            block_x = block_right;
        }
        block_y = block_bottom;
    }

    Image::new(source.width, source.height, pixels).map_err(|_| PixelateError::InvalidImage)
}

fn validate_image(image: &Image) -> Result<(), PixelateError> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|count| count.checked_mul(4))
        .ok_or(PixelateError::InvalidImage)?;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_DIMENSION
        || image.height > MAX_DIMENSION
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
