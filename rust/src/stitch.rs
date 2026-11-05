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
        let h = self.prev.height;
        if next.width != self.prev.width
            || next.height != h
            || dy == 0
            || dy >= h
            || top + bottom >= h
        {
            return Err("Ungültiger Bildabgleich");
        }
        let center = *self.center.get_or_insert((top + h - bottom) / 2);
        let seam = (center + dy / 2).clamp(self.start, h);
        let next_start = seam.saturating_sub(dy);
        let stride = self.prev.width * 4;
        let new_len = self.rows.len() / stride + seam - self.start + h - next_start;
        if new_len > MAX_LENGTH
            || new_len
                .checked_mul(stride)
                .filter(|n| *n <= MAX_BYTES)
                .is_none()
        {
            return Err("Maximale Bildgröße erreicht");
        }
        self.rows
            .extend_from_slice(&self.prev.pixels[self.start * stride..seam * stride]);
        self.start = next_start;
        self.prev = next;
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
