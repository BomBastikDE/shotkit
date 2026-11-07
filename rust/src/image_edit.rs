//! Non-destructive image operations over tightly packed BGRA [`Image`] pixels.
//!
//! Cropping validates a non-empty rectangle wholly inside the source and copies
//! bytes exactly, including alpha. Framing returns a new image, preserving the
//! original pixels exactly. An optional deterministic soft shadow is rendered
//! into transparent padding beneath the frame; it uses separable box coverage,
//! not a photo-realistic Gaussian blur.

use crate::{
    image_handle::ImageHandle,
    import_admission::TrackedImage,
    resource_budget::{AdmissionClass, BudgetError, ResourceBudget},
    shapes::Color,
    stitch::Image,
};
use std::{fmt, sync::Arc};

const MAX_SHADOW_RADIUS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Build a crop rectangle from two inclusive image-pixel endpoints.
///
/// Endpoints are normalized for either drag direction and clipped to the
/// image. The returned rectangle uses the exclusive width/height convention
/// expected by [`crop`].
pub fn crop_rect_from_points(
    a: (i32, i32),
    b: (i32, i32),
    image_size: (u32, u32),
) -> Option<CropRect> {
    if image_size.0 == 0 || image_size.1 == 0 {
        return None;
    }
    let max_x = i64::from(image_size.0) - 1;
    let max_y = i64::from(image_size.1) - 1;
    let ax = i64::from(a.0).clamp(0, max_x);
    let ay = i64::from(a.1).clamp(0, max_y);
    let bx = i64::from(b.0).clamp(0, max_x);
    let by = i64::from(b.1).clamp(0, max_y);
    let left = ax.min(bx);
    let top = ay.min(by);
    Some(CropRect {
        x: left as usize,
        y: top as usize,
        width: (ax.max(bx) - left + 1) as usize,
        height: (ay.max(by) - top + 1) as usize,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowOptions {
    /// Offset of the shadow from the framed image, in pixels.
    pub offset_x: i32,
    pub offset_y: i32,
    /// Radius of the separable box blur; zero produces a hard-edged shadow.
    pub blur_radius: usize,
    pub opacity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameOptions {
    pub width: usize,
    pub color: Color,
    pub shadow: Option<ShadowOptions>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageEditError {
    Budget(BudgetError),
    InvalidCrop,
    InvalidShadow,
    ImageTooLarge,
    AllocationFailed,
}
impl fmt::Display for ImageEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(error) => write!(f, "Kein freies Bildbudget für die Bearbeitung: {error}"),
            Self::InvalidCrop => {
                f.write_str("Der Ausschnitt liegt außerhalb des Bildes oder ist leer")
            }
            Self::InvalidShadow => f.write_str("Ungültige Schlagschatten-Einstellungen"),
            Self::ImageTooLarge => f.write_str("Das bearbeitete Bild ist zu groß"),
            Self::AllocationFailed => {
                f.write_str("Nicht genügend Speicher für das bearbeitete Bild")
            }
        }
    }
}
impl std::error::Error for ImageEditError {}

/// Return an exact pixel copy of a checked image-space crop.
/// This portable legacy path does not register output bytes in a resource pool.
pub fn crop(source: &Image, rect: CropRect) -> Result<Image, ImageEditError> {
    let len = crop_len(source, rect)?;
    copy_crop(source, rect, reserve_pixels(len)?)
}

/// Reserve crop OUTPUT pixels in the caller's shared pool before allocating.
/// Existing source/history ownership is retained while the output is created.
/// Reserved admission permits an existing-document edit to use pool headroom;
/// it neither bypasses the common ceiling nor accounts for GDI, shapes or RSS.
/// The returned immutable handle owns pixels and lease until its last owner
/// drops, including when retained in a document, undo/redo or a suspended plan.
pub fn crop_with_budget(
    source: &Image,
    rect: CropRect,
    budget: &ResourceBudget,
) -> Result<ImageHandle, ImageEditError> {
    crop_with_budget_using(source, rect, budget, reserve_pixels)
}

fn crop_with_budget_using(
    source: &Image,
    rect: CropRect,
    budget: &ResourceBudget,
    allocate: impl FnOnce(usize) -> Result<Vec<u8>, ImageEditError>,
) -> Result<ImageHandle, ImageEditError> {
    let len = crop_len(source, rect)?;
    let mut lease = budget
        .try_acquire(len, AdmissionClass::Reserved)
        .map_err(ImageEditError::Budget)?;
    // No budget mutex crosses allocation (or any caller-supplied callback).
    let pixels = allocate(len)?;
    // A fallible exact reserve may still return spare capacity. Reconcile the
    // actual retained allocation before copying pixels or handing off ownership.
    lease
        .try_resize(pixels.capacity())
        .map_err(ImageEditError::Budget)?;
    let image = copy_crop(source, rect, pixels)?;
    Ok(ImageHandle::from_imported(Arc::new(
        TrackedImage::from_reserved(image, lease),
    )))
}

fn crop_len(source: &Image, rect: CropRect) -> Result<usize, ImageEditError> {
    validate_source(source)?;
    let right = rect
        .x
        .checked_add(rect.width)
        .ok_or(ImageEditError::InvalidCrop)?;
    let bottom = rect
        .y
        .checked_add(rect.height)
        .ok_or(ImageEditError::InvalidCrop)?;
    if rect.width == 0 || rect.height == 0 || right > source.width || bottom > source.height {
        return Err(ImageEditError::InvalidCrop);
    }
    rect.width
        .checked_mul(rect.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(ImageEditError::ImageTooLarge)
}

// Called only after crop_len validated source, geometry and all byte products.
// Copying into the preallocated Vec cannot grow it beyond admitted capacity.
fn copy_crop(source: &Image, rect: CropRect, mut pixels: Vec<u8>) -> Result<Image, ImageEditError> {
    let len = rect.width * rect.height * 4;
    if !pixels.is_empty() || pixels.capacity() < len {
        return Err(ImageEditError::AllocationFailed);
    }
    let source_stride = source.width * 4;
    let row_bytes = rect.width * 4;
    for y in rect.y..rect.y + rect.height {
        let begin = y * source_stride + rect.x * 4;
        pixels.extend_from_slice(&source.pixels[begin..begin + row_bytes]);
    }
    Ok(Image {
        width: rect.width,
        height: rect.height,
        pixels,
    })
}

/// Add a colored frame and optional soft shadow without modifying `source`.
/// `color` uses the crate's RGBA order; output storage is BGRA.
pub fn add_frame(source: &Image, options: FrameOptions) -> Result<Image, ImageEditError> {
    let layout = frame_layout(source, options)?;
    let pixels = reserve_pixels(layout.byte_len)?;
    render_frame(source, options, layout, pixels)
}

/// Reserve frame/shadow OUTPUT pixels before allocation in the caller's shared
/// pool. Source/history remain retained; Reserved admission can use headroom but
/// cannot bypass the common ceiling. Pixels and lease share immutable ownership.
pub fn add_frame_with_budget(
    source: &Image,
    options: FrameOptions,
    budget: &ResourceBudget,
) -> Result<ImageHandle, ImageEditError> {
    add_frame_with_budget_using(source, options, budget, reserve_pixels)
}

fn add_frame_with_budget_using(
    source: &Image,
    options: FrameOptions,
    budget: &ResourceBudget,
    allocate: impl FnOnce(usize) -> Result<Vec<u8>, ImageEditError>,
) -> Result<ImageHandle, ImageEditError> {
    let layout = frame_layout(source, options)?;
    let mut lease = budget
        .try_acquire(layout.byte_len, AdmissionClass::Reserved)
        .map_err(ImageEditError::Budget)?;
    // No budget lock crosses allocation. Charge any spare retained capacity
    // before rendering or handing the output to document/history ownership.
    let pixels = allocate(layout.byte_len)?;
    lease
        .try_resize(pixels.capacity())
        .map_err(ImageEditError::Budget)?;
    let image = render_frame(source, options, layout, pixels)?;
    Ok(ImageHandle::from_imported(Arc::new(
        TrackedImage::from_reserved(image, lease),
    )))
}

struct FrameLayout {
    radius: usize,
    left_shadow: usize,
    top_shadow: usize,
    card_width: usize,
    card_height: usize,
    content_x: usize,
    content_y: usize,
    out_width: usize,
    out_height: usize,
    byte_len: usize,
}

fn frame_layout(source: &Image, options: FrameOptions) -> Result<FrameLayout, ImageEditError> {
    validate_source(source)?;
    let shadow = options.shadow;
    if shadow.is_some_and(|s| s.blur_radius > MAX_SHADOW_RADIUS) {
        return Err(ImageEditError::InvalidShadow);
    }
    let radius = shadow.map_or(0, |s| s.blur_radius);
    let (left_shadow, top_shadow, right_shadow, bottom_shadow) = if let Some(s) = shadow {
        let left = radius
            .checked_add(if s.offset_x < 0 {
                s.offset_x.unsigned_abs() as usize
            } else {
                0
            })
            .ok_or(ImageEditError::ImageTooLarge)?;
        let top = radius
            .checked_add(if s.offset_y < 0 {
                s.offset_y.unsigned_abs() as usize
            } else {
                0
            })
            .ok_or(ImageEditError::ImageTooLarge)?;
        let right = radius
            .checked_add(s.offset_x.max(0) as usize)
            .ok_or(ImageEditError::ImageTooLarge)?;
        let bottom = radius
            .checked_add(s.offset_y.max(0) as usize)
            .ok_or(ImageEditError::ImageTooLarge)?;
        (left, top, right, bottom)
    } else {
        (0, 0, 0, 0)
    };
    let card_width = source
        .width
        .checked_add(
            options
                .width
                .checked_mul(2)
                .ok_or(ImageEditError::ImageTooLarge)?,
        )
        .ok_or(ImageEditError::ImageTooLarge)?;
    let card_height = source
        .height
        .checked_add(
            options
                .width
                .checked_mul(2)
                .ok_or(ImageEditError::ImageTooLarge)?,
        )
        .ok_or(ImageEditError::ImageTooLarge)?;
    let content_x = left_shadow
        .checked_add(options.width)
        .ok_or(ImageEditError::ImageTooLarge)?;
    let content_y = top_shadow
        .checked_add(options.width)
        .ok_or(ImageEditError::ImageTooLarge)?;
    let out_width = left_shadow
        .checked_add(card_width)
        .and_then(|n| n.checked_add(right_shadow))
        .ok_or(ImageEditError::ImageTooLarge)?;
    let out_height = top_shadow
        .checked_add(card_height)
        .and_then(|n| n.checked_add(bottom_shadow))
        .ok_or(ImageEditError::ImageTooLarge)?;
    let byte_len = out_width
        .checked_mul(out_height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(ImageEditError::ImageTooLarge)?;
    if byte_len > 512 * 1024 * 1024 {
        return Err(ImageEditError::ImageTooLarge);
    }

    Ok(FrameLayout {
        radius,
        left_shadow,
        top_shadow,
        card_width,
        card_height,
        content_x,
        content_y,
        out_width,
        out_height,
        byte_len,
    })
}

// Geometry and byte products are checked before allocation. Rendering writes
// only into the admitted output capacity and does not allocate scratch images.
fn render_frame(
    source: &Image,
    options: FrameOptions,
    layout: FrameLayout,
    mut pixels: Vec<u8>,
) -> Result<Image, ImageEditError> {
    let FrameLayout {
        radius,
        left_shadow,
        top_shadow,
        card_width,
        card_height,
        content_x,
        content_y,
        out_width,
        out_height,
        byte_len,
    } = layout;
    if !pixels.is_empty() || pixels.capacity() < byte_len {
        return Err(ImageEditError::AllocationFailed);
    }
    let shadow = options.shadow;
    // Reservation above makes initialization infallible: no further allocation
    // is needed, and transparent padding keeps its original zero-filled bytes.
    pixels.resize(byte_len, 0);
    if let Some(shadow) = shadow {
        if shadow.opacity > 0 {
            let sx = content_x as i64 + shadow.offset_x as i64;
            let sy = content_y as i64 + shadow.offset_y as i64;
            let r = radius as i64;
            let kernel = (2 * r + 1) as f64;
            for y in 0..out_height {
                let y = y as i64;
                let y_overlap =
                    interval_overlap(y - r, y + r, sy, sy + card_height as i64 - 1) as f64 / kernel;
                if y_overlap == 0.0 {
                    continue;
                }
                for x in 0..out_width {
                    let x = x as i64;
                    let x_overlap = interval_overlap(x - r, x + r, sx, sx + card_width as i64 - 1)
                        as f64
                        / kernel;
                    let alpha = (shadow.opacity as f64 * x_overlap * y_overlap).round() as u8;
                    if alpha > 0 {
                        let offset = (y as usize * out_width + x as usize) * 4;
                        pixels[offset + 3] = alpha;
                    }
                }
            }
        }
    }

    // The frame is opaque/colored according to the requested RGBA alpha. The
    // original image is copied byte-for-byte on top, preserving its alpha.
    let frame_bgra = [
        options.color.b,
        options.color.g,
        options.color.r,
        options.color.a,
    ];
    for y in top_shadow..top_shadow + card_height {
        for x in left_shadow..left_shadow + card_width {
            let offset = (y * out_width + x) * 4;
            pixels[offset..offset + 4].copy_from_slice(&frame_bgra);
        }
    }
    let source_stride = source.width * 4;
    let dest_stride = out_width * 4;
    for y in 0..source.height {
        let from = y * source_stride;
        let to = (content_y + y) * dest_stride + content_x * 4;
        pixels[to..to + source_stride].copy_from_slice(&source.pixels[from..from + source_stride]);
    }
    Image::new(out_width, out_height, pixels).map_err(|_| ImageEditError::ImageTooLarge)
}

/// Reserve the complete output before writing any pixels. The caller must not
/// grow beyond `byte_len`, so allocation failure stays on the Result path even
/// when release builds abort on panic. The source image is never modified.
fn reserve_pixels(byte_len: usize) -> Result<Vec<u8>, ImageEditError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(byte_len)
        .map_err(|_| ImageEditError::AllocationFailed)?;
    Ok(pixels)
}

fn interval_overlap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    (a1.min(b1) - a0.max(b0) + 1).max(0)
}

fn validate_source(source: &Image) -> Result<(), ImageEditError> {
    let expected = source
        .width
        .checked_mul(source.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(ImageEditError::ImageTooLarge)?;
    if source.width == 0
        || source.height == 0
        || expected != source.pixels.len()
        || expected > 512 * 1024 * 1024
    {
        return Err(ImageEditError::ImageTooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_frame_options() -> FrameOptions {
        FrameOptions {
            width: 1,
            color: Color::rgba(23, 45, 67, 128),
            shadow: None,
        }
    }

    #[test]
    fn budgeted_frame_rejects_invalid_geometry_and_ceiling_before_allocation() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let original = source.clone();
        let budget = ResourceBudget::new(8, 0).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        for options in [
            FrameOptions {
                width: usize::MAX,
                ..budget_frame_options()
            },
            FrameOptions {
                shadow: Some(ShadowOptions {
                    offset_x: 0,
                    offset_y: 0,
                    blur_radius: MAX_SHADOW_RADIUS + 1,
                    opacity: 255,
                }),
                ..budget_frame_options()
            },
        ] {
            assert!(matches!(
                add_frame_with_budget_using(&source, options, &budget, |_| panic!(
                    "invalid frame allocated"
                )),
                Err(ImageEditError::ImageTooLarge | ImageEditError::InvalidShadow)
            ));
        }
        assert!(matches!(
            add_frame_with_budget_using(&source, budget_frame_options(), &budget, |_| panic!(
                "refused frame allocated"
            )),
            Err(ImageEditError::Budget(_))
        ));
        assert_eq!(source, original);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    #[test]
    fn budgeted_frame_allocation_failure_and_capacity_refusal_release_only_output() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let original = source.clone();
        let source_ptr = source.pixels.as_ptr();
        // Frame is 4 x 3 BGRA pixels; all of its 48 bytes use reserved headroom.
        let budget = ResourceBudget::new(56, 48).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        assert_eq!(
            add_frame_with_budget_using(&source, budget_frame_options(), &budget, |len| {
                assert_eq!(len, 48);
                // Reentering the pool proves no budget mutex crosses allocation.
                assert_eq!(budget.snapshot().used_bytes, 56);
                drop(budget.try_acquire(0, AdmissionClass::Reserved).unwrap());
                Err(ImageEditError::AllocationFailed)
            }),
            Err(ImageEditError::AllocationFailed)
        );
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
        assert!(matches!(
            add_frame_with_budget_using(&source, budget_frame_options(), &budget, |_| Ok(
                Vec::with_capacity(49)
            )),
            Err(ImageEditError::Budget(_))
        ));
        assert_eq!(source, original);
        assert_eq!(source.pixels.as_ptr(), source_ptr);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    #[test]
    fn budgeted_frame_matches_legacy_pixels_at_exact_ceiling_for_all_modes() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        for (width, shadow) in [
            (1, None),
            (
                0,
                Some(ShadowOptions {
                    offset_x: 2,
                    offset_y: 1,
                    blur_radius: 1,
                    opacity: 112,
                }),
            ),
            (
                1,
                Some(ShadowOptions {
                    offset_x: -2,
                    offset_y: -1,
                    blur_radius: 1,
                    opacity: 112,
                }),
            ),
        ] {
            let options = FrameOptions {
                width,
                shadow,
                ..budget_frame_options()
            };
            let expected = add_frame(&source, options).unwrap();
            let bytes = expected.pixels.len();
            let budget = ResourceBudget::new(8 + bytes, bytes).unwrap();
            let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
            let output = add_frame_with_budget(&source, options, &budget).unwrap();
            assert_eq!(output.image(), &expected);
            assert_eq!(
                budget.snapshot().used_bytes,
                retained.bytes() + output.pixels.capacity()
            );
            assert!(budget.try_acquire(1, AdmissionClass::Reserved).is_err());
            drop(output);
            assert_eq!(budget.snapshot().used_bytes, retained.bytes());
        }
    }

    #[test]
    fn budgeted_frame_charges_actual_capacity_until_last_shared_owner() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let budget = ResourceBudget::new(80, 0).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        let output = add_frame_with_budget_using(&source, budget_frame_options(), &budget, |_| {
            Ok(Vec::with_capacity(53))
        })
        .unwrap();
        let capacity = output.pixels.capacity();
        assert!(capacity >= 53);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes() + capacity);
        let last = output.clone();
        drop(output);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes() + capacity);
        drop(last);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    fn one_pixel_crop() -> CropRect {
        CropRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        }
    }

    #[test]
    fn budgeted_crop_rejects_geometry_and_shared_ceiling_before_allocation() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let original = source.clone();
        let budget = ResourceBudget::new(8, 0).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        let invalid = CropRect {
            x: usize::MAX,
            ..one_pixel_crop()
        };
        assert_eq!(
            crop_with_budget_using(&source, invalid, &budget, |_| panic!(
                "invalid crop allocated"
            )),
            Err(ImageEditError::InvalidCrop)
        );
        assert!(matches!(
            crop_with_budget_using(&source, one_pixel_crop(), &budget, |_| panic!(
                "refused crop allocated"
            )),
            Err(ImageEditError::Budget(_))
        ));
        assert_eq!(source, original);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    #[test]
    fn budgeted_crop_allocation_failure_and_capacity_refusal_release_only_output() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let original = source.clone();
        let source_ptr = source.pixels.as_ptr();
        let budget = ResourceBudget::new(12, 4).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        assert_eq!(
            crop_with_budget_using(&source, one_pixel_crop(), &budget, |len| {
                // Reenter the same pool: no mutex may cross allocation.
                assert_eq!(len, 4);
                assert_eq!(budget.snapshot().used_bytes, 12);
                drop(budget.try_acquire(0, AdmissionClass::Reserved).unwrap());
                Err(ImageEditError::AllocationFailed)
            }),
            Err(ImageEditError::AllocationFailed)
        );
        assert_eq!(budget.snapshot().used_bytes, 8);
        assert!(matches!(
            crop_with_budget_using(&source, one_pixel_crop(), &budget, |_| {
                Ok(Vec::with_capacity(5))
            }),
            Err(ImageEditError::Budget(_))
        ));
        assert_eq!(source, original);
        assert_eq!(source.pixels.as_ptr(), source_ptr);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    #[test]
    fn budgeted_crop_charges_actual_capacity_and_last_shared_owner() {
        let source = Image::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        let budget = ResourceBudget::new(32, 4).unwrap();
        let retained = budget.try_acquire(8, AdmissionClass::Normal).unwrap();
        let output = crop_with_budget_using(&source, one_pixel_crop(), &budget, |_| {
            Ok(Vec::with_capacity(9))
        })
        .unwrap();
        let capacity = output.pixels.capacity();
        assert!(capacity >= 9);
        assert_eq!(output.pixels, [4, 5, 6, 128]);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes() + capacity);
        let last = output.clone();
        drop(output);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes() + capacity);
        drop(last);
        assert_eq!(budget.snapshot().used_bytes, retained.bytes());
    }

    #[test]
    fn output_reservation_returns_error_for_unrepresentable_capacity() {
        // Deterministically exercises reservation failure without exhausting RAM.
        assert_eq!(
            reserve_pixels(usize::MAX),
            Err(ImageEditError::AllocationFailed)
        );
    }

    #[test]
    fn rejected_frame_size_preserves_source_pixels() {
        let source = Image::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        for width in [usize::MAX, 100_000] {
            assert_eq!(
                add_frame(
                    &source,
                    FrameOptions {
                        width,
                        color: Color::RED,
                        shadow: None,
                    },
                ),
                Err(ImageEditError::ImageTooLarge)
            );
            assert_eq!((source.width, source.height), (1, 1));
            assert_eq!(source.pixels, [1, 2, 3, 4]);
        }
    }

    fn rgba_pixel(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
        [b, g, r, a]
    }

    #[test]
    fn crop_copies_pixels_and_alpha_exactly() {
        let source = Image::new(
            3,
            2,
            vec![
                1, 2, 3, 0, 4, 5, 6, 17, 7, 8, 9, 255, 10, 11, 12, 1, 13, 14, 15, 128, 16, 17, 18,
                255,
            ],
        )
        .unwrap();
        let result = crop(
            &source,
            CropRect {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
        )
        .unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(
            result.pixels,
            vec![4, 5, 6, 17, 7, 8, 9, 255, 13, 14, 15, 128, 16, 17, 18, 255]
        );
    }

    #[test]
    fn crop_rect_from_points_includes_both_image_edges_and_is_direction_independent() {
        let expected = CropRect {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        };
        assert_eq!(
            crop_rect_from_points((0, 0), (99, 79), (100, 80)),
            Some(expected)
        );
        assert_eq!(
            crop_rect_from_points((99, 79), (0, 0), (100, 80)),
            Some(expected)
        );
    }

    #[test]
    fn crop_from_full_image_endpoints_preserves_last_row_and_column() {
        let pixels: Vec<u8> = (0..24).collect();
        let image = Image::new(3, 2, pixels).unwrap();
        let rect = crop_rect_from_points((0, 0), (2, 1), (3, 2)).unwrap();
        assert_eq!(crop(&image, rect).unwrap(), image);
    }

    #[test]
    fn crop_rect_from_points_normalizes_and_clips_each_endpoint() {
        assert_eq!(
            crop_rect_from_points((4, 3), (2, 1), (6, 5)),
            Some(CropRect {
                x: 2,
                y: 1,
                width: 3,
                height: 3,
            })
        );
        assert_eq!(
            crop_rect_from_points((-10, -4), (9, 8), (5, 4)),
            Some(CropRect {
                x: 0,
                y: 0,
                width: 5,
                height: 4,
            })
        );
        assert_eq!(crop_rect_from_points((0, 0), (1, 1), (0, 4)), None);
    }

    #[test]
    fn crop_rejects_empty_overflow_and_outside_rectangles() {
        let source = Image::new(2, 2, vec![0; 16]).unwrap();
        for rect in [
            CropRect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            CropRect {
                x: 1,
                y: 1,
                width: 2,
                height: 1,
            },
            CropRect {
                x: usize::MAX,
                y: 0,
                width: 1,
                height: 1,
            },
        ] {
            assert_eq!(crop(&source, rect), Err(ImageEditError::InvalidCrop));
        }
    }

    #[test]
    fn frame_keeps_source_bytes_and_adds_bgra_border_around_transparent_edges() {
        let source = Image::new(
            2,
            2,
            vec![0, 0, 0, 0, 10, 20, 30, 255, 40, 50, 60, 77, 70, 80, 90, 255],
        )
        .unwrap();
        let result = add_frame(
            &source,
            FrameOptions {
                width: 1,
                color: Color::rgba(200, 100, 50, 255),
                shadow: None,
            },
        )
        .unwrap();
        assert_eq!((result.width, result.height), (4, 4));
        assert_eq!(&result.pixels[0..4], &rgba_pixel(200, 100, 50, 255));
        assert_eq!(&result.pixels[20..28], &source.pixels[..8]);
        assert_eq!(
            &result.pixels[(2 * 4 + 1) * 4..(2 * 4 + 1) * 4 + 8],
            &source.pixels[8..]
        );
        assert_eq!(&result.pixels[20..24], &[0, 0, 0, 0]);
    }

    #[test]
    fn shadow_adds_transparent_padding_and_has_bounded_alpha() {
        let source = Image::new(1, 1, vec![20, 30, 40, 255]).unwrap();
        let result = add_frame(
            &source,
            FrameOptions {
                width: 0,
                color: Color::RED,
                shadow: Some(ShadowOptions {
                    offset_x: 1,
                    offset_y: 0,
                    blur_radius: 1,
                    opacity: 128,
                }),
            },
        )
        .unwrap();
        assert_eq!((result.width, result.height), (4, 3));
        assert!(result
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0 && p[3] < 128));
        assert_eq!(&result.pixels[20..24], &source.pixels[..]);
    }

    #[test]
    fn rejects_unreasonable_shadow_blur() {
        let source = Image::new(1, 1, vec![0; 4]).unwrap();
        assert_eq!(
            add_frame(
                &source,
                FrameOptions {
                    width: 0,
                    color: Color::RED,
                    shadow: Some(ShadowOptions {
                        offset_x: 0,
                        offset_y: 0,
                        blur_radius: MAX_SHADOW_RADIUS + 1,
                        opacity: 255
                    })
                }
            ),
            Err(ImageEditError::InvalidShadow)
        );
    }
}
