//! Alpha-correct document rendering for Windows annotation images.
//!
//! Ordinary annotations are rendered to straight-alpha BGRA layers and
//! composited in document order. Pixelation and soft blur sample the source image
//! sanitized by every redaction, independent of their document order,
//! and replaces only its clipped region in the accumulated result.

mod cursor;
mod effects;

use crate::{
    alpha_composite::source_over,
    alpha_render,
    gaussian_blur::gaussian_blur_region,
    image_handle::ImageHandle,
    pixelate::{pixelate_into, PixelateRect},
    resource_budget::{AdmissionClass, BudgetLease, ResourceBudget},
    shapes::{Shape, ShapeKind},
    stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION},
};

use std::sync::OnceLock;

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
/// Aggregate retained preview pixel capacity across editors and reentrant paints.
/// This excludes source images, shape metadata, transient render buffers, GDI,
/// allocator overhead and RSS; it is separate from the opt-in import budget.
const PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;
static PREVIEW_CACHE_BUDGET: OnceLock<ResourceBudget> = OnceLock::new();
#[cfg(all(test, windows))]
const PIXELATE_BLOCK_SIZE: usize = 12;

/// Render document shapes over `source`, leaving both inputs unchanged.
///
/// Pixelation regions are clipped to the image. They are derived from the
/// shape's floating-point endpoints using floor for the near edges and ceil
/// for the far edges, then clamped to the image bounds.
pub fn render_document(source: &Image, shapes: &[Shape]) -> Result<Image, String> {
    render_document_over(source, source, shapes)
}

/// A transient UI render: pixels and their shared-pool reservation stay together.
/// Field order drops the allocation before releasing admission. This accounts
/// only for the final Rust output buffer. Budgeted rendering also admits its
/// temporary redaction-sanitized sampling copy; other scratch, cache and GDI
/// remain outside this pool.
pub struct BudgetedRender {
    image: Image,
    pixel_lease: BudgetLease,
}

impl BudgetedRender {
    pub fn image(&self) -> &Image {
        debug_assert_eq!(self.pixel_lease.bytes(), self.image.pixels.capacity());
        &self.image
    }

    /// Flatten without exposing mutable pixel ownership or changing capacity.
    pub fn flatten_checkerboard(&mut self) -> Result<(), String> {
        crate::alpha_composite::flatten_checkerboard_in_place(&mut self.image)
    }
}

/// Reserve final document output and any redaction-sanitized sampling copy.
/// Existing-document work may use reserved headroom, but cannot bypass the
/// caller's shared ceiling.
pub fn render_document_budgeted(
    source: &Image,
    shapes: &[Shape],
    budget: &ResourceBudget,
) -> Result<BudgetedRender, String> {
    let mut pixel_lease = reserve_output(source, budget)?;
    let image = render_document_over_with_lease(
        source,
        source,
        shapes,
        Some(&mut pixel_lease),
        Some(budget),
    )?;
    Ok(BudgetedRender { image, pixel_lease })
}

fn reserve_output(source: &Image, budget: &ResourceBudget) -> Result<BudgetLease, String> {
    validate_image(source)?;
    budget
        .try_acquire(source.pixels.len(), AdmissionClass::Reserved)
        .map_err(|error| format!("Kein freies Bildbudget für die Darstellung: {error}"))
}

/// Return the original immutable image when there are no annotations.
/// Retaining an imported handle also retains its pixel reservation. An annotated
/// result owns a new, untracked allocation and leaves the source lease intact.
pub fn render_document_shared(
    source: &ImageHandle,
    shapes: &[Shape],
) -> Result<ImageHandle, String> {
    if shapes.is_empty() {
        validate_image(source)?;
        return Ok(source.clone());
    }
    render_document(source, shapes).map(ImageHandle::from_image)
}

/// Cached, fully rendered prefix of one image's annotation stack.
/// The original image is retained because pixelation samples that image even
/// when earlier annotations are already present in the accumulated output.
/// Its lease travels with the cache, even while a paint takes it out of its
/// window's RefCell. Fields drop in declaration order: pixels before their lease.
pub struct PreviewCache {
    source: ImageHandle,
    shapes: Vec<Shape>,
    rendered: Image,
    pixel_lease: BudgetLease,
}

impl PreviewCache {
    // Reconcile allocator capacity before retention. Parameter order and struct
    // field order keep pixels ahead of the lease on every drop/unwind path.
    fn try_new(
        mut pixel_lease: BudgetLease,
        source: &ImageHandle,
        prefix: &[Shape],
        rendered: Image,
    ) -> Option<Self> {
        if pixel_lease.try_resize(rendered.pixels.capacity()).is_err() {
            return None;
        }
        Some(Self {
            source: source.clone(),
            shapes: prefix.to_vec(),
            rendered,
            pixel_lease,
        })
    }
}

/// Render a preview while retaining the unchanged prefix across mouse moves.
/// `stable_count` must identify the first potentially changing shape; later
/// shapes are still rendered in order, including pixelation against `source`.
pub fn render_preview(
    source: &ImageHandle,
    shapes: &[Shape],
    stable_count: usize,
    cache: &mut Option<PreviewCache>,
) -> Result<Image, String> {
    let budget = PREVIEW_CACHE_BUDGET.get_or_init(|| {
        ResourceBudget::new(PREVIEW_CACHE_BYTES, 0).expect("constant preview budget is valid")
    });
    render_preview_with_budget(source, shapes, stable_count, cache, budget)
}

/// Render UI preview with shared admission for its final output and any
/// redaction-sanitized sampling copy. Prefix cache
/// admission remains in its independent pool, including uncached fallbacks.
pub fn render_preview_budgeted(
    source: &ImageHandle,
    shapes: &[Shape],
    stable_count: usize,
    cache: &mut Option<PreviewCache>,
    budget: &ResourceBudget,
) -> Result<BudgetedRender, String> {
    let mut pixel_lease = reserve_output(source, budget)?;
    let cache_budget = PREVIEW_CACHE_BUDGET.get_or_init(|| {
        ResourceBudget::new(PREVIEW_CACHE_BYTES, 0).expect("constant preview budget is valid")
    });
    let image = render_preview_with_output_lease(
        source,
        shapes,
        stable_count,
        cache,
        cache_budget,
        Some(&mut pixel_lease),
        Some(budget),
    )?;
    Ok(BudgetedRender { image, pixel_lease })
}

// Kept private so callers cannot move an existing cache between accounting
// pools. Tests inject a small pool without depending on global test ordering.
fn render_preview_with_budget(
    source: &ImageHandle,
    shapes: &[Shape],
    stable_count: usize,
    cache: &mut Option<PreviewCache>,
    budget: &ResourceBudget,
) -> Result<Image, String> {
    render_preview_with_output_lease(source, shapes, stable_count, cache, budget, None, None)
}

fn render_preview_with_output_lease(
    source: &ImageHandle,
    shapes: &[Shape],
    stable_count: usize,
    cache: &mut Option<PreviewCache>,
    budget: &ResourceBudget,
    output_lease: Option<&mut BudgetLease>,
    sampling_budget: Option<&ResourceBudget>,
) -> Result<Image, String> {
    let Some(prefix) = shapes.get(..stable_count) else {
        *cache = None;
        return Err("Ungültiger Vorschau-Präfix.".into());
    };
    // Pixelation must sample a source sanitized by every redaction, including
    // those after the cached prefix. Their edits invalidate that dependency.
    if shapes
        .iter()
        .any(|shape| matches!(shape.kind, ShapeKind::Redact { .. }))
    {
        *cache = None;
        return render_document_over_with_lease(
            source,
            source,
            shapes,
            output_lease,
            sampling_budget,
        );
    }
    // An empty prefix is just the source image. Rendering it into the cache
    // would retain a redundant full-size pixel buffer and copy it again below.
    if prefix.is_empty() {
        *cache = None;
        return render_document_over_with_lease(
            source,
            source,
            shapes,
            output_lease,
            sampling_budget,
        );
    }
    let reusable = cache
        .as_ref()
        .is_some_and(|entry| entry.source.ptr_eq(source) && entry.shapes == prefix);
    if !reusable {
        // Release stale pixels and their lease BEFORE rebuilding. Failed
        // admission or rendering must never leave the old source/prefix alive.
        *cache = None;
        validate_image(source)?;
        let Ok(pixel_lease) = budget.try_acquire(source.pixels.len(), AdmissionClass::Normal)
        else {
            return render_document_over_with_lease(
                source,
                source,
                shapes,
                output_lease,
                sampling_budget,
            );
        };
        // try_acquire has released its mutex. Rendering can dispatch GDI+
        // callbacks and reenter paint; the provisional lease remains owned here.
        let rendered =
            render_document_over_with_lease(source, source, prefix, None, sampling_budget)?;
        // Vec::try_reserve_exact may allocate excess capacity. Only retain the
        // buffer after its ACTUAL capacity has been admitted. A rejected growth
        // drops the candidate before releasing its lease and renders uncached.
        *cache = PreviewCache::try_new(pixel_lease, source, prefix, rendered);
        if cache.is_none() {
            return render_document_over_with_lease(
                source,
                source,
                shapes,
                output_lease,
                sampling_budget,
            );
        }
    }
    let entry = cache.as_ref().ok_or("Vorschau-Cache fehlt.")?;
    debug_assert_eq!(entry.pixel_lease.bytes(), entry.rendered.pixels.capacity());
    let mut result = render_document_over_with_lease(
        source,
        &entry.rendered,
        &shapes[stable_count..],
        output_lease,
        sampling_budget,
    )?;
    // Later pixelation samples the original, so restore redactions from the
    // cached prefix as well as those from the changing suffix.
    apply_redactions(&mut result.pixels, source.width, source.height, shapes)?;
    Ok(result)
}

fn render_document_over(
    source: &Image,
    initial: &Image,
    shapes: &[Shape],
) -> Result<Image, String> {
    render_document_over_with_lease(source, initial, shapes, None, None)
}

fn render_document_over_with_lease(
    source: &Image,
    initial: &Image,
    shapes: &[Shape],
    output_lease: Option<&mut BudgetLease>,
    sampling_budget: Option<&ResourceBudget>,
) -> Result<Image, String> {
    validate_image(source)?;
    validate_same_dimensions(source, initial)?;

    let mut output_pixels = allocate_output(initial, output_lease)?;

    // Filters sampling the original must never reveal redacted pixels outside
    // the redaction. Source-independent shapes need no second image buffer.
    let sanitized_source = if shapes
        .iter()
        .any(|shape| matches!(shape.kind, ShapeKind::Redact { .. }))
        && shapes.iter().any(|shape| samples_source(&shape.kind))
    {
        Some(allocate_sampling_source(source, shapes, sampling_budget)?)
    } else {
        None
    };
    let sampling_source = sanitized_source
        .as_ref()
        .map_or(source, SamplingImage::image);

    for shape in shapes {
        match &shape.kind {
            ShapeKind::Redact { .. } => {}
            ShapeKind::Cursor { data } => cursor::cursor(&mut output_pixels, source, data)?,
            ShapeKind::Spotlight { data } => {
                effects::spotlight(&mut output_pixels, sampling_source, data)?
            }
            ShapeKind::Magnifier { data } => {
                effects::magnifier(&mut output_pixels, sampling_source, data, &shape.style)?
            }
            ShapeKind::Blur { start, end } => {
                let Some(region) =
                    clipped_region(source.width, source.height, start.x, start.y, end.x, end.y)?
                else {
                    continue;
                };
                if !shape.style.width.is_finite() || shape.style.width <= 0.0 {
                    return Err("Ungültige Pixelationsstärke.".into());
                }
                let block_size = ((shape.style.width * 3.0) as usize).clamp(12, 4096);
                pixelate_into(sampling_source, &mut output_pixels, region, block_size)
                    .map_err(|error| format!("Pixelation fehlgeschlagen: {error:?}"))?;
            }
            ShapeKind::GaussianBlur { start, end } => {
                let Some(region) =
                    clipped_region(source.width, source.height, start.x, start.y, end.x, end.y)?
                else {
                    continue;
                };
                if !shape.style.width.is_finite() || shape.style.width <= 0.0 {
                    return Err("Ungültige Weichzeichnungsstärke.".into());
                }
                // Reuse the width selector as filter strength. The kernel is
                // bounded independently of image dimensions and style color.
                let radius = (shape.style.width * 2.0).round().clamp(1.0, 64.0) as usize;
                let blurred = gaussian_blur_region(sampling_source, region, radius)
                    .map_err(|error| format!("Weichzeichnen fehlgeschlagen: {error:?}"))?;
                // The filter returns a compact region rather than a full image.
                for row in 0..region.height {
                    let dst = ((region.y + row) * source.width + region.x) * 4;
                    let src = row * region.width * 4;
                    output_pixels[dst..dst + region.width * 4]
                        .copy_from_slice(&blurred.pixels[src..src + region.width * 4]);
                }
            }
            ShapeKind::Freehand { points }
                if points.len() >= 2
                    && points.len() <= 1_000_000
                    && shape.style.width.is_finite()
                    && shape.style.width > 0.0
                    && points.iter().all(|point| {
                        point.x.is_finite()
                            && point.y.is_finite()
                            && point.x.abs() < 1_000_000.0
                            && point.y.abs() < 1_000_000.0
                    }) =>
            {
                // Restrict the temporary layer to the stroke's conservative
                // bounds. Integer translation preserves the pixel grid.
                let bounds = shape.bounds();
                let margin = shape.style.width.clamp(1.0, 256.0) * 12.0 + 16.0;
                let Some(region) = clipped_region(
                    source.width,
                    source.height,
                    bounds.x - margin,
                    bounds.y - margin,
                    bounds.x + bounds.width + margin,
                    bounds.y + bounds.height + margin,
                )?
                else {
                    continue;
                };
                let mut local = shape.clone();
                local.offset(-(region.x as f32), -(region.y as f32));
                let layer = alpha_render::render_shape(region.width, region.height, &local)
                    .map_err(|error| format!("Shape-Rendering fehlgeschlagen: {error:?}"))?;
                for y in 0..region.height {
                    let start = ((region.y + y) * source.width + region.x) * 4;
                    let output = &mut output_pixels[start..start + region.width * 4];
                    let input = &layer.pixels[y * region.width * 4..(y + 1) * region.width * 4];
                    for (dst, src) in output
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(input.as_chunks::<4>().0.iter())
                    {
                        let result = source_over(*dst, *src, 255);
                        dst.copy_from_slice(&result);
                    }
                }
            }
            _ => {
                let layer = alpha_render::render_shape(source.width, source.height, shape)
                    .map_err(|error| format!("Shape-Rendering fehlgeschlagen: {error:?}"))?;
                validate_same_dimensions(source, &layer)?;
                for (dst, src) in output_pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(layer.pixels.as_chunks::<4>().0.iter())
                {
                    let result = source_over(
                        [dst[0], dst[1], dst[2], dst[3]],
                        [src[0], src[1], src[2], src[3]],
                        255,
                    );
                    dst.copy_from_slice(&result);
                }
            }
        }
    }

    apply_redactions(&mut output_pixels, source.width, source.height, shapes)?;
    Image::new(source.width, source.height, output_pixels)
        .map_err(|error| format!("Gerendertes Bild ist ungültig: {error}"))
}

// Keep sampling pixels and their admission together across every shape render,
// including GDI callbacks and disposer reentrancy. Fields drop in declaration
// order; no mutex guard or editor-state borrow lives in this owner.
struct SamplingImage {
    image: Image,
    pixel_lease: Option<BudgetLease>,
}

impl SamplingImage {
    fn image(&self) -> &Image {
        if let Some(lease) = &self.pixel_lease {
            debug_assert_eq!(lease.bytes(), self.image.pixels.capacity());
        }
        &self.image
    }
}

fn allocate_sampling_source(
    source: &Image,
    shapes: &[Shape],
    budget: Option<&ResourceBudget>,
) -> Result<SamplingImage, String> {
    // Declare admission before pixels so every failure/unwind drops the buffer
    // first. Acquisition and capacity reconciliation release their own mutex
    // before callers can reach a renderer or native callback.
    let mut pixel_lease = budget
        .map(|budget| {
            budget
                .try_acquire(source.pixels.len(), AdmissionClass::Reserved)
                .map_err(|error| {
                    format!("Kein freies Bildbudget für die geschwärzte Sampling-Quelle: {error}")
                })
        })
        .transpose()?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(source.pixels.len())
        .map_err(|_| "Nicht genug Speicher für die geschwärzte Pixelationsquelle.".to_owned())?;
    reconcile_sampling_capacity(&pixels, pixel_lease.as_mut())?;
    pixels.extend_from_slice(&source.pixels);
    apply_redactions(&mut pixels, source.width, source.height, shapes)?;
    let image = Image::new(source.width, source.height, pixels)
        .map_err(|error| format!("Geschwärzte Pixelationsquelle ist ungültig: {error}"))?;
    Ok(SamplingImage { image, pixel_lease })
}

fn reconcile_sampling_capacity(
    pixels: &Vec<u8>,
    lease: Option<&mut BudgetLease>,
) -> Result<(), String> {
    if let Some(lease) = lease {
        lease.try_resize(pixels.capacity()).map_err(|error| {
            format!("Bildspeicher für die geschwärzte Sampling-Quelle konnte nicht verbucht werden: {error}")
        })?;
    }
    Ok(())
}

// Account allocator excess before any shape renderer can dispatch callbacks.
// On rejected growth the local Vec drops while the caller still owns its lease.
fn allocate_output(initial: &Image, lease: Option<&mut BudgetLease>) -> Result<Vec<u8>, String> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(initial.pixels.len())
        .map_err(|_| "Nicht genug Speicher für das gerenderte Dokument.".to_owned())?;
    reconcile_output_capacity(&pixels, lease)?;
    pixels.extend_from_slice(&initial.pixels);
    Ok(pixels)
}

fn reconcile_output_capacity(
    pixels: &Vec<u8>,
    lease: Option<&mut BudgetLease>,
) -> Result<(), String> {
    if let Some(lease) = lease {
        lease.try_resize(pixels.capacity()).map_err(|error| {
            format!("Bildspeicher für die Darstellung konnte nicht verbucht werden: {error}")
        })?;
    }
    Ok(())
}

// Exhaustive so newly added effects must explicitly choose a safety policy.
fn samples_source(kind: &ShapeKind) -> bool {
    match kind {
        ShapeKind::Blur { .. } | ShapeKind::GaussianBlur { .. } | ShapeKind::Magnifier { .. } => {
            true
        }
        ShapeKind::Line { .. }
        | ShapeKind::Arrow { .. }
        | ShapeKind::Rectangle { .. }
        | ShapeKind::Ellipse { .. }
        | ShapeKind::Highlight { .. }
        | ShapeKind::Redact { .. }
        | ShapeKind::Freehand { .. }
        | ShapeKind::Text { .. }
        | ShapeKind::Cursor { .. }
        | ShapeKind::Spotlight { .. }
        | ShapeKind::Callout { .. }
        | ShapeKind::Stamp { .. }
        | ShapeKind::Step { .. } => false,
    }
}

// Redactions are applied last, independent of annotation order. Never composite
// them: an opaque replacement prevents alpha or antialiasing from exposing data.
fn apply_redactions(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    shapes: &[Shape],
) -> Result<(), String> {
    for shape in shapes {
        if let ShapeKind::Redact { start, end } = &shape.kind {
            if let Some(region) = clipped_region(width, height, start.x, start.y, end.x, end.y)? {
                for y in region.y..region.y + region.height {
                    let row = (y * width + region.x) * 4;
                    for pixel in pixels[row..row + region.width * 4].as_chunks_mut::<4>().0 {
                        pixel.copy_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_image(image: &Image) -> Result<(), String> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Bildgröße läuft über.".to_owned())?;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_EDITABLE_IMAGE_DIMENSION
        || image.height > MAX_EDITABLE_IMAGE_DIMENSION
        || bytes != image.pixels.len()
        || bytes > MAX_IMAGE_BYTES
    {
        return Err("Ungültige Bildgröße oder BGRA-Pufferlänge.".into());
    }
    Ok(())
}

fn validate_same_dimensions(source: &Image, layer: &Image) -> Result<(), String> {
    validate_image(layer)?;
    if source.width != layer.width || source.height != layer.height {
        return Err("Shape-Ebene hat andere Abmessungen als das Originalbild.".into());
    }
    Ok(())
}

fn clipped_region(
    width: usize,
    height: usize,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) -> Result<Option<PixelateRect>, String> {
    if ![x1, y1, x2, y2].into_iter().all(f32::is_finite) {
        return Err("Bildregion enthält ungültige Koordinaten.".into());
    }

    let left = (x1.min(x2) as f64).floor().clamp(0.0, width as f64) as usize;
    let top = (y1.min(y2) as f64).floor().clamp(0.0, height as f64) as usize;
    let right = (x1.max(x2) as f64).ceil().clamp(0.0, width as f64) as usize;
    let bottom = (y1.max(y2) as f64).ceil().clamp(0.0, height as f64) as usize;

    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some(PixelateRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixelate::pixelate_region;
    #[cfg(windows)]
    use crate::shapes::{Color, Point, Style};
    use std::sync::Arc;

    fn untracked_arc(handle: &ImageHandle) -> &Arc<Image> {
        let ImageHandle::Untracked(image) = handle else {
            panic!("expected untracked test image");
        };
        image
    }

    #[test]
    fn redaction_and_spotlight_do_not_need_a_sampling_copy() {
        use crate::shapes::{Point, Rect, Spotlight};
        let source = Image::new(4, 2, [20, 70, 150, 255].repeat(8)).unwrap();
        let original = source.clone();
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0),
        });
        let spotlight = Shape::new(ShapeKind::Spotlight {
            data: Spotlight::new(
                Rect {
                    x: 1.0,
                    y: 0.0,
                    width: 2.0,
                    height: 2.0,
                },
                0.5,
            )
            .unwrap(),
        });
        assert!(!samples_source(&redact.kind));
        assert!(!samples_source(&spotlight.kind));
        let mut sanitized = source.clone();
        apply_redactions(&mut sanitized.pixels, 4, 2, std::slice::from_ref(&redact)).unwrap();
        for shapes in [
            vec![redact.clone(), spotlight.clone()],
            vec![spotlight.clone(), redact.clone()],
        ] {
            let mut expected = source.pixels.clone();
            let ShapeKind::Spotlight { data } = &spotlight.kind else {
                unreachable!()
            };
            effects::spotlight(&mut expected, &sanitized, data).unwrap();
            apply_redactions(&mut expected, 4, 2, &shapes).unwrap();
            assert_eq!(render_document(&source, &shapes).unwrap().pixels, expected);
        }
        assert_eq!(source, original);
        assert!(samples_source(&ShapeKind::Blur {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0)
        }));
        assert!(samples_source(&ShapeKind::GaussianBlur {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0)
        }));
    }

    #[test]
    fn shared_export_reuses_unannotated_image_and_validates_it() {
        use crate::shapes::Point;
        let source = ImageHandle::from_image(
            Image::new(2, 1, vec![10, 20, 30, 40, 50, 60, 70, 80]).unwrap(),
        );
        let reused = render_document_shared(&source, &[]).unwrap();
        assert!(source.ptr_eq(&reused));
        assert_eq!(*reused, render_document(&source, &[]).unwrap());
        let png = crate::image_io::encode(&reused, image::ImageFormat::Png).unwrap();
        assert_eq!(crate::image_io::decode(&png).unwrap(), *source);

        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0),
        });
        let rendered = render_document_shared(&source, std::slice::from_ref(&redact)).unwrap();
        assert!(!source.ptr_eq(&rendered));
        assert_eq!(*rendered, render_document(&source, &[redact]).unwrap());
        assert_eq!(&source.pixels[..4], &[10, 20, 30, 40]);
        assert_eq!(&rendered.pixels[..4], &[0, 0, 0, 255]);

        let malformed = ImageHandle::from_image(Image {
            width: 1,
            height: 1,
            pixels: vec![],
        });
        let too_wide = ImageHandle::from_image(Image::new(30_001, 1, vec![0; 30_001 * 4]).unwrap());
        let too_tall = ImageHandle::from_image(Image::new(1, 30_001, vec![0; 30_001 * 4]).unwrap());
        let zero = ImageHandle::from_image(Image {
            width: 0,
            height: 1,
            pixels: vec![],
        });
        let excess = ImageHandle::from_image(Image {
            width: 1,
            height: 1,
            pixels: vec![0; 8],
        });
        let overflow = ImageHandle::from_image(Image {
            width: usize::MAX,
            height: 2,
            pixels: vec![],
        });
        let boundary = ImageHandle::from_image(Image::new(30_000, 1, vec![0; 30_000 * 4]).unwrap());
        assert!(boundary.ptr_eq(&render_document_shared(&boundary, &[]).unwrap()));
        for image in [malformed, too_wide, too_tall, zero, excess, overflow] {
            assert!(render_document_shared(&image, &[]).is_err());
            assert!(render_document(&image, &[]).is_err());
        }
    }

    #[test]
    fn cursor_preview_matches_export_and_redaction_wins_in_both_orders() {
        use crate::cursor_snapshot::CursorPixels;
        use crate::shapes::{CursorSprite, Point};
        let source =
            ImageHandle::from_image(Image::new(3, 1, [10, 20, 30, 255].repeat(3)).unwrap());
        let cursor = Shape::new(ShapeKind::Cursor {
            data: CursorSprite::new(
                Point::new(0.0, 0.0),
                3,
                1,
                CursorPixels::Raster {
                    and_mask: vec![255; 3],
                    xor_rgb: vec![255; 9],
                },
            )
            .unwrap(),
        });
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(1.0, 0.0),
            end: Point::new(2.0, 1.0),
        });
        for shapes in [vec![cursor.clone(), redact.clone()], vec![redact, cursor]] {
            let expected = render_document(&source, &shapes).unwrap();
            assert_eq!(&expected.pixels[4..8], &[0, 0, 0, 255]);
            assert_eq!(&expected.pixels[0..4], &[245, 235, 225, 255]);
            for split in 0..=shapes.len() {
                assert_eq!(
                    render_preview(&source, &shapes, split, &mut None).unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn stamps_preview_matches_export_and_never_overpaints_redaction() {
        use crate::shapes::{Point, StampKind};
        let source =
            ImageHandle::from_image(Image::new(60, 60, [20, 40, 60, 90].repeat(3600)).unwrap());
        for kind in StampKind::ALL {
            let mut stamp = Shape::new(ShapeKind::Stamp {
                center: Point::new(30.0, 30.0),
                kind,
            });
            stamp.style.font_size = 48.0;
            stamp.style.color.a = 128;
            let redact = Shape::new(ShapeKind::Redact {
                start: Point::new(20.0, 20.0),
                end: Point::new(40.0, 40.0),
            });
            for shapes in [
                vec![stamp.clone()],
                vec![stamp.clone(), redact.clone()],
                vec![redact.clone(), stamp.clone()],
            ] {
                let expected = render_document(&source, &shapes).unwrap();
                let mut cache = None;
                for split in 0..=shapes.len() {
                    assert_eq!(
                        render_preview(&source, &shapes, split, &mut cache).unwrap(),
                        expected
                    );
                }
                if shapes.len() == 2 {
                    for y in 20..40 {
                        for x in 20..40 {
                            let i = (y * 60 + x) * 4;
                            assert_eq!(&expected.pixels[i..i + 4], &[0, 0, 0, 255]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn gaussian_blur_matches_kernel_preserves_outside_and_ignores_style_color() {
        use crate::shapes::{Color, Point};
        let source = Image::new(
            25,
            19,
            (0..25 * 19)
                .flat_map(|i| [(i * 17) as u8, (i * 3) as u8, 90, [0, 96, 255][i % 3]])
                .collect(),
        )
        .unwrap();
        let before = source.clone();
        let mut shape = Shape::new(ShapeKind::GaussianBlur {
            start: Point::new(20.2, 15.1),
            end: Point::new(3.2, 2.8),
        });
        shape.style.width = 3.0;
        shape.style.color = Color::rgba(255, 0, 0, 0);
        let region = PixelateRect {
            x: 3,
            y: 2,
            width: 18,
            height: 14,
        };
        let filtered = gaussian_blur_region(&source, region, 6).unwrap();
        let rendered = render_document(&source, &[shape.clone()]).unwrap();
        for y in 0..source.height {
            for x in 0..source.width {
                let i = (y * source.width + x) * 4;
                let expected = if (3..21).contains(&x) && (2..16).contains(&y) {
                    let j = ((y - 2) * region.width + x - 3) * 4;
                    &filtered.pixels[j..j + 4]
                } else {
                    &source.pixels[i..i + 4]
                };
                assert_eq!(&rendered.pixels[i..i + 4], expected);
            }
        }
        shape.style.color = Color::rgba(0, 255, 0, 255);
        assert_eq!(
            rendered,
            render_document(&source, &[shape.clone()]).unwrap()
        );
        shape.style.width = 8.0;
        assert_ne!(
            rendered,
            render_document(&source, &[shape.clone()]).unwrap()
        );
        for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            shape.style.width = width;
            assert!(render_document(&source, &[shape.clone()]).is_err());
        }
        assert_eq!(source, before);
    }

    #[test]
    fn overlapping_filters_cannot_expose_redacted_pixels_in_any_order_or_preview() {
        use crate::shapes::Point;
        let source = Image::new(30, 20, [200, 180, 160, 128].repeat(600)).unwrap();
        let mut secret = source.clone();
        for y in 3..9 {
            for x in 4..10 {
                let i = (y * 30 + x) * 4;
                secret.pixels[i..i + 4].copy_from_slice(&[1, 240, 30, 255]);
            }
        }
        let filters = [
            Shape::new(ShapeKind::GaussianBlur {
                start: Point::new(0.0, 0.0),
                end: Point::new(22.0, 16.0),
            }),
            Shape::new(ShapeKind::GaussianBlur {
                start: Point::new(2.0, 1.0),
                end: Point::new(30.0, 20.0),
            }),
            Shape::new(ShapeKind::Blur {
                start: Point::new(0.0, 0.0),
                end: Point::new(30.0, 20.0),
            }),
        ];
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(9.2, 8.2),
            end: Point::new(4.1, 3.1),
        });
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            for position in 0..=3 {
                let mut shapes: Vec<_> = order.iter().map(|&i| filters[i].clone()).collect();
                shapes.insert(position, redact.clone());
                let expected = render_document(&source, &shapes).unwrap();
                assert_eq!(expected, render_document(&secret, &shapes).unwrap());
                for y in 3..9 {
                    for x in 4..10 {
                        let i = (y * 30 + x) * 4;
                        assert_eq!(&expected.pixels[i..i + 4], &[0, 0, 0, 255]);
                    }
                }
                let original = ImageHandle::from_image(secret.clone());
                let mut cache = None;
                for split in 0..=shapes.len() {
                    assert_eq!(
                        render_preview(&original, &shapes, split, &mut cache).unwrap(),
                        expected
                    );
                    assert!(cache.is_none());
                }
            }
        }
    }

    #[test]
    fn gaussian_preview_cache_matches_export_across_strength_edits_and_overlaps() {
        use crate::shapes::Point;
        let source = ImageHandle::from_image(
            Image::new(
                30,
                20,
                (0..600)
                    .flat_map(|i| [i as u8, (i * 11) as u8, 42, [0, 128, 255][i % 3]])
                    .collect(),
            )
            .unwrap(),
        );
        let first = Shape::new(ShapeKind::GaussianBlur {
            start: Point::new(1.0, 2.0),
            end: Point::new(24.0, 18.0),
        });
        let mut second = Shape::new(ShapeKind::GaussianBlur {
            start: Point::new(5.0, 0.0),
            end: Point::new(29.0, 20.0),
        });
        let mut cache = None;
        for width in [1.0, 4.0, 8.0, 2.0] {
            second.style.width = width;
            let shapes = [first.clone(), second.clone()];
            for split in 0..=shapes.len() {
                assert_eq!(
                    render_preview(&source, &shapes, split, &mut cache).unwrap(),
                    render_document(&source, &shapes).unwrap()
                );
            }
        }
    }

    #[test]
    fn redaction_prevents_pixelation_leaking_source_across_region_and_cache() {
        use crate::shapes::{Color, Point};
        let mut one = Image::new(30, 20, [200, 180, 160, 255].repeat(600)).unwrap();
        let mut two = one.clone();
        // Fractional rectangle is rounded outward to cover all touched pixels.
        for y in 3..9 {
            for x in 4..10 {
                let i = (y * 30 + x) * 4;
                one.pixels[i..i + 4].copy_from_slice(&[1, 2, 3, 4]);
                two.pixels[i..i + 4].copy_from_slice(&[240, 230, 220, 210]);
            }
        }
        let mut redact = Shape::new(ShapeKind::Redact {
            start: Point::new(9.2, 8.2),
            end: Point::new(4.1, 3.1),
        });
        redact.style.color = Color::rgba(255, 0, 0, 0);
        let blur = Shape::new(ShapeKind::Blur {
            start: Point::new(0.0, 0.0),
            end: Point::new(30.0, 20.0),
        });
        for shapes in [
            vec![redact.clone(), blur.clone()],
            vec![blur.clone(), redact.clone()],
        ] {
            let result = render_document(&one, &shapes).unwrap();
            assert_eq!(result, render_document(&two, &shapes).unwrap());
            for y in 3..9 {
                for x in 4..10 {
                    let i = (y * 30 + x) * 4;
                    assert_eq!(&result.pixels[i..i + 4], &[0, 0, 0, 255]);
                }
            }
            let source = ImageHandle::from_image(one.clone());
            let mut cache = None;
            for split in 0..=shapes.len() {
                assert_eq!(
                    render_preview(&source, &shapes, split, &mut cache).unwrap(),
                    result
                );
                assert!(cache.is_none());
            }
        }
    }

    #[test]
    fn redaction_clips_and_leaves_outside_unchanged() {
        use crate::shapes::Point;
        let source = Image::new(4, 4, [11, 22, 33, 44].repeat(16)).unwrap();
        let shape = Shape::new(ShapeKind::Redact {
            start: Point::new(-3.0, -2.0),
            end: Point::new(1.2, 1.2),
        });
        let result = render_document(&source, &[shape]).unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                let expected = if x < 2 && y < 2 {
                    [0, 0, 0, 255]
                } else {
                    [11, 22, 33, 44]
                };
                assert_eq!(&result.pixels[i..i + 4], &expected);
            }
        }
    }

    #[test]
    fn redactions_only_preserve_alpha_outside_their_union() {
        use crate::shapes::Point;
        let source = ImageHandle::from_image(
            Image::new(
                5,
                4,
                (0..20)
                    .flat_map(|i| [10 + i as u8, 80, 120, [0, 64, 128, 255][i % 4]])
                    .collect(),
            )
            .unwrap(),
        );
        let original = (*source).clone();
        let shapes = [
            Shape::new(ShapeKind::Redact {
                start: Point::new(-2.0, -1.0),
                end: Point::new(1.2, 1.2),
            }),
            // Reversed fractional endpoints overlap the first rectangle and
            // extend past the right edge of the image.
            Shape::new(ShapeKind::Redact {
                start: Point::new(6.0, 2.2),
                end: Point::new(1.2, 1.2),
            }),
            Shape::new(ShapeKind::Redact {
                start: Point::new(8.0, -3.0),
                end: Point::new(9.0, -1.0),
            }),
        ];
        // Explicit union after outward rounding and clipping:
        // XX... / XXXXX / .XXXX / .....
        let covered = [
            true, true, false, false, false, true, true, true, true, true, false, true, true, true,
            true, false, false, false, false, false,
        ];
        let mut expected = original.clone();
        for (pixel, covered) in expected
            .pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(covered)
        {
            if covered {
                *pixel = [0, 0, 0, 255];
            }
        }
        assert_eq!(render_document(&source, &shapes).unwrap(), expected);
        let mut cache = None;
        let blur = Shape::new(ShapeKind::Blur {
            start: Point::new(0.0, 0.0),
            end: Point::new(3.0, 3.0),
        });
        render_preview(&source, &[blur], 1, &mut cache).unwrap();
        assert!(cache.is_some());
        for split in 0..=shapes.len() {
            assert_eq!(
                render_preview(&source, &shapes, split, &mut cache).unwrap(),
                expected
            );
            assert!(cache.is_none());
        }
        for bad in [f32::NAN, f32::INFINITY] {
            let invalid = Shape::new(ShapeKind::Redact {
                start: Point::new(bad, 0.0),
                end: Point::new(1.0, 1.0),
            });
            assert!(render_document(&source, std::slice::from_ref(&invalid)).is_err());
            assert!(render_preview(&source, &[invalid], 0, &mut cache).is_err());
            assert!(cache.is_none());
        }
        assert_eq!(*source, original);
    }

    #[test]
    fn empty_preview_prefix_matches_document_and_releases_previous_cache() {
        use crate::shapes::Point;
        let source = ImageHandle::from_image(
            Image::new(
                32,
                24,
                (0..32 * 24)
                    .flat_map(|i| [i as u8, (i * 3) as u8, 42, [0, 128, 255][i % 3]])
                    .collect(),
            )
            .unwrap(),
        );
        let blur = Shape::new(ShapeKind::Blur {
            start: Point::new(2.0, 2.0),
            end: Point::new(28.0, 20.0),
        });
        let mut cache = None;
        render_preview(&source, std::slice::from_ref(&blur), 1, &mut cache).unwrap();
        assert!(cache.is_some());
        assert_eq!(Arc::strong_count(untracked_arc(&source)), 2);

        // Only the cache retains this old source now. Clearing the cache must
        // drop its ownership, together with its independently rendered pixels.
        let previous_source = Arc::downgrade(untracked_arc(&source));
        let replacement = ImageHandle::from_image((*source).clone());
        drop(source);
        assert!(previous_source.upgrade().is_some());
        for shapes in [vec![blur], vec![]] {
            assert_eq!(
                render_preview(&replacement, &shapes, 0, &mut cache).unwrap(),
                render_document(&replacement, &shapes).unwrap()
            );
            assert!(cache.is_none());
            assert!(previous_source.upgrade().is_none());
            assert_eq!(Arc::strong_count(untracked_arc(&replacement)), 1);
        }
        assert!(render_preview(&replacement, &[], 1, &mut cache).is_err());
        let malformed = ImageHandle::from_image(Image {
            width: 1,
            height: 1,
            pixels: vec![],
        });
        assert!(render_preview(&malformed, &[], 0, &mut cache).is_err());
    }

    fn budget_preview_fixture() -> (ImageHandle, Vec<Shape>) {
        use crate::shapes::Point;
        let source = ImageHandle::from_image(
            Image::new(
                32,
                24,
                (0..32 * 24)
                    .flat_map(|i| [i as u8, (i * 3) as u8, 42, [0, 128, 255][i % 3]])
                    .collect(),
            )
            .unwrap(),
        );
        let shapes = vec![
            Shape::new(ShapeKind::Blur {
                start: Point::new(2.0, 2.0),
                end: Point::new(28.0, 20.0),
            }),
            Shape::new(ShapeKind::GaussianBlur {
                start: Point::new(8.0, 5.0),
                end: Point::new(31.0, 23.0),
            }),
        ];
        (source, shapes)
    }

    #[test]
    fn budgeted_sampling_preserves_privacy_for_all_sampling_effects_and_preview_splits() {
        use crate::shapes::{Magnifier, MagnifierFrame, Point, Rect};
        let source =
            ImageHandle::from_image(Image::new(8, 4, [7, 10, 240, 255].repeat(32)).unwrap());
        let original = (*source).clone();
        let mut alternate = original.clone();
        // Differences exist ONLY inside the redaction. No sampled destination
        // may depend on those hidden source colors, in either document order.
        for y in 0..2 {
            for x in 0..2 {
                let offset = (y * 8 + x) * 4;
                alternate.pixels[offset..offset + 4].copy_from_slice(&[99, 200, 3, 17]);
            }
        }
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(2.0, 2.0),
        });
        let mut magnifier = Shape::new(ShapeKind::Magnifier {
            data: Magnifier::new(
                Rect {
                    x: 4.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                Point::new(1.0, 1.0),
                2.0,
                MagnifierFrame::Rectangle,
            )
            .unwrap(),
        });
        magnifier.style.width = 0.25;
        let effects = [
            Shape::new(ShapeKind::Blur {
                start: Point::new(0.0, 0.0),
                end: Point::new(6.0, 4.0),
            }),
            Shape::new(ShapeKind::GaussianBlur {
                start: Point::new(0.0, 0.0),
                end: Point::new(6.0, 4.0),
            }),
            magnifier,
        ];
        let bytes = source.pixels.len();
        // Only the source fits normal admission; BOTH temporary allocations
        // must be permitted to use existing-document headroom.
        let budget = ResourceBudget::new(bytes * 3, bytes * 2).unwrap();
        let source_lease = budget.try_acquire(bytes, AdmissionClass::Normal).unwrap();
        for effect in effects {
            for shapes in [
                vec![redact.clone(), effect.clone()],
                vec![effect.clone(), redact.clone()],
            ] {
                let expected = render_document(&source, &shapes).unwrap();
                assert_eq!(expected, render_document(&alternate, &shapes).unwrap());
                let output = render_document_budgeted(&source, &shapes, &budget).unwrap();
                assert_eq!(output.image(), &expected);
                // Sampling is temporary; only the source and returned output
                // remain charged after the renderer returns.
                assert_eq!(
                    budget.snapshot().used_bytes,
                    bytes + output.image().pixels.capacity()
                );
                drop(output);
                let mut public_cache = None;
                let output =
                    render_preview_budgeted(&source, &shapes, 1, &mut public_cache, &budget)
                        .unwrap();
                assert_eq!(output.image(), &expected);
                assert!(public_cache.is_none());
                drop(output);
                for cache_bytes in [0, bytes * 8] {
                    let cache_budget = ResourceBudget::new(cache_bytes, 0).unwrap();
                    let mut cache = None;
                    // Exercise clearing an existing prefix when a redaction is
                    // added, as well as redacted previews with every split.
                    render_preview_with_budget(
                        &source,
                        std::slice::from_ref(&effect),
                        1,
                        &mut cache,
                        &cache_budget,
                    )
                    .unwrap();
                    for split in 0..=shapes.len() {
                        let mut lease = reserve_output(&source, &budget).unwrap();
                        let image = render_preview_with_output_lease(
                            &source,
                            &shapes,
                            split,
                            &mut cache,
                            &cache_budget,
                            Some(&mut lease),
                            Some(&budget),
                        )
                        .unwrap();
                        let output = BudgetedRender {
                            image,
                            pixel_lease: lease,
                        };
                        assert_eq!(output.image(), &expected);
                        assert!(cache.is_none());
                        assert_eq!(cache_budget.snapshot().used_bytes, 0);
                        drop(output);
                        assert_eq!(budget.snapshot().used_bytes, bytes);
                    }
                }
            }
        }
        assert_eq!(*source, original);
        drop(source_lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn sampling_admission_rejection_never_falls_back_to_unsanitized_source() {
        use crate::shapes::{Point, Rect, Spotlight};
        let (source, mut shapes) = budget_preview_fixture();
        let original = (*source).clone();
        let bytes = source.pixels.len();
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(3.0, 4.0),
            end: Point::new(19.0, 15.0),
        });
        shapes.push(redact.clone());
        let budget = ResourceBudget::new(bytes * 3 - 1, 0).unwrap();
        let source_lease = budget.try_acquire(bytes, AdmissionClass::Normal).unwrap();
        let error = render_document_budgeted(&source, &shapes, &budget)
            .err()
            .unwrap();
        assert!(error.contains("Sampling-Quelle"));
        assert_eq!(budget.snapshot().used_bytes, bytes);
        let cache_budget = ResourceBudget::new(bytes * 8, 0).unwrap();
        let mut cache = None;
        render_preview_with_budget(&source, &shapes[..1], 1, &mut cache, &cache_budget).unwrap();
        assert!(cache.is_some());
        for split in 0..=shapes.len() {
            let mut public_cache = None;
            let error =
                render_preview_budgeted(&source, &shapes, split, &mut public_cache, &budget)
                    .err()
                    .unwrap();
            assert!(error.contains("Sampling-Quelle"));
            assert!(public_cache.is_none());
            assert_eq!(budget.snapshot().used_bytes, bytes);
            let mut lease = reserve_output(&source, &budget).unwrap();
            let error = render_preview_with_output_lease(
                &source,
                &shapes,
                split,
                &mut cache,
                &cache_budget,
                Some(&mut lease),
                Some(&budget),
            )
            .unwrap_err();
            assert!(error.contains("Sampling-Quelle"));
            assert!(cache.is_none());
            assert_eq!(cache_budget.snapshot().used_bytes, 0);
            drop(lease);
            assert_eq!(budget.snapshot().used_bytes, bytes);
        }
        // Source-independent operations still work with room for output only.
        let spotlight = Shape::new(ShapeKind::Spotlight {
            data: Spotlight::new(
                Rect {
                    x: 1.0,
                    y: 1.0,
                    width: 8.0,
                    height: 8.0,
                },
                0.5,
            )
            .unwrap(),
        });
        let output_only = ResourceBudget::new(bytes, 0).unwrap();
        for independent in [vec![], vec![redact.clone()], vec![redact, spotlight]] {
            let output = render_document_budgeted(&source, &independent, &output_only).unwrap();
            assert_eq!(
                output.image(),
                &render_document(&source, &independent).unwrap()
            );
            drop(output);
            assert_eq!(output_only.snapshot().used_bytes, 0);
        }
        assert_eq!(*source, original);
        drop(source_lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn sampling_lease_survives_nested_work_and_releases_on_error_and_unwind() {
        use crate::shapes::Point;
        let (source, mut shapes) = budget_preview_fixture();
        shapes.push(Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(2.0, 2.0),
        }));
        let bytes = source.pixels.len();
        let budget = ResourceBudget::new(bytes * 2, 0).unwrap();
        let sampling = allocate_sampling_source(&source, &shapes, Some(&budget)).unwrap();
        assert_eq!(
            budget.snapshot().used_bytes,
            sampling.image().pixels.capacity()
        );
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Reserved).unwrap();
        // Models a callback while the outer renderer owns its sanitized source.
        // Reentering this same pool would deadlock if a guard were retained.
        let result = std::panic::catch_unwind(|| {
            let _sampling = sampling;
            let _blocker = blocker;
            assert!(render_document_budgeted(&source, &shapes, &budget).is_err());
            assert_eq!(budget.snapshot().used_bytes, bytes * 2);
            panic!("simulate cancelled render callback");
        });
        assert!(result.is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
        let invalid_redaction = Shape::new(ShapeKind::Redact {
            start: Point::new(f32::NAN, 0.0),
            end: Point::new(2.0, 2.0),
        });
        assert!(allocate_sampling_source(&source, &[invalid_redaction], Some(&budget)).is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
        // A rendering error after successful sampling also releases BOTH leases.
        shapes[0].style.width = f32::NAN;
        let error = render_document_budgeted(&source, &shapes, &budget)
            .err()
            .unwrap();
        assert!(error.contains("Pixelationsstärke"));
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn sampling_capacity_reconciliation_rejects_excess_atomically() {
        let pixels = Vec::<u8>::with_capacity(32);
        let capacity = pixels.capacity();
        let small = ResourceBudget::new(capacity - 1, 0).unwrap();
        let mut lease = small.try_acquire(1, AdmissionClass::Reserved).unwrap();
        assert!(reconcile_sampling_capacity(&pixels, Some(&mut lease)).is_err());
        assert_eq!(lease.bytes(), 1);
        assert_eq!(small.snapshot().used_bytes, 1);
        drop(pixels);
        drop(lease);
        assert_eq!(small.snapshot().used_bytes, 0);
    }

    #[test]
    fn output_admission_uses_headroom_and_releases_on_drop_and_render_error() {
        let (source, shapes) = budget_preview_fixture();
        let bytes = source.pixels.len();
        let budget = ResourceBudget::new(bytes * 3, bytes * 2).unwrap();
        let source_lease = budget.try_acquire(bytes, AdmissionClass::Normal).unwrap();
        assert!(budget.try_acquire(1, AdmissionClass::Normal).is_err());
        let output = render_document_budgeted(&source, &shapes, &budget).unwrap();
        assert_eq!(output.image(), &render_document(&source, &shapes).unwrap());
        assert_eq!(
            budget.snapshot().used_bytes,
            bytes + output.image().pixels.capacity()
        );
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Reserved).unwrap();
        assert!(render_document_budgeted(&source, &shapes, &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, bytes * 3);
        drop(blocker);
        drop(output);
        assert_eq!(budget.snapshot().used_bytes, bytes);
        let mut invalid = shapes;
        invalid[0].style.width = f32::NAN;
        assert!(render_document_budgeted(&source, &invalid, &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, bytes);
        drop(source_lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn output_reservation_survives_nested_work_and_releases_on_unwind() {
        let (source, shapes) = budget_preview_fixture();
        let budget = ResourceBudget::new(source.pixels.len() * 2, 0).unwrap();
        let output = render_document_budgeted(&source, &shapes, &budget).unwrap();
        assert_eq!(
            budget.snapshot().used_bytes,
            output.image().pixels.capacity()
        );
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Reserved).unwrap();
        let nested_budget = budget.clone();
        let result = std::panic::catch_unwind(move || {
            // Models work reentered while outer pixels are still owned.
            assert!(render_document_budgeted(&source, &shapes, &nested_budget).is_err());
            assert!(!output.image().pixels.is_empty());
            assert_eq!(blocker.bytes(), spare);
            panic!("simulate cancelled UI render");
        });
        assert!(result.is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn output_capacity_is_reconciled_before_use_and_growth_failure_is_atomic() {
        let pixels = Vec::<u8>::with_capacity(32);
        let capacity = pixels.capacity();
        let small = ResourceBudget::new(capacity - 1, 0).unwrap();
        let mut lease = small.try_acquire(1, AdmissionClass::Reserved).unwrap();
        assert!(reconcile_output_capacity(&pixels, Some(&mut lease)).is_err());
        assert_eq!(lease.bytes(), 1);
        drop(pixels);
        drop(lease);
        assert_eq!(small.snapshot().used_bytes, 0);

        let image = Image::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let budget = ResourceBudget::new(16, 0).unwrap();
        let mut lease = reserve_output(&image, &budget).unwrap();
        let pixels = allocate_output(&image, Some(&mut lease)).unwrap();
        assert_eq!(lease.bytes(), pixels.capacity());
        assert_eq!(budget.snapshot().used_bytes, pixels.capacity());
        assert_eq!(pixels, image.pixels);
        drop(pixels);
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn budgeted_preview_preserves_cache_fallback_redactions_and_checkerboard() {
        use crate::shapes::Point;
        let (source, mut shapes) = budget_preview_fixture();
        let original = (*source).clone();
        let output_budget = ResourceBudget::new(source.pixels.len() * 4, 0).unwrap();
        for cache_bytes in [0, source.pixels.len() * 8] {
            let cache_budget = ResourceBudget::new(cache_bytes, 0).unwrap();
            let mut cache = None;
            for redacted in [false, true] {
                if redacted {
                    shapes.push(Shape::new(ShapeKind::Redact {
                        start: Point::new(3.0, 4.0),
                        end: Point::new(19.0, 15.0),
                    }));
                }
                for split in 0..=shapes.len() {
                    // Repeat to exercise both cache construction and reuse.
                    for _ in 0..2 {
                        let mut pixel_lease = reserve_output(&source, &output_budget).unwrap();
                        let image = render_preview_with_output_lease(
                            &source,
                            &shapes,
                            split,
                            &mut cache,
                            &cache_budget,
                            Some(&mut pixel_lease),
                            Some(&output_budget),
                        )
                        .unwrap();
                        let mut output = BudgetedRender { image, pixel_lease };
                        let expected = render_document(&source, &shapes).unwrap();
                        assert_eq!(output.image(), &expected);
                        output.flatten_checkerboard().unwrap();
                        assert_eq!(
                            output.image(),
                            &crate::alpha_composite::flatten_checkerboard(&expected).unwrap()
                        );
                        drop(output);
                        assert_eq!(output_budget.snapshot().used_bytes, 0);
                    }
                }
                if redacted {
                    shapes.pop();
                }
            }
            drop(cache);
            assert_eq!(cache_budget.snapshot().used_bytes, 0);
        }
        assert_eq!(*source, original);
    }

    #[test]
    fn budgeted_preview_rejection_keeps_cache_and_nested_outputs_cannot_bypass_pool() {
        let (source, shapes) = budget_preview_fixture();
        let budget = ResourceBudget::new(source.pixels.len() * 2, 0).unwrap();
        let mut cache = None;
        let first = render_preview_budgeted(&source, &shapes, 1, &mut cache, &budget).unwrap();
        let retained = cache.as_ref().map(|entry| entry.rendered.pixels.as_ptr());
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Reserved).unwrap();
        assert!(render_preview_budgeted(&source, &shapes, 1, &mut cache, &budget).is_err());
        assert_eq!(
            cache.as_ref().map(|entry| entry.rendered.pixels.as_ptr()),
            retained
        );
        drop(blocker);
        drop(first);
        assert_eq!(budget.snapshot().used_bytes, 0);
        // Invalid prefix errors release the provisional output reservation.
        assert!(
            render_preview_budgeted(&source, &shapes, shapes.len() + 1, &mut cache, &budget)
                .is_err()
        );
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn preview_budget_aggregates_editors_and_falls_back_without_data_loss() {
        let (source, shapes) = budget_preview_fixture();
        let original = (*source).clone();
        let budget = ResourceBudget::new(source.pixels.len() * 8, 0).unwrap();
        let mut first = None;
        let mut second = None;
        let expected = render_document(&source, &shapes).unwrap();
        for cache in [&mut first, &mut second] {
            assert_eq!(
                render_preview_with_budget(&source, &shapes, 1, cache, &budget).unwrap(),
                expected
            );
            assert!(cache.is_some());
        }
        let first_bytes = first.as_ref().unwrap().rendered.pixels.capacity();
        let second_bytes = second.as_ref().unwrap().rendered.pixels.capacity();
        assert_eq!(budget.snapshot().used_bytes, first_bytes + second_bytes);
        // Occupy all remaining headroom: a third editor must retain no prefix.
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Normal).unwrap();
        let mut third = None;
        assert_eq!(
            render_preview_with_budget(&source, &shapes, 1, &mut third, &budget).unwrap(),
            expected
        );
        assert!(third.is_none());
        assert_eq!(*source, original);
        // Dropping an editor releases only that editor's rendered pixel capacity.
        drop(first);
        assert_eq!(budget.snapshot().used_bytes, second_bytes + spare);
        drop(second);
        drop(blocker);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn moved_out_preview_keeps_lease_during_nested_paint_and_releases_on_early_exit() {
        use std::{cell::RefCell, rc::Rc};
        let (source, shapes) = budget_preview_fixture();
        let budget = ResourceBudget::new(source.pixels.len() * 8, 0).unwrap();
        let mut initial = None;
        render_preview_with_budget(&source, &shapes, 1, &mut initial, &budget).unwrap();
        let shared = Rc::new(RefCell::new(initial));
        let used = budget.snapshot().used_bytes;
        let spare = budget.snapshot().ceiling_bytes - used;
        let blocker = budget.try_acquire(spare, AdmissionClass::Normal).unwrap();
        // Mirrors composited_bitmap: the RefCell is empty and unborrowed while
        // the outer paint's local cache remains charged to the shared pool.
        let local = shared.borrow_mut().take();
        assert_eq!(budget.snapshot().used_bytes, used + spare);
        let mut nested = shared.borrow_mut().take();
        assert_eq!(
            render_preview_with_budget(&source, &shapes, 1, &mut nested, &budget).unwrap(),
            render_document(&source, &shapes).unwrap()
        );
        assert!(nested.is_none());
        // Early return/unwind drops local ownership just like a window closing.
        drop(local);
        assert_eq!(budget.snapshot().used_bytes, spare);
        drop(blocker);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn stale_preview_is_released_before_rebuild_and_on_render_or_prefix_error() {
        let (source, mut shapes) = budget_preview_fixture();
        let budget = ResourceBudget::new(source.pixels.len() * 8, 0).unwrap();
        let mut cache = None;
        render_preview_with_budget(&source, &shapes, 1, &mut cache, &budget).unwrap();
        let previous = Arc::downgrade(untracked_arc(&source));
        let replacement = ImageHandle::from_image((*source).clone());
        drop(source);
        assert!(previous.upgrade().is_some());
        // Leave no room for a replacement until the stale lease is released.
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Normal).unwrap();
        render_preview_with_budget(&replacement, &shapes, 1, &mut cache, &budget).unwrap();
        assert!(cache.is_some());
        assert!(previous.upgrade().is_none());
        drop(blocker);
        assert_eq!(
            budget.snapshot().used_bytes,
            cache.as_ref().unwrap().rendered.pixels.capacity()
        );
        shapes[0].style.width = f32::NAN;
        assert!(render_preview_with_budget(&replacement, &shapes, 1, &mut cache, &budget).is_err());
        assert!(cache.is_none());
        assert_eq!(budget.snapshot().used_bytes, 0);
        shapes[0].style.width = 3.0;
        render_preview_with_budget(&replacement, &shapes, 1, &mut cache, &budget).unwrap();
        assert!(render_preview_with_budget(&replacement, &shapes, 3, &mut cache, &budget).is_err());
        assert!(cache.is_none());
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn preview_retention_accounts_capacity_and_rejects_allocator_excess() {
        let (source, shapes) = budget_preview_fixture();
        let mut rendered = render_document(&source, &shapes[..1]).unwrap();
        rendered.pixels.reserve_exact(source.pixels.len());
        assert!(rendered.pixels.capacity() > rendered.pixels.len());
        let small = ResourceBudget::new(rendered.pixels.len(), 0).unwrap();
        let lease = small
            .try_acquire(rendered.pixels.len(), AdmissionClass::Normal)
            .unwrap();
        assert!(PreviewCache::try_new(lease, &source, &shapes[..1], rendered).is_none());
        assert_eq!(small.snapshot().used_bytes, 0);

        let mut rendered = render_document(&source, &shapes[..1]).unwrap();
        rendered.pixels.reserve_exact(source.pixels.len());
        let capacity = rendered.pixels.capacity();
        let budget = ResourceBudget::new(capacity, 0).unwrap();
        let lease = budget
            .try_acquire(rendered.pixels.len(), AdmissionClass::Normal)
            .unwrap();
        let cache = PreviewCache::try_new(lease, &source, &shapes[..1], rendered).unwrap();
        assert_eq!(budget.snapshot().used_bytes, capacity);
        assert_eq!(cache.pixel_lease.bytes(), capacity);
        drop(cache);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn zero_preview_budget_preserves_pixels_for_every_prefix_and_redactions() {
        use crate::shapes::Point;
        let (source, mut shapes) = budget_preview_fixture();
        let original = (*source).clone();
        let budget = ResourceBudget::new(0, 0).unwrap();
        let mut cache = None;
        for redacted in [false, true] {
            if redacted {
                shapes.push(Shape::new(ShapeKind::Redact {
                    start: Point::new(3.0, 4.0),
                    end: Point::new(19.0, 15.0),
                }));
            }
            let expected = render_document(&source, &shapes).unwrap();
            for split in 0..=shapes.len() {
                assert_eq!(
                    render_preview_with_budget(&source, &shapes, split, &mut cache, &budget)
                        .unwrap(),
                    expected
                );
                assert!(cache.is_none());
                assert_eq!(budget.snapshot().used_bytes, 0);
            }
        }
        assert_eq!(*source, original);
    }

    #[test]
    fn tracked_preview_cache_retains_identity_pixels_and_lease_until_cleared() {
        use crate::{
            import_admission::ImportAdmission,
            resource_budget::{AdmissionClass, ResourceBudget},
            shapes::Point,
        };
        let image = Image::new(
            24,
            12,
            (0..24 * 12)
                .flat_map(|i| [i as u8, (i * 3) as u8, 42, 255])
                .collect(),
        )
        .unwrap();
        let bytes = image.pixels.capacity();
        let pixels = image.pixels.as_ptr();
        let budget = ResourceBudget::new(bytes, 0).unwrap();
        let owner = ImportAdmission::new(budget.clone());
        let lease = budget.try_acquire(bytes, AdmissionClass::Normal).unwrap();
        let tracked = owner.try_begin(image, lease).unwrap().complete();
        let weak = Arc::downgrade(&tracked);
        let source = ImageHandle::from_imported(tracked);
        let shared = render_document_shared(&source, &[]).unwrap();
        assert!(source.ptr_eq(&shared));
        assert_eq!(shared.pixels.as_ptr(), pixels);
        let shapes = [Shape::new(ShapeKind::Blur {
            start: Point::new(1.0, 1.0),
            end: Point::new(23.0, 11.0),
        })];
        let expected = render_document(&source, &shapes).unwrap();
        let mut cache = None;
        assert_eq!(
            render_preview(&source, &shapes, 1, &mut cache).unwrap(),
            expected
        );
        assert!(cache.as_ref().unwrap().source.ptr_eq(&source));
        let cached_pixels = cache.as_ref().unwrap().rendered.pixels.as_ptr();
        assert_eq!(
            render_preview(&shared, &shapes, 1, &mut cache).unwrap(),
            expected
        );
        assert_eq!(
            cache.as_ref().unwrap().rendered.pixels.as_ptr(),
            cached_pixels
        );
        drop(owner);
        drop(source);
        drop(shared);
        assert!(weak.upgrade().is_some());
        assert_eq!(budget.snapshot().used_bytes, bytes);
        assert_eq!(cache.as_ref().unwrap().source.pixels.as_ptr(), pixels);
        assert_eq!(
            render_document(&cache.as_ref().unwrap().source, &shapes).unwrap(),
            expected
        );

        // Clearing the prefix releases the last tracked owner; its independent
        // rendered buffer must not accidentally keep or detach the lease.
        let replacement = ImageHandle::from_image(Image::new(1, 1, vec![0; 4]).unwrap());
        render_preview(&replacement, &[], 0, &mut cache).unwrap();
        assert!(cache.is_none());
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn preview_cache_preserves_pixelation_source_and_invalidates_changes() {
        use crate::shapes::Point;
        let pixels = (0..64 * 48)
            .flat_map(|i| [(i % 251) as u8, ((i * 3) % 253) as u8, 42, 255])
            .collect();
        let source = ImageHandle::from_image(Image::new(64, 48, pixels).unwrap());
        let mut first = Shape::new(ShapeKind::Blur {
            start: Point::new(2.0, 2.0),
            end: Point::new(40.0, 30.0),
        });
        first.style.width = 3.0;
        let mut second = Shape::new(ShapeKind::Blur {
            start: Point::new(20.0, 10.0),
            end: Point::new(62.0, 42.0),
        });
        second.style.width = 5.0;
        let mut cache = None;
        let shapes = vec![first.clone(), second.clone()];
        let expected = render_document(&source, &shapes).unwrap();
        assert_eq!(
            render_preview(&source, &shapes, 1, &mut cache).unwrap(),
            expected
        );
        let cached_pixels = cache.as_ref().unwrap().rendered.pixels.as_ptr();
        second.style.width = 8.0;
        let changed = vec![first.clone(), second];
        assert_eq!(
            render_preview(&source, &changed, 1, &mut cache).unwrap(),
            render_document(&source, &changed).unwrap()
        );
        assert_eq!(
            cache.as_ref().unwrap().rendered.pixels.as_ptr(),
            cached_pixels
        );
        first.style.width = 7.0;
        let changed_prefix = vec![first, changed[1].clone()];
        assert_eq!(
            render_preview(&source, &changed_prefix, 1, &mut cache).unwrap(),
            render_document(&source, &changed_prefix).unwrap()
        );
        assert_eq!(cache.as_ref().unwrap().shapes, changed_prefix[..1]);
        let replacement = ImageHandle::from_image((*source).clone());
        render_preview(&replacement, &changed_prefix, 1, &mut cache).unwrap();
        assert!(cache.as_ref().unwrap().source.ptr_eq(&replacement));
        assert!(render_preview(&replacement, &changed_prefix, 3, &mut cache).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn preview_cache_matches_document_for_transparent_shapes_and_pixelation() {
        let source = ImageHandle::from_image(Image::new(64, 48, vec![80; 64 * 48 * 4]).unwrap());
        let mut arrow = Shape::new(ShapeKind::Line {
            start: Point::new(2.0, 3.0),
            end: Point::new(58.0, 39.0),
        });
        arrow.style.color = Color::rgba(230, 10, 10, 160);
        let blur = Shape::new(ShapeKind::Blur {
            start: Point::new(10.0, 8.0),
            end: Point::new(50.0, 35.0),
        });
        let shapes = vec![arrow, blur];
        let mut cache = None;
        for split in 0..=shapes.len() {
            assert_eq!(
                render_preview(&source, &shapes, split, &mut cache).unwrap(),
                render_document(&source, &shapes).unwrap()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn bounded_freehand_matches_full_image_render_pixel_for_pixel() {
        let image = Image::new(
            180,
            140,
            (0..180 * 140)
                .flat_map(|i| [i as u8, (i / 180) as u8, 90, [0, 128, 255][i % 3]])
                .collect(),
        )
        .unwrap();
        for (points, width, alpha) in [
            (
                vec![
                    Point::new(30.0, 35.0),
                    Point::new(80.0, 90.0),
                    Point::new(130.0, 45.0),
                ],
                4.0,
                255,
            ),
            (
                vec![
                    Point::new(-10.25, 4.5),
                    Point::new(60.5, 70.25),
                    Point::new(210.0, 150.0),
                ],
                1.0,
                128,
            ),
            (
                vec![
                    Point::new(50.0, 50.0),
                    Point::new(51.0, 50.0),
                    Point::new(60.0, 60.0),
                ],
                37.0,
                200,
            ),
        ] {
            let mut shape = Shape::new(ShapeKind::Freehand { points });
            shape.style.width = width;
            shape.style.color = Color::rgba(200, 30, 80, alpha);
            let layer = alpha_render::render_shape(image.width, image.height, &shape).unwrap();
            let mut expected = image.clone();
            for (dst, src) in expected
                .pixels
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(layer.pixels.as_chunks::<4>().0.iter())
            {
                let result = source_over(*dst, *src, 255);
                dst.copy_from_slice(&result);
            }
            let actual = render_document(&image, &[shape]).unwrap();
            assert!(
                actual == expected,
                "bounded freehand changed rendered pixels"
            );
        }
    }

    #[test]
    fn pixelation_uses_at_least_twelve_pixel_blocks() {
        use crate::shapes::Point;
        let mut pixels = Vec::new();
        for y in 0..24 {
            for x in 0..24 {
                pixels.extend_from_slice(&[(x * 9) as u8, (y * 9) as u8, 0, 255]);
            }
        }
        let original = Image::new(24, 24, pixels).unwrap();
        let region = PixelateRect {
            x: 0,
            y: 0,
            width: 24,
            height: 24,
        };
        let expected = pixelate_region(&original, region, 12).unwrap();
        for width in [1.0, 2.0, 3.0, 4.0] {
            let mut blur = Shape::new(ShapeKind::Blur {
                start: Point::new(0.0, 0.0),
                end: Point::new(24.0, 24.0),
            });
            blur.style.width = width;
            assert_eq!(render_document(&original, &[blur]).unwrap(), expected);
        }
    }

    #[test]
    fn pixelation_respects_line_width() {
        use crate::shapes::Point;
        let mut pixels = Vec::new();
        for y in 0..24 {
            for x in 0..24 {
                pixels.extend_from_slice(&[(x * 9) as u8, (y * 9) as u8, 0, 255]);
            }
        }
        let original = Image::new(24, 24, pixels).unwrap();
        let mut blur = Shape::new(ShapeKind::Blur {
            start: Point::new(0.0, 0.0),
            end: Point::new(24.0, 24.0),
        });
        blur.style.width = 2.0;
        let small = render_document(&original, &[blur.clone()]).unwrap();
        blur.style.width = 8.0;
        let large = render_document(&original, &[blur]).unwrap();
        assert_ne!(small, large);
    }

    #[cfg(windows)]
    fn point(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[cfg(windows)]
    fn highlight(start: Point, end: Point, color: Color) -> Shape {
        let mut shape = Shape::new(ShapeKind::Highlight { start, end });
        shape.style = Style {
            color,
            ..Style::default()
        };
        shape
    }

    #[cfg(windows)]
    #[test]
    fn highlight_composition_preserves_base_alpha_and_source() {
        let original = Image::new(16, 16, [20, 40, 60, 91].repeat(16 * 16)).unwrap();
        let before = original.clone();
        let shape = highlight(
            point(2.0, 2.0),
            point(12.0, 12.0),
            Color::rgba(255, 0, 0, 255),
        );
        let result = render_document(&original, &[shape]).unwrap();
        let outside = 0;
        assert_eq!(&result.pixels[outside..outside + 4], &[20, 40, 60, 91]);
        let inside = (7 * 16 + 7) * 4;
        assert!(result.pixels[inside + 3] > 91);
        assert!(result.pixels[inside + 2] > 60);
        assert_eq!(original, before);
    }

    #[cfg(windows)]
    #[test]
    fn normal_shapes_compose_in_document_order() {
        let original = Image::new(16, 16, vec![0; 16 * 16 * 4]).unwrap();
        let first = highlight(
            point(1.0, 1.0),
            point(14.0, 14.0),
            Color::rgba(255, 0, 0, 255),
        );
        let second = highlight(
            point(1.0, 1.0),
            point(14.0, 14.0),
            Color::rgba(0, 0, 255, 255),
        );
        let forward = render_document(&original, &[first.clone(), second.clone()]).unwrap();
        let reverse = render_document(&original, &[second, first]).unwrap();
        let center = (7 * 16 + 7) * 4;
        assert_ne!(
            &forward.pixels[center..center + 4],
            &reverse.pixels[center..center + 4]
        );
    }

    #[cfg(windows)]
    #[test]
    fn pixelation_samples_original_and_overwrites_prior_shapes_only_in_region() {
        let mut pixels = Vec::new();
        for y in 0..24 {
            for x in 0..24 {
                pixels.extend_from_slice(&[x as u8, y as u8, 40, if x == 0 { 80 } else { 255 }]);
            }
        }
        let original = Image::new(24, 24, pixels).unwrap();
        let before = original.clone();
        let marker = highlight(
            point(0.0, 0.0),
            point(12.0, 12.0),
            Color::rgba(255, 0, 0, 255),
        );
        let pixelate = Shape::new(ShapeKind::Blur {
            start: point(0.0, 0.0),
            end: point(12.0, 12.0),
        });
        let result = render_document(&original, &[marker, pixelate]).unwrap();
        let pixelated = pixelate_region(
            &original,
            PixelateRect {
                x: 0,
                y: 0,
                width: 12,
                height: 12,
            },
            PIXELATE_BLOCK_SIZE,
        )
        .unwrap();
        let replaced = (5 * 24 + 5) * 4;
        assert_eq!(
            &result.pixels[replaced..replaced + 4],
            &pixelated.pixels[replaced..replaced + 4]
        );
        let untouched = (20 * 24 + 20) * 4;
        assert_eq!(
            &result.pixels[untouched..untouched + 4],
            &original.pixels[untouched..untouched + 4]
        );
        assert_eq!(original, before);
    }

    #[test]
    fn blur_coordinates_are_finite_and_clipped_to_image() {
        assert_eq!(
            clipped_region(10, 10, -5.0, 2.2, 4.1, 12.0).unwrap(),
            Some(PixelateRect {
                x: 0,
                y: 2,
                width: 5,
                height: 8
            })
        );
        assert_eq!(
            clipped_region(10, 10, 12.0, 12.0, 14.0, 14.0).unwrap(),
            None
        );
        assert!(clipped_region(10, 10, f32::NAN, 0.0, 1.0, 1.0).is_err());
    }
}
