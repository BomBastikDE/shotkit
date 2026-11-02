//! Pixel-exact rectangle editing for the virtual-desktop selection overlay.

pub type Rect = (i32, i32, i32, i32);

/// Translate a half-open rectangle while keeping it inside half-open bounds.
pub fn translate(rect: Rect, dx: i32, dy: i32, bounds: Rect) -> Rect {
    let width = rect.2.saturating_sub(rect.0).max(1);
    let height = rect.3.saturating_sub(rect.1).max(1);
    let max_left = bounds.2.saturating_sub(width).max(bounds.0);
    let max_top = bounds.3.saturating_sub(height).max(bounds.1);
    let left = rect.0.saturating_add(dx).clamp(bounds.0, max_left);
    let top = rect.1.saturating_add(dy).clamp(bounds.1, max_top);
    (
        left,
        top,
        left.saturating_add(width).min(bounds.2),
        top.saturating_add(height).min(bounds.3),
    )
}

/// Change the right or bottom edge by one pixel. Width and height stay >= 1.
/// Bounds use screen coordinates and may have negative origins.
pub fn resize(rect: Rect, dx: i32, dy: i32, bounds: Rect) -> Rect {
    let left = rect.0.clamp(bounds.0, bounds.2.saturating_sub(1));
    let top = rect.1.clamp(bounds.1, bounds.3.saturating_sub(1));
    let right = rect
        .2
        .saturating_add(dx)
        .clamp(left.saturating_add(1), bounds.2);
    let bottom = rect
        .3
        .saturating_add(dy)
        .clamp(top.saturating_add(1), bounds.3);
    (left, top, right, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: Rect = (-1920, -100, 1920, 980);

    #[test]
    fn arrows_translate_exactly_one_pixel_and_keep_size() {
        let rect = (-100, 20, 80, 170);
        assert_eq!(translate(rect, -1, 0, DESKTOP), (-101, 20, 79, 170));
        assert_eq!(translate(rect, 1, 0, DESKTOP), (-99, 20, 81, 170));
        assert_eq!(translate(rect, 0, -1, DESKTOP), (-100, 19, 80, 169));
        assert_eq!(translate(rect, 0, 1, DESKTOP), (-100, 21, 80, 171));
    }

    #[test]
    fn shift_arrows_resize_right_and_bottom_edges_by_one_pixel() {
        let rect = (-100, 20, 80, 170);
        assert_eq!(resize(rect, -1, 0, DESKTOP), (-100, 20, 79, 170));
        assert_eq!(resize(rect, 1, 0, DESKTOP), (-100, 20, 81, 170));
        assert_eq!(resize(rect, 0, -1, DESKTOP), (-100, 20, 80, 169));
        assert_eq!(resize(rect, 0, 1, DESKTOP), (-100, 20, 80, 171));
    }

    #[test]
    fn movement_and_resize_clamp_to_negative_origin_virtual_desktop() {
        assert_eq!(
            translate((-1920, -100, -1800, 0), -1, -1, DESKTOP),
            (-1920, -100, -1800, 0)
        );
        assert_eq!(
            translate((1800, 900, 1920, 980), 1, 1, DESKTOP),
            (1800, 900, 1920, 980)
        );
        assert_eq!(
            resize((1800, 900, 1920, 980), 1, 1, DESKTOP),
            (1800, 900, 1920, 980)
        );
        assert_eq!(
            resize((-1920, -100, -1800, 0), -1, -1, DESKTOP),
            (-1920, -100, -1801, -1)
        );
    }

    #[test]
    fn resize_never_collapses_below_one_pixel() {
        assert_eq!(resize((0, 0, 1, 1), -1, -1, (0, 0, 10, 10)), (0, 0, 1, 1));
    }
}
