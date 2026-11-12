//! Bounded Gaussian-like blur for straight-alpha BGRA images.
//!
//! Three separable box filters approximate a Gaussian in linear time. Sampling
//! clamps to the selected rectangle: no pixels outside it can enter the result.
//! The caller MUST supply a source with all document redactions already applied,
//! regardless of annotation order, and reapply them to the final document.

use crate::{
    pixelate::PixelateRect,
    resource_budget::{AdmissionClass, BudgetError, BudgetLease, ResourceBudget},
    stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION},
};

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_WORK_BYTES: usize = 256 * 1024 * 1024;
const MAX_RADIUS: usize = 64;
/// Edge of the tiles used when a region exceeds the single-pass work limit.
const TILE_CORE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlurError {
    InvalidImage,
    InvalidRegion,
    InvalidRadius,
    RegionTooLarge,
    AllocationFailed,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum BlurLayerError {
    Blur(BlurError),
    BudgetRejected(BudgetError),
}

impl From<BlurError> for BlurLayerError {
    fn from(error: BlurError) -> Self {
        Self::Blur(error)
    }
}

// Keep legacy blur diagnostics intact while distinguishing shared-pool refusal.
impl std::fmt::Debug for BlurLayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blur(error) => std::fmt::Debug::fmt(error, f),
            Self::BudgetRejected(error) => f.debug_tuple("BudgetRejected").field(error).finish(),
        }
    }
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
    let count = validate_request(source, region, radius)?;
    render_validated(source, region, radius, count, None).map_err(|error| match error {
        BlurLayerError::Blur(error) => error,
        // With no lease the kernel cannot invoke budget admission.
        BlurLayerError::BudgetRejected(_) => unreachable!("unbudgeted Gaussian admission"),
    })
}

/// Pixels remain charged while the document copies the blurred selection.
/// Fields drop in declaration order: pixels before their lease.
pub(crate) struct BlurLayer {
    image: Image,
    pixel_lease: Option<BudgetLease>,
}

impl BlurLayer {
    pub(crate) fn image(&self) -> &Image {
        if let Some(lease) = &self.pixel_lease {
            debug_assert_eq!(lease.bytes(), self.image.pixels.capacity());
        }
        &self.image
    }
}

/// Existing-document work shares the caller's pool and may use its headroom.
/// None preserves standalone rendering without introducing another budget.
pub(crate) fn gaussian_blur_layer(
    source: &Image,
    region: PixelateRect,
    radius: usize,
    budget: Option<&ResourceBudget>,
) -> Result<BlurLayer, BlurLayerError> {
    let count = validate_request(source, region, radius)?;
    // Admission is owned outside the kernel: all scratch drops before this
    // lease on every error/unwind, and before shrinking to the returned pixels.
    let mut pixel_lease = budget
        .map(|budget| {
            budget
                .try_acquire(
                    {
                        let scratch = scratch_pixels(region, radius, count, TILE_CORE);
                        work_bytes(scratch, scratch, count * 4)?
                    },
                    AdmissionClass::Reserved,
                )
                .map_err(BlurLayerError::BudgetRejected)
        })
        .transpose()?;
    let image = render_validated(source, region, radius, count, pixel_lease.as_mut())?;
    if let Some(lease) = &mut pixel_lease {
        lease
            .try_resize(image.pixels.capacity())
            .map_err(BlurLayerError::BudgetRejected)?;
    }
    Ok(BlurLayer { image, pixel_lease })
}

fn validate_request(
    source: &Image,
    region: PixelateRect,
    radius: usize,
) -> Result<usize, BlurError> {
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
    // The region lies inside a valid image, so its four-byte result fits.
    // Scratch memory is bounded separately, see `scratch_pixels`.
    if count
        .checked_mul(4)
        .filter(|&n| n <= MAX_IMAGE_BYTES)
        .is_none()
    {
        return Err(BlurError::RegionTooLarge);
    }
    Ok(count)
}

/// Two `[u32; 4]` buffers and the four-byte result fit the single-pass limit.
fn fits_single_pass(count: usize) -> bool {
    count
        .checked_mul(36)
        .is_some_and(|bytes| bytes <= MAX_WORK_BYTES)
}

/// Elements in each of the two scratch buffers. Small regions are filtered in
/// one piece. Larger ones are filtered tile by tile, so the scratch buffers
/// stay bounded however large the selection is (a blur over a whole 4K
/// screenshot used to fail the complete document render).
fn scratch_pixels(region: PixelateRect, radius: usize, count: usize, tile_core: usize) -> usize {
    if fits_single_pass(count) && tile_core == TILE_CORE {
        return count;
    }
    let extent = tile_core + 2 * tile_margin(radius);
    region.width.min(extent) * region.height.min(extent)
}

/// Three box passes per axis let a wrong value travel `radius` pixels each.
/// Context this wide around a tile makes its core identical to the single pass.
fn tile_margin(radius: usize) -> usize {
    radius * 3
}

// Capacity is in elements, including for the two 16-byte work vectors.
fn work_bytes(a: usize, b: usize, pixels: usize) -> Result<usize, BlurError> {
    a.checked_mul(std::mem::size_of::<[u32; 4]>())
        .and_then(|bytes| {
            b.checked_mul(std::mem::size_of::<[u32; 4]>())
                .and_then(|other| bytes.checked_add(other))
        })
        .and_then(|bytes| {
            pixels
                .checked_mul(std::mem::size_of::<u8>())
                .and_then(|other| bytes.checked_add(other))
        })
        .ok_or(BlurError::RegionTooLarge)
}

fn reconcile_work_capacity(
    capacities: [usize; 3],
    lease: Option<&mut BudgetLease>,
) -> Result<(), BlurLayerError> {
    let bytes = work_bytes(capacities[0], capacities[1], capacities[2])?;
    if let Some(lease) = lease {
        lease
            .try_resize(bytes)
            .map_err(BlurLayerError::BudgetRejected)?;
    }
    #[cfg(test)]
    WORK_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow_mut().as_mut() {
            observer(capacities);
        }
    });
    Ok(())
}

// A test-only hook observes live reservations and can unwind at each stage.
#[cfg(test)]
type WorkObserver = Box<dyn FnMut([usize; 3])>;
#[cfg(test)]
thread_local! {
    static WORK_OBSERVER: std::cell::RefCell<Option<WorkObserver>> = const {
        std::cell::RefCell::new(None)
    };
}

fn render_validated(
    source: &Image,
    region: PixelateRect,
    radius: usize,
    count: usize,
    mut lease: Option<&mut BudgetLease>,
) -> Result<Image, BlurLayerError> {
    // Validation has already checked all region arithmetic.
    if !fits_single_pass(count) {
        return render_tiled(source, region, radius, count, TILE_CORE, lease);
    }
    let right = region.x + region.width;
    let bottom = region.y + region.height;
    let pixel_bytes = count * 4;
    let mut a = Vec::new();
    a.try_reserve_exact(count)
        .map_err(|_| BlurError::AllocationFailed)?;
    reconcile_work_capacity([a.capacity(), count, pixel_bytes], lease.as_deref_mut())?;
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
    reconcile_work_capacity(
        [a.capacity(), b.capacity(), pixel_bytes],
        lease.as_deref_mut(),
    )?;
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
        .try_reserve_exact(pixel_bytes)
        .map_err(|_| BlurError::AllocationFailed)?;
    reconcile_work_capacity([a.capacity(), b.capacity(), pixels.capacity()], lease)?;
    for p in a {
        pixels.extend_from_slice(&straight_pixel(p));
    }
    Image::new(region.width, region.height, pixels)
        .map_err(|_| BlurLayerError::Blur(BlurError::InvalidRegion))
}

/// Premultiplied sums back to straight alpha; hidden colours are discarded.
fn straight_pixel(p: [u32; 4]) -> [u8; 4] {
    let alpha = (p[3] + 127) / 255;
    if alpha == 0 || p[3] == 0 {
        return [0, 0, 0, 0];
    }
    let channel = |value: u32| {
        ((u64::from(value) * 255 + u64::from(p[3]) / 2) / u64::from(p[3])).min(255) as u8
    };
    [
        channel(p[0]),
        channel(p[1]),
        channel(p[2]),
        alpha.min(255) as u8,
    ]
}

/// Same result as the single pass, computed tile by tile. Each tile is filtered
/// together with `tile_margin` pixels of context, clipped to the region, so
/// sampling still clamps only at the edges of the selection itself.
fn render_tiled(
    source: &Image,
    region: PixelateRect,
    radius: usize,
    count: usize,
    tile_core: usize,
    mut lease: Option<&mut BudgetLease>,
) -> Result<Image, BlurLayerError> {
    let pixel_bytes = count * 4;
    let scratch = scratch_pixels(region, radius, count, tile_core);
    let margin = tile_margin(radius);
    let mut a: Vec<[u32; 4]> = Vec::new();
    a.try_reserve_exact(scratch)
        .map_err(|_| BlurError::AllocationFailed)?;
    reconcile_work_capacity([a.capacity(), scratch, pixel_bytes], lease.as_deref_mut())?;
    let mut b: Vec<[u32; 4]> = Vec::new();
    b.try_reserve_exact(scratch)
        .map_err(|_| BlurError::AllocationFailed)?;
    reconcile_work_capacity(
        [a.capacity(), b.capacity(), pixel_bytes],
        lease.as_deref_mut(),
    )?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(pixel_bytes)
        .map_err(|_| BlurError::AllocationFailed)?;
    reconcile_work_capacity([a.capacity(), b.capacity(), pixels.capacity()], lease)?;
    pixels.resize(pixel_bytes, 0u8);

    // Tile coordinates are relative to the region.
    for core_y in (0..region.height).step_by(tile_core) {
        let core_bottom = (core_y + tile_core).min(region.height);
        let top = core_y.saturating_sub(margin);
        let height = (core_bottom + margin).min(region.height) - top;
        for core_x in (0..region.width).step_by(tile_core) {
            let core_right = (core_x + tile_core).min(region.width);
            let left = core_x.saturating_sub(margin);
            let width = (core_right + margin).min(region.width) - left;
            a.clear();
            for y in top..top + height {
                let row = ((region.y + y) * source.width + region.x + left) * 4;
                for p in source.pixels[row..row + width * 4].as_chunks::<4>().0 {
                    let alpha = u32::from(p[3]);
                    a.push([
                        u32::from(p[0]) * alpha,
                        u32::from(p[1]) * alpha,
                        u32::from(p[2]) * alpha,
                        alpha * 255,
                    ]);
                }
            }
            b.clear();
            b.resize(width * height, [0u32; 4]);
            for _ in 0..3 {
                for y in 0..height {
                    box_line(&a, &mut b, y * width, 1, width, radius);
                }
                for x in 0..width {
                    box_line(&b, &mut a, x, width, height, radius);
                }
            }
            for y in core_y..core_bottom {
                let from = (y - top) * width + (core_x - left);
                let to = (y * region.width + core_x) * 4;
                let tile_row = &a[from..from + (core_right - core_x)];
                let out_row = &mut pixels[to..to + (core_right - core_x) * 4];
                for (out, p) in out_row.as_chunks_mut::<4>().0.iter_mut().zip(tile_row) {
                    *out = straight_pixel(*p);
                }
            }
        }
    }
    Image::new(region.width, region.height, pixels)
        .map_err(|_| BlurLayerError::Blur(BlurError::InvalidRegion))
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

    struct ObserveWork;

    impl ObserveWork {
        fn install(observer: impl FnMut([usize; 3]) + 'static) -> Self {
            WORK_OBSERVER.with(|slot| {
                assert!(slot.borrow_mut().replace(Box::new(observer)).is_none());
            });
            Self
        }
    }

    impl Drop for ObserveWork {
        fn drop(&mut self) {
            WORK_OBSERVER.with(|slot| *slot.borrow_mut() = None);
        }
    }

    #[test]
    fn budget_counts_all_live_capacities_and_retains_only_result_after_kernel() {
        use std::{cell::Cell, rc::Rc};
        let source = image(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 0]);
        let expected = gaussian_blur_region(&source, rect(0, 0, 2, 1), 64).unwrap();
        let budget = ResourceBudget::new(128, 128).unwrap();
        let stages = Rc::new(Cell::new(0));
        let observer_budget = budget.clone();
        let observed_stages = Rc::clone(&stages);
        let observer = ObserveWork::install(move |capacities| {
            observed_stages.set(observed_stages.get() + 1);
            assert_eq!(
                observer_budget.snapshot().used_bytes,
                work_bytes(capacities[0], capacities[1], capacities[2]).unwrap()
            );
            assert!(observer_budget
                .try_acquire(1, AdmissionClass::Normal)
                .is_err());
        });
        // Both work vectors and result coexist at the final observed stage.
        let layer = gaussian_blur_layer(&source, rect(0, 0, 2, 1), 64, Some(&budget)).unwrap();
        assert_eq!(stages.get(), 3);
        assert_eq!(layer.image(), &expected);
        assert_eq!(
            budget.snapshot().used_bytes,
            layer.image().pixels.capacity()
        );
        drop(observer);
        let spare = budget.snapshot().ceiling_bytes - budget.snapshot().used_bytes;
        let blocker = budget.try_acquire(spare, AdmissionClass::Reserved).unwrap();
        assert!(matches!(
            gaussian_blur_layer(&source, rect(0, 0, 2, 1), 1, Some(&budget)),
            Err(BlurLayerError::BudgetRejected(
                BudgetError::LimitExceeded { .. }
            ))
        ));
        assert_eq!(budget.snapshot().used_bytes, 128);
        drop(blocker);
        drop(layer);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn full_peak_is_required_and_standalone_and_optional_paths_remain_equivalent() {
        let source = image(1, 1, vec![12, 34, 56, 127]);
        let small = ResourceBudget::new(35, 0).unwrap();
        assert!(matches!(
            gaussian_blur_layer(&source, rect(0, 0, 1, 1), 1, Some(&small)),
            Err(BlurLayerError::BudgetRejected(
                BudgetError::LimitExceeded { .. }
            ))
        ));
        assert_eq!(small.snapshot().used_bytes, 0);
        let exact = ResourceBudget::new(36, 36).unwrap();
        let layer = gaussian_blur_layer(&source, rect(0, 0, 1, 1), 1, Some(&exact)).unwrap();
        assert_eq!(layer.image().pixels, source.pixels);
        assert_eq!(exact.snapshot().used_bytes, layer.image().pixels.capacity());
        drop(layer);
        assert_eq!(exact.snapshot().used_bytes, 0);
        let unbudgeted = gaussian_blur_layer(&source, rect(0, 0, 1, 1), 64, None).unwrap();
        assert_eq!(
            unbudgeted.image(),
            &gaussian_blur_region(&source, rect(0, 0, 1, 1), 64).unwrap()
        );
    }

    #[test]
    fn capacity_reconciliation_counts_elements_and_rejects_excess_atomically() {
        let a = Vec::<[u32; 4]>::with_capacity(3);
        let b = Vec::<[u32; 4]>::with_capacity(5);
        let pixels = Vec::<u8>::with_capacity(17);
        let capacities = [a.capacity(), b.capacity(), pixels.capacity()];
        let total = a.capacity() * std::mem::size_of::<[u32; 4]>()
            + b.capacity() * std::mem::size_of::<[u32; 4]>()
            + pixels.capacity() * std::mem::size_of::<u8>();
        let budget = ResourceBudget::new(total - 1, 0).unwrap();
        let mut lease = budget.try_acquire(36, AdmissionClass::Reserved).unwrap();
        assert!(matches!(
            reconcile_work_capacity(capacities, Some(&mut lease)),
            Err(BlurLayerError::BudgetRejected(
                BudgetError::LimitExceeded { .. }
            ))
        ));
        assert_eq!(lease.bytes(), 36);
        assert_eq!(budget.snapshot().used_bytes, 36);
        assert_eq!(
            work_bytes(capacities[0], capacities[1], capacities[2]),
            Ok(total)
        );
        for capacities in [[usize::MAX, 0, 0], [0, usize::MAX, 0], [1, 1, usize::MAX]] {
            assert_eq!(
                reconcile_work_capacity(capacities, Some(&mut lease)),
                Err(BlurLayerError::Blur(BlurError::RegionTooLarge))
            );
            assert_eq!(lease.bytes(), 36);
        }
        drop((a, b, pixels));
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn scratch_admission_releases_on_each_stage_unwind_and_keeps_existing_lease() {
        use std::{cell::Cell, rc::Rc};
        let source = image(2, 2, [123, 45, 67, 0].repeat(4));
        for unwind_stage in 1..=3 {
            let budget = ResourceBudget::new(256, 0).unwrap();
            let existing = budget.try_acquire(7, AdmissionClass::Normal).unwrap();
            let stage = Rc::new(Cell::new(0));
            let observer_budget = budget.clone();
            let observer = ObserveWork::install(move |capacities| {
                stage.set(stage.get() + 1);
                assert_eq!(
                    observer_budget.snapshot().used_bytes,
                    7 + work_bytes(capacities[0], capacities[1], capacities[2]).unwrap()
                );
                if stage.get() == unwind_stage {
                    panic!("cancel Gaussian stage {unwind_stage}");
                }
            });
            let result = std::panic::catch_unwind(|| {
                let _layer = gaussian_blur_layer(&source, rect(0, 0, 2, 2), 1, Some(&budget));
            });
            assert!(result.is_err());
            drop(observer);
            assert_eq!(budget.snapshot().used_bytes, 7);
            drop(existing);
            assert_eq!(budget.snapshot().used_bytes, 0);
        }
    }

    fn noise(width: usize, height: usize) -> Image {
        // Deterministic content with varying colour and alpha, including zero.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height * 4 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            pixels.push((state >> 24) as u8);
        }
        for pixel in pixels.as_chunks_mut::<4>().0.iter_mut().step_by(7) {
            pixel[3] = 0;
        }
        image(width, height, pixels)
    }

    #[test]
    fn tiled_blur_is_identical_to_the_single_pass() {
        let source = noise(97, 83);
        for region in [rect(0, 0, 97, 83), rect(5, 9, 61, 70), rect(40, 0, 1, 83)] {
            for radius in [1, 2, 5] {
                let count = region.width * region.height;
                let whole = gaussian_blur_region(&source, region, radius).unwrap();
                // Tiles smaller than, equal to and larger than their context.
                for tile_core in [1, 7, 16, 64] {
                    let tiled =
                        render_tiled(&source, region, radius, count, tile_core, None).unwrap();
                    assert!(
                        tiled == whole,
                        "region {region:?}, radius {radius}, tile {tile_core}"
                    );
                }
            }
        }
    }

    #[test]
    fn full_4k_selection_is_blurred_with_bounded_scratch() {
        // 3840x2160 needs about 285 MiB in one piece and used to be rejected.
        let (width, height) = (3840, 2160);
        let count = width * height;
        assert!(!fits_single_pass(count));
        let region = rect(0, 0, width, height);
        let scratch = scratch_pixels(region, MAX_RADIUS, count, TILE_CORE);
        assert_eq!(scratch, (TILE_CORE + 6 * MAX_RADIUS).pow(2));
        assert!(work_bytes(scratch, scratch, 0).unwrap() <= 64 * 1024 * 1024);

        let source = image(width, height, [40, 80, 120, 255].repeat(count));
        let budget = ResourceBudget::new(256 * 1024 * 1024, 0).unwrap();
        let layer = gaussian_blur_layer(&source, region, 8, Some(&budget)).unwrap();
        assert_eq!(layer.image().pixels.len(), count * 4);
        assert!(layer
            .image()
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [40, 80, 120, 255]));
        assert_eq!(budget.snapshot().used_bytes, count * 4);
        drop(layer);
        assert_eq!(budget.snapshot().used_bytes, 0);
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
