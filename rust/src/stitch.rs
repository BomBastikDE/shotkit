//! Pure image rotation and seam composition for scroll capture.
//! Pixels are tightly packed BGRA, top row first. No Windows APIs are used here.
const MAX_LENGTH: usize = 30_000;
const MAX_BYTES: usize = 512 * 1024 * 1024;

/// Dimension supported by the document renderer and its editing operations.
pub const MAX_EDITABLE_IMAGE_DIMENSION: usize = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Clockwise,
    Counterclockwise,
}

/// A rejected rotation retains the original image, including its allocation.
#[derive(Debug)]
pub struct RotationFailure {
    pub image: Image,
    pub error: &'static str,
}

impl Image {
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Result<Self, &'static str> {
        let image = Self {
            width,
            height,
            pixels,
        };
        image.validate()?;
        Ok(image)
    }
    /// Check public fields before using an image in an unsafe native buffer copy.
    pub fn validate(&self) -> Result<(), &'static str> {
        let len = self
            .width
            .checked_mul(self.height)
            .and_then(|n| n.checked_mul(4))
            .ok_or("Bildgröße überläuft")?;
        if self.width == 0 || self.height == 0 || len != self.pixels.len() || len > MAX_BYTES {
            return Err("Ungültige Bildgröße");
        }
        Ok(())
    }

    /// Rotate by 90 degrees, returning the unchanged source if validation or
    /// destination reservation fails. The source stays alive until completion.
    pub fn try_rotate(self, rotation: Rotation) -> Result<Self, RotationFailure> {
        self.try_rotate_with_reserve(rotation, |pixels, additional| {
            pixels
                .try_reserve_exact(additional)
                .map_err(|_| "Nicht genügend Speicher für die Bilddrehung")
        })
    }

    fn try_rotate_with_reserve(
        self,
        rotation: Rotation,
        reserve: impl FnOnce(&mut Vec<u8>, usize) -> Result<(), &'static str>,
    ) -> Result<Self, RotationFailure> {
        if let Err(error) = self.validate() {
            return Err(RotationFailure { image: self, error });
        }
        let mut pixels = Vec::new();
        if let Err(error) = reserve(&mut pixels, self.pixels.len()) {
            return Err(RotationFailure { image: self, error });
        }
        // Reservation precedes initialization, so resize cannot allocate on the
        // production path. OS overcommit can still prevent physical page access.
        pixels.resize(self.pixels.len(), 0);
        let w = self.height;
        for y in 0..self.height {
            for x in 0..self.width {
                let (nx, ny) = match rotation {
                    Rotation::Clockwise => (self.height - 1 - y, x),
                    Rotation::Counterclockwise => (y, self.width - 1 - x),
                };
                let src = (y * self.width + x) * 4;
                let dst = (ny * w + nx) * 4;
                pixels[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        Ok(Self {
            width: self.height,
            height: self.width,
            pixels,
        })
    }

    /// Rotate clockwise by 90 degrees, as in the original rightward capture.
    pub fn clockwise(&self) -> Self {
        self.rotate(true)
    }
    pub fn counterclockwise(&self) -> Self {
        self.rotate(false)
    }
    fn rotate(&self, clockwise: bool) -> Self {
        let mut pixels = vec![0; self.pixels.len()];
        let w = self.height;
        for y in 0..self.height {
            for x in 0..self.width {
                let (nx, ny) = if clockwise {
                    (self.height - 1 - y, x)
                } else {
                    (y, self.width - 1 - x)
                };
                let src = (y * self.width + x) * 4;
                let dst = (ny * w + nx) * 4;
                pixels[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        Self {
            width: self.height,
            height: self.width,
            pixels,
        }
    }
}

/// Holds completed rows and the last frame. `advance` accepts a validated match.
pub struct Stitcher {
    prev: Image,
    rows: Vec<u8>,
    start: usize,
    center: Option<usize>,
}
impl Stitcher {
    pub fn new(frame: Image) -> Result<Self, &'static str> {
        frame.validate()?;
        if frame.height > MAX_LENGTH {
            return Err("Aufnahmebereich überschreitet 30000 Pixel");
        }
        Ok(Self {
            prev: frame,
            rows: Vec::new(),
            start: 0,
            center: None,
        })
    }
    pub fn frame(&self) -> &Image {
        &self.prev
    }
    pub fn length(&self) -> usize {
        self.rows.len() / (self.prev.width * 4) + self.prev.height - self.start
    }
    pub fn advance(
        &mut self,
        next: Image,
        dy: usize,
        top: usize,
        bottom: usize,
    ) -> Result<(), &'static str> {
        self.advance_with_reserve(next, dy, top, bottom, |rows, additional| {
            rows.try_reserve(additional)
                .map_err(|_| "Nicht genügend Speicher für die Scrollaufnahme")
        })
    }

    fn advance_with_reserve(
        &mut self,
        next: Image,
        dy: usize,
        top: usize,
        bottom: usize,
        reserve: impl FnOnce(&mut Vec<u8>, usize) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let h = self.prev.height;
        next.validate()?;
        if next.width != self.prev.width
            || next.height != h
            || dy == 0
            || dy >= h
            || top.checked_add(bottom).is_none_or(|total| total >= h)
        {
            return Err("Ungültiger Bildabgleich");
        }
        let center = self.center.unwrap_or((top + h - bottom) / 2);
        let seam = (center + dy / 2).clamp(self.start, h);
        let next_start = seam.saturating_sub(dy);
        let stride = self.prev.width * 4;
        let new_len = self.rows.len() / stride + seam - self.start + h - next_start;
        let Some(total_bytes) = new_len.checked_mul(stride).filter(|n| *n <= MAX_BYTES) else {
            return Err("Maximale Bildgröße erreicht");
        };
        if new_len > MAX_LENGTH {
            return Err("Maximale Bildgröße erreicht");
        }
        // Reserve the entire accepted result, including the last frame's tail,
        // before changing the stitcher. A failure leaves the previous partial
        // capture finishable without another allocation.
        if !(self.rows.is_empty() && seam == self.start && next_start == 0) {
            let additional = total_bytes - self.rows.len();
            reserve(&mut self.rows, additional)?;
        }
        self.rows
            .extend_from_slice(&self.prev.pixels[self.start * stride..seam * stride]);
        self.start = next_start;
        self.prev = next;
        self.center = Some(center);
        Ok(())
    }
    pub fn finish(mut self) -> Result<Image, &'static str> {
        let stride = self.prev.width * 4;
        let len = self.length();
        if len > MAX_LENGTH
            || len
                .checked_mul(stride)
                .filter(|n| *n <= MAX_BYTES)
                .is_none()
        {
            return Err("Maximale Bildgröße erreicht");
        }
        if self.rows.is_empty() && self.start == 0 {
            return Ok(self.prev);
        }
        // For a valid stitcher, advance already reserved the complete result;
        // this does not allocate on the normal finalization path.
        self.rows
            .try_reserve((len * stride) - self.rows.len())
            .map_err(|_| "Nicht genügend Speicher für die Scrollaufnahme")?;
        self.rows
            .extend_from_slice(&self.prev.pixels[self.start * stride..]);
        Image::new(self.prev.width, len, self.rows)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn validates_public_image_fields_before_native_copy() {
        let image = super::Image {
            width: 2,
            height: 2,
            pixels: vec![0; 15],
        };
        assert!(image.validate().is_err());
        let overflow = super::Image {
            width: usize::MAX,
            height: 2,
            pixels: vec![],
        };
        assert!(overflow.validate().is_err());
        assert!(super::Image::new(2, 2, vec![0; 16])
            .unwrap()
            .validate()
            .is_ok());
    }

    use super::*;
    fn image(w: usize, rows: &[u8]) -> Image {
        let mut p = Vec::new();
        for row in rows {
            for _ in 0..w {
                p.extend_from_slice(&[*row, 0, 0, 255]);
            }
        }
        Image::new(w, rows.len(), p).unwrap()
    }
    #[test]
    fn rotations_are_inverse() {
        let a = image(3, &[1, 2, 3, 4]);
        assert_eq!(a, a.clockwise().counterclockwise());
        assert_eq!(a, a.counterclockwise().clockwise());
    }

    #[test]
    fn fallible_rotations_match_existing_pixels_for_varied_dimensions_and_alpha() {
        for (width, height) in [(1, 1), (2, 2), (3, 3), (2, 3), (3, 2), (1, 5), (6, 1)] {
            let pixels = (0..width * height)
                .flat_map(|i| [i as u8, (i * 3) as u8, (i * 7) as u8, (i * 17) as u8])
                .collect();
            let original = Image::new(width, height, pixels).unwrap();
            let clockwise = original.clone().try_rotate(Rotation::Clockwise).unwrap();
            let counterclockwise = original
                .clone()
                .try_rotate(Rotation::Counterclockwise)
                .unwrap();
            assert_eq!(clockwise, original.clockwise());
            assert_eq!(counterclockwise, original.counterclockwise());
            assert_eq!((clockwise.width, clockwise.height), (height, width));
            assert_eq!(
                clockwise.try_rotate(Rotation::Counterclockwise).unwrap(),
                original
            );
            assert_eq!(
                counterclockwise.try_rotate(Rotation::Clockwise).unwrap(),
                original
            );
        }
    }

    #[test]
    fn fallible_rotations_have_expected_rectangular_pixel_order() {
        let original = Image::new(
            3,
            2,
            (1..=6).flat_map(|i| [i, 20 + i, 40 + i, 60 + i]).collect(),
        )
        .unwrap();
        for (rotation, order) in [
            (Rotation::Clockwise, [4, 1, 5, 2, 6, 3]),
            (Rotation::Counterclockwise, [3, 6, 2, 5, 1, 4]),
        ] {
            let result = original.clone().try_rotate(rotation).unwrap();
            let expected = order
                .into_iter()
                .flat_map(|i| [i, 20 + i, 40 + i, 60 + i])
                .collect::<Vec<_>>();
            assert_eq!(result, Image::new(2, 3, expected).unwrap());
        }
    }

    #[test]
    fn invalid_rotation_preserves_source_before_reserving() {
        for rotation in [Rotation::Clockwise, Rotation::Counterclockwise] {
            for (width, height, len) in [(2, 2, 15), (0, 3, 0), (usize::MAX, 2, 4)] {
                let mut pixels = Vec::with_capacity(len + 16);
                pixels.resize(len, 123);
                let original = Image {
                    width,
                    height,
                    pixels,
                };
                let expected = original.clone();
                let allocation = original.pixels.as_ptr();
                let capacity = original.pixels.capacity();
                let expected_error = original.validate().unwrap_err();
                let failure = original
                    .try_rotate_with_reserve(rotation, |_, _| {
                        panic!("invalid images must not reserve a destination")
                    })
                    .unwrap_err();
                assert_eq!(failure.error, expected_error);
                assert_eq!(failure.image.pixels.as_ptr(), allocation);
                assert_eq!(failure.image.pixels.capacity(), capacity);
                assert_eq!(failure.image, expected);
            }
        }
    }

    #[test]
    fn failed_rotation_reservation_preserves_original_allocation_and_pixels() {
        for rotation in [Rotation::Clockwise, Rotation::Counterclockwise] {
            let mut original = image(3, &[1, 2, 3, 4, 5]);
            original.pixels.reserve(31);
            let expected = original.clone();
            let allocation = original.pixels.as_ptr();
            let capacity = original.pixels.capacity();
            let failure = original
                .try_rotate_with_reserve(rotation, |pixels, additional| {
                    assert!(pixels.is_empty());
                    assert_eq!(pixels.capacity(), 0);
                    assert_eq!(additional, expected.pixels.len());
                    Err("injected reservation failure")
                })
                .unwrap_err();
            assert_eq!(failure.error, "injected reservation failure");
            assert_eq!(failure.image.pixels.as_ptr(), allocation);
            assert_eq!(failure.image.pixels.capacity(), capacity);
            assert_eq!(failure.image, expected);
            // The recovered image remains usable for a later successful retry.
            assert_eq!(
                failure.image.try_rotate(rotation).unwrap(),
                match rotation {
                    Rotation::Clockwise => expected.clockwise(),
                    Rotation::Counterclockwise => expected.counterclockwise(),
                }
            );
        }
    }

    #[test]
    fn seams_reconstruct_content() {
        let mut s = Stitcher::new(image(2, &[0, 1, 2, 3, 4, 5])).unwrap();
        s.advance(image(2, &[3, 4, 5, 6, 7, 8]), 3, 0, 0).unwrap();
        s.advance(image(2, &[6, 7, 8, 9, 10, 11]), 3, 0, 0).unwrap();
        assert_eq!(
            s.finish().unwrap(),
            image(2, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
        );
    }
    #[test]
    fn reject_invalid_match_preserves_original() {
        let original = image(2, &[1, 2, 3]);
        let mut s = Stitcher::new(original.clone()).unwrap();
        assert!(s.advance(image(2, &[4, 5, 6]), 3, 0, 0).is_err());
        assert_eq!(s.finish().unwrap(), original);
    }
    #[test]
    fn rejects_inconsistent_image() {
        assert!(Image::new(2, 3, vec![0; 8]).is_err());
    }

    #[test]
    fn failed_reservation_preserves_previous_scroll_capture() {
        let initial = image(2, &[1, 2, 3, 4]);
        let next = image(2, &[2, 3, 4, 5]);
        let later = image(2, &[3, 4, 5, 6]);
        let mut first_failure = Stitcher::new(initial.clone()).unwrap();
        assert_eq!(
            first_failure.advance_with_reserve(next.clone(), 1, 0, 0, |_, _| Err("reserve failed")),
            Err("reserve failed")
        );
        assert!(first_failure.center.is_none());
        assert_eq!(first_failure.finish().unwrap(), initial);
        let mut stitcher = Stitcher::new(initial.clone()).unwrap();
        let mut control = Stitcher::new(initial).unwrap();
        stitcher.advance(next.clone(), 1, 0, 0).unwrap();
        control.advance(next, 1, 0, 0).unwrap();
        let before = stitcher.length();
        assert_eq!(
            stitcher.advance_with_reserve(later, 1, 0, 0, |_, _| Err("reserve failed")),
            Err("reserve failed")
        );
        assert_eq!(stitcher.length(), before);
        assert!(stitcher.rows.capacity() >= stitcher.length() * stitcher.prev.width * 4);
        let allocation = stitcher.rows.as_ptr();
        let recovered = stitcher.finish().unwrap();
        assert_eq!(recovered.pixels.as_ptr(), allocation);
        assert_eq!(recovered, control.finish().unwrap());
    }

    #[test]
    fn empty_prefix_needs_no_finalization_reservation() {
        let initial = image(1, &[1, 2]);
        let next = image(1, &[2, 3]);
        let allocation = next.pixels.as_ptr();
        let mut stitcher = Stitcher::new(initial).unwrap();
        stitcher
            .advance_with_reserve(next, 1, 0, 1, |_, _| {
                panic!("empty prefix must not require a reservation")
            })
            .unwrap();
        let result = stitcher.finish().unwrap();
        assert_eq!(result.pixels.as_ptr(), allocation);
        assert_eq!(result, image(1, &[2, 3]));
    }

    #[test]
    fn stitcher_rejects_invalid_public_image_fields_and_overflowing_margins() {
        let invalid = Image {
            width: 2,
            height: 2,
            pixels: vec![0; 4],
        };
        assert!(Stitcher::new(invalid.clone()).is_err());
        let frame = image(1, &[1, 2, 3]);
        let mut stitcher = Stitcher::new(frame.clone()).unwrap();
        assert!(stitcher.advance(invalid, 1, 0, 0).is_err());
        let malformed_same_size = Image {
            width: 1,
            height: 3,
            pixels: vec![0; 4],
        };
        assert!(stitcher.advance(malformed_same_size, 1, 0, 0).is_err());
        assert!(stitcher.advance(frame.clone(), 1, usize::MAX, 1).is_err());
        assert_eq!(stitcher.finish().unwrap(), frame);
    }

    #[test]
    fn scroll_capture_length_limit_accepts_30000_and_rejects_30001() {
        let at_limit = Image::new(1, MAX_LENGTH, vec![0; MAX_LENGTH * 4]).unwrap();
        assert!(Stitcher::new(at_limit).is_ok());

        let over_limit = Image::new(1, MAX_LENGTH + 1, vec![0; (MAX_LENGTH + 1) * 4]).unwrap();
        assert_eq!(
            Stitcher::new(over_limit).err(),
            Some("Aufnahmebereich überschreitet 30000 Pixel")
        );
    }

    #[test]
    fn stitched_length_limit_keeps_exact_limit_result_after_rejecting_next_step() {
        let frame = image(1, &[1, 2, 3]);
        let mut stitcher = Stitcher::new(frame.clone()).unwrap();
        for _ in 0..(MAX_LENGTH - frame.height) {
            stitcher
                .advance(frame.clone(), 1, 0, 0)
                .expect("each one-row step should reach the limit exactly");
        }
        assert_eq!(stitcher.length(), MAX_LENGTH);

        assert_eq!(
            stitcher.advance(frame, 1, 0, 0),
            Err("Maximale Bildgröße erreicht")
        );
        let completed = stitcher
            .finish()
            .expect("the last valid partial result remains finishable");
        assert_eq!(completed.height, MAX_LENGTH);
    }
}
