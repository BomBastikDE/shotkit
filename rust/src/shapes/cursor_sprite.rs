//! Owned, editable cursor raster, independent of native cursor handle lifetime.
use super::{Point, Rect};
use crate::cursor_snapshot::{CursorPixels, MAX_CURSOR_SIDE};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct CursorSprite {
    pub origin: Point,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<CursorPixels>,
}

impl CursorSprite {
    pub fn new(origin: Point, width: u32, height: u32, pixels: CursorPixels) -> Option<Self> {
        let value = Self {
            origin,
            width,
            height,
            pixels: Arc::new(pixels),
        };
        value.validate().then_some(value)
    }

    pub fn validate(&self) -> bool {
        if !self.origin.x.is_finite()
            || !self.origin.y.is_finite()
            || self.origin.x.abs() > 1_000_000.0
            || self.origin.y.abs() > 1_000_000.0
            || self.width == 0
            || self.height == 0
            || self.width > MAX_CURSOR_SIDE
            || self.height > MAX_CURSOR_SIDE
        {
            return false;
        }
        let Some(count) = (self.width as usize).checked_mul(self.height as usize) else {
            return false;
        };
        match self.pixels.as_ref() {
            CursorPixels::PremultipliedRgba(bytes) => {
                bytes.len() == count * 4
                    && bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[..3].iter().all(|&channel| channel <= p[3]))
            }
            CursorPixels::Raster { and_mask, xor_rgb } => {
                and_mask.len() == count
                    && xor_rgb.len() == count * 3
                    && and_mask.iter().all(|&mask| mask == 0 || mask == 255)
            }
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect {
            x: self.origin.x,
            y: self.origin.y,
            width: self.width as f32,
            height: self.height as f32,
        }
    }

    /// Atomic movement; a cursor has no resize handles.
    pub fn offset(&mut self, dx: f32, dy: f32) -> bool {
        if !dx.is_finite() || !dy.is_finite() {
            return false;
        }
        let mut next = self.clone();
        next.origin.x += dx;
        next.origin.y += dy;
        if !next.validate() {
            return false;
        }
        *self = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_rasters_are_rejected_and_moves_are_atomic() {
        assert!(CursorSprite::new(
            Point::new(0.0, 0.0),
            1,
            1,
            CursorPixels::PremultipliedRgba(vec![255, 0, 0, 0])
        )
        .is_none());
        assert!(CursorSprite::new(
            Point::new(0.0, 0.0),
            1,
            1,
            CursorPixels::Raster {
                and_mask: vec![1],
                xor_rgb: vec![0, 0, 0]
            }
        )
        .is_none());
        let mut sprite = CursorSprite::new(
            Point::new(-2.0, 3.0),
            1,
            1,
            CursorPixels::Raster {
                and_mask: vec![255],
                xor_rgb: vec![255, 255, 255],
            },
        )
        .unwrap();
        let old = sprite.clone();
        assert!(!sprite.offset(f32::NAN, 0.0));
        assert_eq!(sprite, old);
        assert!(sprite.offset(4.0, -2.0));
        assert_eq!(sprite.origin, Point::new(2.0, 1.0));
    }
}
