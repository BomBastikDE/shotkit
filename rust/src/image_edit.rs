//! Non-destructive image operations over tightly packed BGRA [`Image`] pixels.
//!
//! Cropping validates a non-empty rectangle wholly inside the source and copies
//! bytes exactly, including alpha. Framing returns a new image, preserving the
//! original pixels exactly. An optional deterministic soft shadow is rendered
//! into transparent padding beneath the frame; it uses separable box coverage,
//! not a photo-realistic Gaussian blur.

use crate::{
    alpha_composite::source_over,
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
    InvalidTornEdge,
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
            Self::InvalidTornEdge => f.write_str(
                "Rissrand nicht möglich: keine Seite gewählt oder das Bild ist dafür zu klein",
            ),
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
/// Image sides that receive a torn edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TornSides {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl TornSides {
    pub const BOTTOM: Self = Self {
        top: false,
        right: false,
        bottom: true,
        left: false,
    };
    pub const TOP_AND_BOTTOM: Self = Self {
        top: true,
        right: false,
        bottom: true,
        left: false,
    };
    pub const LEFT_AND_RIGHT: Self = Self {
        top: false,
        right: true,
        bottom: false,
        left: true,
    };
    pub const ALL: Self = Self {
        top: true,
        right: true,
        bottom: true,
        left: true,
    };

    fn any(self) -> bool {
        self.top || self.right || self.bottom || self.left
    }
}

/// A "torn paper" edge on selected sides, optionally with a shadow that
/// follows the torn outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TornEdgeOptions {
    pub sides: TornSides,
    /// Maximum depth of the tear, in pixels.
    pub depth: usize,
    pub shadow: Option<ShadowOptions>,
}

/// Maximum tear depth; deeper tears would remove meaningful content.
pub const MAX_TORN_DEPTH: usize = 64;
/// Distance between the random knots of the torn outline, in pixels.
const TORN_KNOT_SPACING: usize = 9;

/// Apply a torn edge without modifying `source`. The torn band becomes
/// transparent with an anti-aliased outline; the result is deterministic, so
/// preview, undo and export always agree. Pixels deeper than `depth` from a
/// torn side keep their exact BGRA values.
pub fn add_torn_edge(source: &Image, options: TornEdgeOptions) -> Result<Image, ImageEditError> {
    let layout = torn_layout(source, options)?;
    let pixels = reserve_pixels(layout.byte_len)?;
    render_torn_edge(source, options, layout, pixels)
}

/// Budgeted variant of [`add_torn_edge`]; same admission rules as
/// [`add_frame_with_budget`].
pub fn add_torn_edge_with_budget(
    source: &Image,
    options: TornEdgeOptions,
    budget: &ResourceBudget,
) -> Result<ImageHandle, ImageEditError> {
    add_torn_edge_with_budget_using(source, options, budget, reserve_pixels)
}

fn add_torn_edge_with_budget_using(
    source: &Image,
    options: TornEdgeOptions,
    budget: &ResourceBudget,
    allocate: impl FnOnce(usize) -> Result<Vec<u8>, ImageEditError>,
) -> Result<ImageHandle, ImageEditError> {
    let layout = torn_layout(source, options)?;
    let mut lease = budget
        .try_acquire(layout.byte_len, AdmissionClass::Reserved)
        .map_err(ImageEditError::Budget)?;
    let pixels = allocate(layout.byte_len)?;
    lease
        .try_resize(pixels.capacity())
        .map_err(ImageEditError::Budget)?;
    let image = render_torn_edge(source, options, layout, pixels)?;
    Ok(ImageHandle::from_imported(Arc::new(
        TrackedImage::from_reserved(image, lease),
    )))
}

/// The frame options with the same output geometry as a torn edge: no frame,
/// the torn edge's shadow. Documents use it to move annotations consistently.
pub fn torn_edge_frame_equivalent(options: TornEdgeOptions) -> FrameOptions {
    FrameOptions {
        width: 0,
        color: Color::rgba(0, 0, 0, 0),
        shadow: options.shadow,
    }
}

fn torn_layout(source: &Image, options: TornEdgeOptions) -> Result<FrameLayout, ImageEditError> {
    validate_source(source)?;
    let minimum = options
        .depth
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or(ImageEditError::InvalidTornEdge)?;
    let vertical = options.sides.top || options.sides.bottom;
    let horizontal = options.sides.left || options.sides.right;
    if !options.sides.any()
        || options.depth == 0
        || options.depth > MAX_TORN_DEPTH
        || (vertical && source.height < minimum)
        || (horizontal && source.width < minimum)
    {
        return Err(ImageEditError::InvalidTornEdge);
    }
    frame_layout(source, torn_edge_frame_equivalent(options))
}

fn splitmix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Depth of the tear at position `t` along one side: linear between
/// pseudo-random knots, always between 35 % and 100 % of `depth`.
fn tear_depth(side: u64, t: f32, depth: usize) -> f32 {
    let knot = |k: u64| -> f32 {
        let unit =
            (splitmix(side.wrapping_mul(0x1000_0001) ^ k) >> 40) as f32 / (1u64 << 24) as f32;
        depth as f32 * (0.35 + 0.65 * unit)
    };
    let position = t.max(0.0) / TORN_KNOT_SPACING as f32;
    let k = position.floor();
    let fraction = position - k;
    let k = k as u64;
    knot(k) * (1.0 - fraction) + knot(k + 1) * fraction
}

/// Coverage (0..=1) of the pixel centre (x, y) inside the torn outline.
fn torn_coverage(x: usize, y: usize, width: usize, height: usize, options: TornEdgeOptions) -> f32 {
    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
    let reach = options.depth as f32 + 1.0;
    let mut coverage = 1.0_f32;
    let mut side = |enabled: bool, id: u64, distance: f32, along: f32| {
        if enabled && distance < reach {
            let inside = distance - tear_depth(id, along, options.depth) + 0.5;
            coverage = coverage.min(inside.clamp(0.0, 1.0));
        }
    };
    side(options.sides.top, 1, cy, cx);
    side(options.sides.right, 2, width as f32 - cx, cy);
    side(options.sides.bottom, 3, height as f32 - cy, cx);
    side(options.sides.left, 4, cx, cy);
    coverage
}

/// One straight-alpha source pixel after the torn mask.
fn torn_pixel(source: &Image, x: usize, y: usize, options: TornEdgeOptions) -> [u8; 4] {
    let offset = (y * source.width + x) * 4;
    let mut pixel: [u8; 4] = source.pixels[offset..offset + 4]
        .try_into()
        .expect("four-byte pixel");
    let coverage = torn_coverage(x, y, source.width, source.height, options);
    if coverage < 1.0 {
        pixel[3] = (f32::from(pixel[3]) * coverage).round() as u8;
    }
    pixel
}

// Geometry and byte products were checked by torn_layout. Besides the admitted
// output, only one row/column of blur sums is allocated.
fn render_torn_edge(
    source: &Image,
    options: TornEdgeOptions,
    layout: FrameLayout,
    mut pixels: Vec<u8>,
) -> Result<Image, ImageEditError> {
    let FrameLayout {
        radius,
        content_x,
        content_y,
        out_width,
        out_height,
        byte_len,
        ..
    } = layout;
    if !pixels.is_empty() || pixels.capacity() < byte_len {
        return Err(ImageEditError::AllocationFailed);
    }
    pixels.resize(byte_len, 0);
    if let Some(shadow) = options.shadow.filter(|s| s.opacity > 0) {
        // 1. Shadow silhouette: the torn alpha, shifted by the shadow offset,
        //    written into the (still empty) output alpha channel.
        let sx = content_x as i64 + i64::from(shadow.offset_x);
        let sy = content_y as i64 + i64::from(shadow.offset_y);
        for y in 0..source.height {
            for x in 0..source.width {
                let alpha = torn_pixel(source, x, y, options)[3];
                let (ox, oy) = (sx + x as i64, sy + y as i64);
                // frame_layout reserved room for offset and radius on all sides.
                let offset = (oy as usize * out_width + ox as usize) * 4;
                pixels[offset + 3] = alpha;
            }
        }
        // 2. Separable box blur of the alpha channel, then shadow opacity.
        if radius > 0 {
            let mut line = vec![0u32; out_width.max(out_height)];
            box_blur_alpha(&mut pixels, out_width, out_height, radius, &mut line, true);
            box_blur_alpha(&mut pixels, out_width, out_height, radius, &mut line, false);
        }
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = ((u32::from(pixel[3]) * u32::from(shadow.opacity) + 127) / 255) as u8;
        }
    }
    // 3. The torn image over the (black) shadow.
    for y in 0..source.height {
        for x in 0..source.width {
            let pixel = torn_pixel(source, x, y, options);
            let offset = ((content_y + y) * out_width + content_x + x) * 4;
            let below: [u8; 4] = pixels[offset..offset + 4]
                .try_into()
                .expect("four-byte pixel");
            let result = if below[3] == 0 {
                pixel
            } else {
                source_over(below, pixel, 255)
            };
            pixels[offset..offset + 4].copy_from_slice(&result);
        }
    }
    Ok(Image {
        width: out_width,
        height: out_height,
        pixels,
    })
}

/// Box blur of the alpha channel along rows (`horizontal`) or columns, with
/// zero outside the image. `line` must hold at least one row or column.
fn box_blur_alpha(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    line: &mut [u32],
    horizontal: bool,
) {
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let index = |line_no: usize, i: usize| -> usize {
        if horizontal {
            (line_no * width + i) * 4 + 3
        } else {
            (i * width + line_no) * 4 + 3
        }
    };
    let kernel = (2 * radius + 1) as u32;
    for line_no in 0..lines {
        for (i, value) in line.iter_mut().take(length).enumerate() {
            *value = u32::from(pixels[index(line_no, i)]);
        }
        let mut sum: u32 = line[..length.min(radius + 1)].iter().sum();
        for i in 0..length {
            pixels[index(line_no, i)] = ((sum + kernel / 2) / kernel) as u8;
            if i + radius + 1 < length {
                sum += line[i + radius + 1];
            }
            if i >= radius {
                sum -= line[i - radius];
            }
        }
    }
}

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

    fn torn_fixture(width: usize, height: usize) -> Image {
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[(x * 7) as u8, (y * 5) as u8, ((x + y) * 3) as u8, 255]);
            }
        }
        Image::new(width, height, pixels).unwrap()
    }

    fn torn(sides: TornSides, shadow: Option<ShadowOptions>) -> TornEdgeOptions {
        TornEdgeOptions {
            sides,
            depth: 12,
            shadow,
        }
    }

    fn px(image: &Image, x: usize, y: usize) -> [u8; 4] {
        let o = (y * image.width + x) * 4;
        image.pixels[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn torn_bottom_edge_changes_only_the_bottom_band() {
        let source = torn_fixture(60, 50);
        let output = add_torn_edge(&source, torn(TornSides::BOTTOM, None)).unwrap();
        assert_eq!((output.width, output.height), (60, 50));
        // Everything more than `depth` above the bottom keeps its exact bytes.
        let band_start = 50 - 13;
        assert_eq!(
            &output.pixels[..band_start * 60 * 4],
            &source.pixels[..band_start * 60 * 4]
        );
        // The bottom row lies outside every tear (minimum depth 35 %).
        for x in 0..60 {
            assert_eq!(px(&output, x, 49)[3], 0, "bottom row x={x}");
        }
        // Colour channels are never altered, only alpha.
        for y in band_start..50 {
            for x in 0..60 {
                assert_eq!(&px(&output, x, y)[..3], &px(&source, x, y)[..3]);
            }
        }
        // The outline is irregular: opaque and transparent pixels share rows.
        let alphas: Vec<u8> = (0..60).map(|x| px(&output, x, 50 - 6)[3]).collect();
        assert!(alphas.contains(&255) || alphas.contains(&0));
        let band: Vec<u8> = (band_start..50)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .map(|(x, y)| px(&output, x, y)[3])
            .collect();
        assert!(band.contains(&255) && band.contains(&0));
        assert!(
            band.iter().any(|a| *a > 0 && *a < 255),
            "anti-aliased outline"
        );
    }

    #[test]
    fn torn_edge_is_deterministic_and_respects_the_chosen_sides() {
        let source = torn_fixture(48, 48);
        for sides in [
            TornSides::BOTTOM,
            TornSides::TOP_AND_BOTTOM,
            TornSides::LEFT_AND_RIGHT,
            TornSides::ALL,
        ] {
            let first = add_torn_edge(&source, torn(sides, None)).unwrap();
            assert_eq!(first, add_torn_edge(&source, torn(sides, None)).unwrap());
            // Centre untouched; a torn side's outermost line is transparent,
            // an untouched side's outermost line keeps its pixels.
            assert_eq!(px(&first, 24, 24), px(&source, 24, 24));
            let edge = |x: usize, y: usize| px(&first, x, y)[3];
            assert_eq!(edge(24, 0) == 0, sides.top);
            assert_eq!(edge(24, 47) == 0, sides.bottom);
            assert_eq!(edge(0, 24) == 0, sides.left);
            assert_eq!(edge(47, 24) == 0, sides.right);
        }
    }

    #[test]
    fn torn_edge_rejects_no_side_bad_depth_and_too_small_images() {
        let source = torn_fixture(30, 30);
        let no_side = TornEdgeOptions {
            sides: TornSides::default(),
            depth: 12,
            shadow: None,
        };
        assert_eq!(
            add_torn_edge(&source, no_side),
            Err(ImageEditError::InvalidTornEdge)
        );
        for depth in [0, MAX_TORN_DEPTH + 1] {
            let options = TornEdgeOptions {
                depth,
                ..torn(TornSides::BOTTOM, None)
            };
            assert_eq!(
                add_torn_edge(&source, options),
                Err(ImageEditError::InvalidTornEdge)
            );
        }
        // 2 * depth + 1 = 25 rows are needed for a top/bottom tear.
        let short = torn_fixture(100, 24);
        assert_eq!(
            add_torn_edge(&short, torn(TornSides::BOTTOM, None)),
            Err(ImageEditError::InvalidTornEdge)
        );
        // A left/right tear only needs the width.
        assert!(add_torn_edge(&short, torn(TornSides::LEFT_AND_RIGHT, None)).is_ok());
        assert!(add_torn_edge(&torn_fixture(25, 3), torn(TornSides::LEFT_AND_RIGHT, None)).is_ok());
    }

    #[test]
    fn torn_edge_shadow_follows_the_outline_and_keeps_the_content() {
        let source = torn_fixture(60, 50);
        let shadow = ShadowOptions {
            offset_x: 6,
            offset_y: 6,
            blur_radius: 10,
            opacity: 112,
        };
        let plain = add_torn_edge(&source, torn(TornSides::BOTTOM, None)).unwrap();
        let output = add_torn_edge(&source, torn(TornSides::BOTTOM, Some(shadow))).unwrap();
        // Same geometry as a frame of width zero with this shadow.
        let frame = add_frame(
            &source,
            torn_edge_frame_equivalent(torn(TornSides::BOTTOM, Some(shadow))),
        )
        .unwrap();
        assert_eq!((output.width, output.height), (frame.width, frame.height));
        assert_eq!((output.width, output.height), (60 + 10 + 16, 50 + 10 + 16));
        // Opaque content is unchanged at the content offset (10, 10).
        for (x, y) in [(0, 0), (30, 20), (59, 30)] {
            assert_eq!(px(&output, 10 + x, 10 + y), px(&plain, x, y));
        }
        // The shadow is black and visible right of the image, but not far
        // above-left of it.
        let right = px(&output, 10 + 60 + 3, 10 + 25);
        assert_eq!(&right[..3], &[0, 0, 0]);
        assert!(right[3] > 0 && right[3] <= 112);
        assert_eq!(px(&output, 0, 0)[3], 0);
        // Below the torn bottom the shadow is torn too: along the bottom
        // shadow band some columns are darker than others.
        let row = 10 + 50 + 2;
        let alphas: Vec<u8> = (15..60).map(|x| px(&output, x, row)[3]).collect();
        let (min, max) = (alphas.iter().min().unwrap(), alphas.iter().max().unwrap());
        assert!(max > min, "shadow follows the irregular outline");
    }

    #[test]
    fn budgeted_torn_edge_matches_legacy_and_releases_on_drop() {
        let source = torn_fixture(40, 40);
        let options = torn(TornSides::ALL, None);
        let expected = add_torn_edge(&source, options).unwrap();
        let bytes = expected.pixels.len();
        let budget = ResourceBudget::new(bytes, 0).unwrap();
        let output = add_torn_edge_with_budget(&source, options, &budget).unwrap();
        assert_eq!(output.image(), &expected);
        assert!(budget.snapshot().used_bytes >= bytes);
        assert!(matches!(
            add_torn_edge_with_budget(&source, options, &budget),
            Err(ImageEditError::Budget(_))
        ));
        drop(output);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
}
