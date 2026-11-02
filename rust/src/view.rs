//! Fit, zoom and pan calculations shared by painting and hit testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
/// `zoom` multiplies fit-to-window; `pan` is the scroll offset in screen pixels.
pub fn layout(
    image: (u32, u32),
    viewport: (i32, i32, i32, i32),
    zoom: f64,
    pan: (i32, i32),
) -> View {
    let (x, y, vw, vh) = viewport;
    let vw = vw.max(1);
    let vh = vh.max(1);
    let iw = image.0.max(1) as f64;
    let ih = image.1.max(1) as f64;
    let fit = (vw as f64 / iw).min(vh as f64 / ih);
    let factor = if zoom.is_finite() {
        zoom.clamp(0.05, 64.0)
    } else {
        1.0
    };
    let w = (iw * fit * factor).round().clamp(1.0, 1_000_000.0) as i32;
    let h = (ih * fit * factor).round().clamp(1.0, 1_000_000.0) as i32;
    let px = if w <= vw {
        (vw - w) / 2
    } else {
        -pan.0.clamp(0, w - vw)
    };
    let py = if h <= vh {
        (vh - h) / 2
    } else {
        -pan.1.clamp(0, h - vh)
    };
    View {
        x: x + px,
        y: y + py,
        width: w,
        height: h,
    }
}
pub fn to_image(view: View, image: (u32, u32), point: (i32, i32)) -> (i32, i32) {
    let w = image.0.max(1).min(i32::MAX as u32) as i32;
    let h = image.1.max(1).min(i32::MAX as u32) as i32;
    let x =
        ((point.0 - view.x) as i64 * w as i64 / view.width.max(1) as i64).clamp(0, (w - 1) as i64);
    let y =
        ((point.1 - view.y) as i64 * h as i64 / view.height.max(1) as i64).clamp(0, (h - 1) as i64);
    (x as i32, y as i32)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fits_preserving_aspect() {
        assert_eq!(
            layout((400, 200), (0, 50, 200, 200), 1.0, (0, 0)),
            View {
                x: 0,
                y: 100,
                width: 200,
                height: 100
            }
        );
    }
    #[test]
    fn zoom_pan_and_hit_test() {
        let v = layout((1000, 2000), (0, 0, 500, 500), 2.0, (0, 250));
        assert_eq!(
            v,
            View {
                x: 0,
                y: -250,
                width: 500,
                height: 1000
            }
        );
        assert_eq!(to_image(v, (1000, 2000), (250, 250)), (500, 1000));
    }
    #[test]
    fn clamps_pan_and_invalid_zoom() {
        let v = layout((100, 100), (0, 0, 100, 100), f64::NAN, (99, 99));
        assert_eq!(
            v,
            View {
                x: 0,
                y: 0,
                width: 100,
                height: 100
            }
        );
        let v = layout((100, 100), (0, 0, 100, 100), 2.0, (999, 999));
        assert_eq!((v.x, v.y), (-100, -100));
    }
}
