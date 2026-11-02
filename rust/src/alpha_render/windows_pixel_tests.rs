    use super::*;
    use crate::shapes::{Point, ShapeKind};

    #[test]
    fn highlight_layer_preserves_transparency_and_marker_opacity() {
        let shape = Shape::new(ShapeKind::Highlight {
            start: Point::new(2.0, 2.0),
            end: Point::new(8.0, 8.0),
        });
        let image = render_shape(12, 12, &shape).expect("GDI+ highlight render");
        let pixel = |x: usize, y: usize| {
            let i = (y * image.width + x) * 4;
            [
                image.pixels[i],
                image.pixels[i + 1],
                image.pixels[i + 2],
                image.pixels[i + 3],
            ]
        };
        assert_eq!(
            pixel(0, 0),
            [0, 0, 0, 0],
            "untouched layer pixels stay transparent"
        );
        assert_eq!(
            pixel(4, 4),
            [0, 0, 255, 105],
            "marker keeps its intended translucency"
        );
    }

    #[test]
    fn step_marker_is_centered_legible_and_keeps_transparent_layer() {
        for color in [
            crate::shapes::Color::rgba(0, 0, 0, 255),
            crate::shapes::Color::rgba(255, 255, 255, 255),
        ] {
            for number in [1, 9, 10, 100, 999, 1000] {
                let mut shape = Shape::new(ShapeKind::Step {
                    center: Point::new(48.0, 48.0),
                    number,
                });
                shape.style.color = color;
                let image = render_shape(96, 96, &shape).expect("GDI+ step render");
                let pixel = |x: usize, y: usize| {
                    let i = (y * image.width + x) * 4;
                    [
                        image.pixels[i],
                        image.pixels[i + 1],
                        image.pixels[i + 2],
                        image.pixels[i + 3],
                    ]
                };
                assert_eq!(pixel(0, 0), [0, 0, 0, 0]);
                let mut glyph = Vec::new();
                // The white outline sits near radius 18.8; inspect only the
                // opaque center region so neither that ring nor transparent
                // pixels outside the marker can satisfy the text assertions.
                let inner_extent = (18.8_f32 * 0.70).floor() as usize;
                let lo = 48 - inner_extent;
                let hi = 48 + inner_extent;
                for y in lo..=hi {
                    for x in lo..=hi {
                        let dx = x as f32 - 48.0;
                        let dy = y as f32 - 48.0;
                        if dx * dx + dy * dy > (18.8_f32 * 0.70).powi(2) {
                            continue;
                        }
                        let p = pixel(x, y);
                        if p[3] == 0 {
                            continue;
                        }
                        let visible = if color.r == 0 {
                            p[0] > 235 && p[1] > 235 && p[2] > 235
                        } else {
                            p[0] < 80 && p[1] < 80 && p[2] < 80
                        };
                        if visible {
                            glyph.push((x, y));
                        }
                    }
                }
                assert!(!glyph.is_empty(), "{number} has visible contrasting glyphs");
                let min_x = glyph.iter().map(|p| p.0).min().unwrap();
                let max_x = glyph.iter().map(|p| p.0).max().unwrap();
                let min_y = glyph.iter().map(|p| p.1).min().unwrap();
                let max_y = glyph.iter().map(|p| p.1).max().unwrap();
                assert!(
                    (min_x + max_x) / 2 >= 45 && (min_x + max_x) / 2 <= 51,
                    "{number} centered horizontally: {min_x}..{max_x}"
                );
                assert!(
                    (min_y + max_y) / 2 >= 45 && (min_y + max_y) / 2 <= 51,
                    "{number} centered vertically: {min_y}..{max_y}"
                );
                assert!(
                    max_x - min_x < 32,
                    "{number} fits within the marker interior"
                );
            }
        }
    }
