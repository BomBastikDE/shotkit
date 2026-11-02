    use super::*;
    use crate::shapes::{Point, ShapeKind};

    #[test]
    fn blur_is_explicitly_left_to_source_aware_renderer() {
        let shape = Shape::new(ShapeKind::Blur {
            start: Point::new(1.0, 1.0),
            end: Point::new(8.0, 8.0),
        });
        assert_eq!(
            render_shape(16, 16, &shape),
            Err(RenderError::UnsupportedShape(
                "Verpixeln benötigt das Originalbild als Quelle"
            ))
        );
    }

    #[test]
    fn rejects_zero_or_overflowing_dimensions() {
        let shape = Shape::new(ShapeKind::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0),
        });
        assert_eq!(
            render_shape(0, 1, &shape),
            Err(RenderError::InvalidDimensions)
        );
        assert_eq!(
            render_shape(usize::MAX, 2, &shape),
            Err(RenderError::InvalidDimensions)
        );
    }

    #[test]
    fn step_labels_scale_for_multiple_digits_and_use_contrast_color() {
        assert_eq!(step_font_size(20.0, 1), 21.0);
        assert_eq!(step_font_size(20.0, 2), 18.0);
        assert!((step_font_size(20.0, 3) - 12.0).abs() < 0.001);
        assert_eq!(step_font_size(20.0, 4), 9.0);
        assert_eq!(
            step_text_color(crate::shapes::Color::rgba(0, 0, 0, 255)),
            0xffff_ffff
        );
        assert_eq!(
            step_text_color(crate::shapes::Color::rgba(255, 255, 255, 255)),
            0xff28_2828
        );
        assert_eq!(
            step_text_color(crate::shapes::Color::rgba(100, 100, 100, 20)),
            0xffff_ffff
        );
    }
