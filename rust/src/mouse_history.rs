//! Select only newly observed mouse samples while preserving their order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseSample {
    pub x: i32,
    pub y: i32,
    pub time: u32,
}

/// Windows returns a bounded history starting with the current point. Recover
/// intermediate samples only when the previous anchor is still in that history;
/// otherwise use the current point alone rather than inventing an old path.
pub fn since_previous(history: &[MouseSample], previous: Option<MouseSample>) -> Vec<MouseSample> {
    let Some(current) = history.first().copied() else {
        return Vec::new();
    };
    let Some(index) = previous.and_then(|anchor| history.iter().position(|point| *point == anchor))
    else {
        return vec![current];
    };
    if index == 0 {
        return vec![current];
    }
    history[..index].iter().rev().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: i32, time: u32) -> MouseSample {
        MouseSample { x, y: -20, time }
    }

    #[test]
    fn restores_intermediate_points_in_chronological_order() {
        let history = [p(5, 15), p(4, 14), p(3, 13), p(2, 12), p(1, 11)];
        assert_eq!(
            since_previous(&history, Some(p(2, 12))),
            vec![p(3, 13), p(4, 14), p(5, 15)]
        );
    }

    #[test]
    fn first_sample_or_expired_anchor_never_replays_unrelated_history() {
        let history = [p(4, 14), p(3, 13), p(2, 12)];
        assert_eq!(since_previous(&history, None), vec![p(4, 14)]);
        assert_eq!(since_previous(&history, Some(p(1, 11))), vec![p(4, 14)]);
        assert!(since_previous(&[], Some(p(1, 11))).is_empty());
    }

    #[test]
    fn recovered_turn_is_preserved_in_committed_freehand_stroke() {
        use crate::{
            editor::EditorController,
            shapes::{Point, ShapeKind, Tool},
            stitch::Image,
        };
        let image = Image::new(100, 100, vec![255; 100 * 100 * 4]).unwrap();
        let mut editor = EditorController::with_image(image).unwrap();
        editor.set_tool(Tool::Pen);
        editor.mouse_down(Point::new(10.0, 50.0));
        let samples = [
            MouseSample {
                x: 80,
                y: 50,
                time: 4,
            },
            MouseSample {
                x: 50,
                y: 10,
                time: 3,
            },
            MouseSample {
                x: 30,
                y: 50,
                time: 2,
            },
        ];
        editor.mouse_move(Point::new(30.0, 50.0));
        for sample in since_previous(&samples, Some(samples[2])) {
            editor.mouse_move(Point::new(sample.x as f32, sample.y as f32));
        }
        editor.mouse_up(Point::new(80.0, 50.0));
        let ShapeKind::Freehand { points } = &editor.shapes()[0].kind else {
            panic!("expected freehand");
        };
        assert!(
            points.contains(&Point::new(50.0, 10.0)),
            "the fast turn must not collapse to a straight line"
        );
        assert_eq!(points.last(), Some(&Point::new(80.0, 50.0)));
    }

    #[test]
    fn equal_coordinates_at_different_times_remain_distinguishable() {
        let history = [p(3, 23), p(2, 22), p(3, 21), p(2, 20)];
        assert_eq!(
            since_previous(&history, Some(p(3, 21))),
            vec![p(2, 22), p(3, 23)]
        );
    }
}
