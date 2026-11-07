//! Turn a sampled mouse cursor into an editable shape of a fresh capture (#36).
//!
//! Platform independent: the Windows capture code samples the cursor with
//! [`crate::cursor_snapshot::capture`] and passes the snapshot here together
//! with the capture rectangle in the same physical screen coordinates.

use crate::cursor_snapshot::CursorSnapshot;
use crate::shapes::{CursorSprite, Point, Shape, ShapeKind};

/// The cursor as a movable, deletable object in image coordinates, or `None`
/// if no part of it lies inside the captured rectangle.
///
/// `origin` is the capture's top-left screen pixel and `size` its width and
/// height. A cursor partly outside the capture is kept; rendering clips it.
pub fn cursor_shape_for_capture(
    snapshot: &CursorSnapshot,
    origin: (i64, i64),
    size: (u32, u32),
) -> Option<Shape> {
    let (left, top) = snapshot.top_left_in_capture(origin).ok()?;
    let right = left.checked_add(i64::from(snapshot.width))?;
    let bottom = top.checked_add(i64::from(snapshot.height))?;
    if right <= 0 || bottom <= 0 || left >= i64::from(size.0) || top >= i64::from(size.1) {
        return None;
    }
    let sprite = CursorSprite::new(
        Point::new(left as f32, top as f32),
        snapshot.width,
        snapshot.height,
        snapshot.pixels.clone(),
    )?;
    Some(Shape::new(ShapeKind::Cursor { data: sprite }))
}

#[cfg(test)]
mod tests {
    use super::cursor_shape_for_capture;
    use crate::cursor_snapshot::{CursorPixels, CursorSnapshot};
    use crate::shapes::ShapeKind;

    fn snapshot(hotspot_on_screen: (i64, i64)) -> CursorSnapshot {
        CursorSnapshot {
            width: 4,
            height: 6,
            screen_hotspot: hotspot_on_screen,
            hotspot: (1, 2),
            pixels: CursorPixels::PremultipliedRgba(vec![0; 4 * 6 * 4]),
        }
    }

    fn origin_of(shape: &crate::shapes::Shape) -> (f32, f32) {
        match &shape.kind {
            ShapeKind::Cursor { data } => (data.origin.x, data.origin.y),
            _ => panic!("cursor shape expected"),
        }
    }

    #[test]
    fn cursor_inside_a_region_capture_is_placed_by_its_hotspot() {
        // Region at screen (100, 50), 200 × 100; hotspot at screen (150, 80).
        let shape = cursor_shape_for_capture(&snapshot((150, 80)), (100, 50), (200, 100)).unwrap();
        assert_eq!(origin_of(&shape), (49.0, 28.0));
    }

    #[test]
    fn cursor_partly_outside_is_kept_and_fully_outside_is_dropped() {
        // Hotspot at the right edge: the sprite extends beyond the capture.
        let partly = cursor_shape_for_capture(&snapshot((299, 60)), (100, 50), (200, 100));
        assert_eq!(origin_of(&partly.unwrap()), (198.0, 8.0));
        // One pixel of the 4 × 6 sprite still overlaps: kept.
        for hotspot in [(98, 60), (150, 47)] {
            assert!(cursor_shape_for_capture(&snapshot(hotspot), (100, 50), (200, 100)).is_some());
        }
        // Entirely right of, below, left of and above the capture.
        for hotspot in [(301, 60), (150, 152), (96, 60), (150, 45)] {
            assert!(
                cursor_shape_for_capture(&snapshot(hotspot), (100, 50), (200, 100)).is_none(),
                "{hotspot:?}"
            );
        }
    }

    #[test]
    fn negative_monitor_coordinates_are_supported() {
        // A monitor left of and above the primary one: origin (-1920, -200).
        let shape = cursor_shape_for_capture(&snapshot((-1900, -150)), (-1920, -200), (1920, 1080))
            .unwrap();
        assert_eq!(origin_of(&shape), (19.0, 48.0));
    }

    #[test]
    fn invalid_cursor_pixels_produce_no_shape() {
        let mut broken = snapshot((150, 80));
        broken.pixels = CursorPixels::PremultipliedRgba(vec![0; 3]);
        assert!(cursor_shape_for_capture(&broken, (100, 50), (200, 100)).is_none());
    }
}
