use super::*;

fn p(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

#[test]
fn shift_constrains_box_shapes_in_both_drag_directions_and_preserves_zero_axes() {
    assert_eq!(
        constrain_endpoint(p(10.0, 10.0), p(20.0, 15.0), true),
        p(20.0, 20.0)
    );
    assert_eq!(
        constrain_endpoint(p(10.0, 10.0), p(5.0, 8.0), true),
        p(5.0, 5.0)
    );
    assert_eq!(
        constrain_endpoint(p(10.0, 10.0), p(10.0, 15.0), true),
        p(10.0, 15.0)
    );
    assert_eq!(
        constrain_endpoint(p(10.0, 10.0), p(15.0, 10.0), true),
        p(15.0, 10.0)
    );
}

#[test]
fn shift_snaps_lines_and_arrows_to_nearest_45_degrees_in_all_quadrants() {
    let length = (10.0_f32 * 10.0 + 4.0 * 4.0).sqrt();
    let right = constrain_endpoint(p(0.0, 0.0), p(10.0, 4.0), false);
    assert!((right.x - length).abs() < 0.0001);
    assert!(right.y.abs() < 0.0001);

    let left = constrain_endpoint(p(0.0, 0.0), p(-10.0, -4.0), false);
    assert!((left.x + length).abs() < 0.0001);
    assert!(left.y.abs() < 0.0001);

    let diagonal = constrain_endpoint(p(2.0, 3.0), p(-8.0, 13.0), false);
    assert!((diagonal.x + 8.0).abs() < 0.0001);
    assert!((diagonal.y - 13.0).abs() < 0.0001);
}

#[test]
fn csharp_midpoint_rounding_is_ties_to_even_for_positive_and_negative_angles() {
    assert_eq!(round_ties_even(0.5), 0.0);
    assert_eq!(round_ties_even(1.5), 2.0);
    assert_eq!(round_ties_even(-0.5), 0.0);
    assert_eq!(round_ties_even(-1.5), -2.0);
}

#[test]
fn shift_modifier_is_applied_to_preview_and_final_mouse_up() {
    for tool in [
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Highlight,
        Tool::Blur,
        Tool::Redact,
    ] {
        let mut e = editor();
        e.set_tool(tool);
        e.mouse_down(p(10.0, 10.0));
        e.mouse_move_with_shift(p(20.0, 15.0), true);
        let preview = e.preview().unwrap();
        assert_eq!(preview.bounds().width, 10.0, "{tool:?}");
        assert_eq!(preview.bounds().height, 10.0, "{tool:?}");
        e.mouse_up_with_shift(p(20.0, 12.0), true);
        assert_eq!(e.shapes()[0].bounds().width, 10.0, "{tool:?}");
        assert_eq!(e.shapes()[0].bounds().height, 10.0, "{tool:?}");
    }

    for tool in [Tool::Line, Tool::Arrow] {
        let mut e = editor();
        e.set_tool(tool);
        e.mouse_down(p(0.0, 0.0));
        e.mouse_up_with_shift(p(10.0, 4.0), true);
        let end = match &e.shapes()[0].kind {
            ShapeKind::Line { end, .. } | ShapeKind::Arrow { end, .. } => *end,
            _ => panic!("expected a two-point line tool"),
        };
        assert!(end.y.abs() < 0.0001, "{tool:?}");
    }

    let mut freehand = editor();
    freehand.set_tool(Tool::Pen);
    freehand.mouse_down(p(1.0, 1.0));
    freehand.mouse_move_with_shift(p(5.0, 8.0), true);
    let preview = freehand.preview().unwrap();
    let ShapeKind::Freehand { points } = preview.kind else {
        panic!("expected freehand");
    };
    assert_eq!(points.last(), Some(&p(5.0, 8.0)));
}

fn editor() -> EditorController {
    EditorController::with_image(Image::new(100, 100, vec![128; 100 * 100 * 4]).unwrap()).unwrap()
}

fn add_rect_and_select(e: &mut EditorController) {
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(50.0, 50.0));
    e.set_tool(Tool::Select);
    // Select the left edge away from the corner handles. A center click misses
    // a rectangle because hit testing targets its outline.
    e.mouse_down(p(12.0, 30.0));
    e.mouse_up(p(12.0, 30.0));
    assert!(e.selection().is_some());
}

#[test]
fn marker_tool_switches_default_red_to_yellow_and_recolors_selection_undoably() {
    let mut e = editor();
    add_rect_and_select(&mut e);
    let old_style = e.selection().unwrap().style.clone();
    assert_eq!(old_style.color, Color::rgba(230, 40, 40, 255));

    assert!(e.set_tool(Tool::Highlight));
    assert_eq!(e.current_style().color, Color::MARKER_YELLOW);
    assert_eq!(e.selection().unwrap().style.color, Color::MARKER_YELLOW);

    // One Undo restores the selected object; the next history item is still
    // the original rectangle creation, so it removes that object.
    assert!(e.undo());
    assert_eq!(e.shapes()[0].style, old_style);
    assert!(e.undo());
    assert!(e.shapes().is_empty());
    assert!(!e.undo());
}

#[test]
fn marker_tool_without_selection_changes_only_the_active_drawing_color() {
    let mut e = editor();
    assert!(!e.set_tool(Tool::Highlight));
    assert_eq!(e.current_style().color, Color::MARKER_YELLOW);
    assert!(e.shapes().is_empty());
    assert!(!e.undo());

    e.mouse_down(p(5.0, 5.0));
    e.mouse_up(p(15.0, 15.0));
    assert_eq!(e.shapes()[0].style.color, Color::MARKER_YELLOW);
}

#[test]
fn marker_tool_preserves_custom_color_and_selected_shape_style() {
    let mut e = editor();
    let blue = Color::rgba(0, 120, 215, 255);
    e.set_current_color(blue);
    add_rect_and_select(&mut e);
    let old_style = e.selection().unwrap().style.clone();

    assert!(!e.set_tool(Tool::Highlight));
    assert_eq!(e.current_style().color, blue);
    assert_eq!(e.selection().unwrap().style, old_style);
}

fn test_rect(x: f32, y: f32) -> Shape {
    Shape::new(ShapeKind::Rectangle {
        start: p(x, y),
        end: p(x + 10.0, y + 10.0),
    })
}

#[test]
fn compose_preview_replaces_in_place_and_keeps_following_shapes() {
    let original = test_rect(1.0, 2.0);
    let following = test_rect(30.0, 40.0);
    let replacement = test_rect(5.0, 6.0);
    let (shapes, stable_count) = compose_preview_shapes(
        &[original, following.clone()],
        Some((Some(0), replacement.clone())),
    )
    .unwrap();

    assert_eq!(shapes, vec![replacement, following]);
    assert_eq!(shapes.len(), 2);
    assert_eq!(stable_count, 0);
}

#[test]
fn compose_preview_appends_creation_after_existing_selected_shapes_once() {
    let selected = test_rect(1.0, 2.0);
    let following = test_rect(30.0, 40.0);
    let draft = test_rect(60.0, 70.0);
    let (shapes, stable_count) = compose_preview_shapes(
        &[selected.clone(), following.clone()],
        Some((None, draft.clone())),
    )
    .unwrap();

    assert_eq!(shapes, vec![selected, following, draft]);
    assert_eq!(shapes.len(), 3);
    assert_eq!(stable_count, 2);
}

#[test]
fn recover_constructor_returns_invalid_image_unchanged() {
    let invalid = Image {
        width: 3,
        height: 2,
        pixels: vec![9, 8, 7, 6],
    };
    let (error, recovered) = EditorController::with_image_recover(invalid.clone()).unwrap_err();
    assert_eq!(error, DocumentError::InvalidImage);
    assert_eq!(recovered, invalid);

    let valid = Image::new(1, 1, vec![4, 3, 2, 1]).unwrap();
    assert_eq!(
        EditorController::with_image_recover(valid.clone())
            .unwrap()
            .document()
            .image(),
        &valid
    );
}

fn rect_editor() -> EditorController {
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(50.0, 40.0));
    e
}

#[test]
fn creates_with_preview_then_commits_once_and_undo_redoes() {
    let mut e = editor();
    e.set_tool(Tool::Arrow);
    e.mouse_down(p(2.0, 3.0));
    e.mouse_move(p(22.0, 33.0));
    assert!(e.shapes().is_empty());
    assert!(matches!(e.preview().unwrap().kind, ShapeKind::Arrow { .. }));
    e.mouse_up(p(24.0, 35.0));
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.selected_index(), Some(0));
    assert!(e.undo());
    assert!(e.shapes().is_empty());
    assert!(e.redo());
    assert_eq!(e.shapes().len(), 1);
}

#[test]
fn preview_index_tracks_existing_shape_drags_but_not_new_shape_creation() {
    let mut e = rect_editor();
    assert_eq!(e.selected_index(), Some(0));

    // Handle drags are allowed while a drawing tool is active.
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(5.0, 5.0));
    assert_eq!(e.preview_index(), Some(0));
    assert_eq!(e.preview().unwrap().bounds().x, 5.0);
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    e.mouse_up(p(5.0, 5.0));

    // A create drag can retain the previous selection, but must append its
    // preview instead of replacing that selected document shape.
    e.mouse_down(p(70.0, 70.0));
    e.mouse_move(p(80.0, 85.0));
    assert_eq!(e.selected_index(), Some(0));
    assert_eq!(e.preview_index(), None);
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.preview().unwrap().bounds().x, 70.0);
}

#[test]
fn moves_as_one_step_and_delete_undo_redo() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_move(p(20.0, 30.0));
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.preview().unwrap().bounds().x, 20.0);
    assert_eq!(e.preview_index(), Some(0));
    e.mouse_up(p(20.0, 30.0));
    assert_eq!(e.shapes()[0].bounds().x, 20.0);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert!(e.redo());
    assert_eq!(e.shapes()[0].bounds().x, 20.0);
    assert!(e.delete_selected());
    assert!(e.shapes().is_empty());
    assert!(e.undo());
    assert_eq!(e.shapes().len(), 1);
    assert!(e.redo());
    assert!(e.shapes().is_empty());
}
#[test]
fn nudge_is_one_undoable_edit_and_requires_a_selection() {
    let mut e = rect_editor();
    e.clear_selection();
    assert!(!e.nudge_selected(1.0, 0.0));
    e.set_tool(Tool::Select);
    select_first(&mut e);
    assert!(e.nudge_selected(1.0, -10.0));
    assert_eq!(e.shapes()[0].bounds().x, 11.0);
    assert_eq!(e.shapes()[0].bounds().y, 0.0);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.shapes()[0].bounds().y, 10.0);
    assert!(e.redo());
    assert_eq!(e.shapes()[0].bounds().x, 11.0);
    assert_eq!(e.shapes()[0].bounds().y, 0.0);
}
#[test]
fn nudge_during_pointer_gesture_does_not_cancel_or_commit_it() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_move(p(30.0, 25.0));
    assert!(!e.nudge_selected(1.0, 0.0));
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert!(e.preview().is_some());
    e.mouse_up(p(30.0, 25.0));
    assert_eq!(e.shapes()[0].bounds().x, 30.0);
}
#[test]
fn resizes_a_selected_handle_as_one_step() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(10.0, 10.0));
    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(4.0, 5.0));
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.preview().unwrap().bounds().x, 4.0);
    e.mouse_up(p(4.0, 5.0));
    assert_eq!(e.shapes()[0].bounds().x, 4.0);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
}

#[test]
fn rectangle_tool_resizes_selected_handle_before_creating() {
    let mut e = rect_editor();
    assert_eq!(e.tool(), Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(4.0, 5.0));
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.preview().unwrap().bounds().x, 4.0);
    e.mouse_up(p(4.0, 5.0));
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.shapes()[0].bounds().x, 4.0);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert!(e.redo());
    assert_eq!(e.shapes()[0].bounds().x, 4.0);
}

#[test]
fn rectangle_tool_still_creates_outside_selected_handles() {
    let mut e = rect_editor();
    e.mouse_down(p(70.0, 70.0));
    e.mouse_move(p(90.0, 85.0));
    assert_eq!(e.shapes().len(), 1);
    assert!(matches!(
        e.preview().unwrap().kind,
        ShapeKind::Rectangle { .. }
    ));
    e.mouse_up(p(90.0, 85.0));
    assert_eq!(e.shapes().len(), 2);
    assert_eq!(e.selected_index(), Some(1));
    assert!(e.undo());
    assert_eq!(e.shapes().len(), 1);
    assert!(e.redo());
    assert_eq!(e.shapes().len(), 2);
}
#[test]
fn cancellation_discards_draft_and_restores_selection() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(10.0, 10.0));
    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(100.0, 100.0));
    e.cancel_drag();
    assert_eq!(e.selected_index(), Some(0));
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert!(e.preview().is_none());
    // The only existing undo point is creation; cancelling the drag added none.
    assert!(e.undo());
    assert!(e.shapes().is_empty());
    assert!(e.redo());
    assert_eq!(e.shapes().len(), 1);
}
#[test]
fn text_and_step_are_creatable() {
    let mut e = editor();
    assert!(e.place_text(p(5.0, 6.0), "note"));
    assert!(matches!(e.shapes()[0].kind, ShapeKind::Text { .. }));
    e.set_tool(Tool::Step);
    e.mouse_down(p(8.0, 9.0));
    e.mouse_up(p(8.0, 9.0));
    assert!(matches!(
        e.shapes()[1].kind,
        ShapeKind::Step { number: 1, .. }
    ));
}

fn two_rectangles() -> EditorController {
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(50.0, 40.0));
    e.mouse_down(p(100.0, 10.0));
    e.mouse_up(p(140.0, 40.0));
    e.set_tool(Tool::Select);
    e
}

fn select_first(e: &mut EditorController) {
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(10.0, 25.0));
    assert_eq!(e.selected_index(), Some(0));
}

#[test]
fn undo_redo_shape_count_change_never_keeps_a_shifted_selection() {
    let mut e = two_rectangles();
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(180.0, 10.0));
    e.mouse_up(p(220.0, 40.0));
    e.set_tool(Tool::Select);
    select_first(&mut e);
    assert!(e.delete_selected());
    e.mouse_down(p(180.0, 25.0));
    e.mouse_up(p(180.0, 25.0));
    assert_eq!(e.selected_index(), Some(1));

    assert!(e.undo());
    assert_eq!(e.shapes().len(), 3);
    assert_eq!(e.selected_index(), None);
    assert!(!e.delete_selected());
    assert!(!e.nudge_selected(1.0, 0.0));
    assert_eq!(e.shapes().len(), 3);

    e.mouse_down(p(100.0, 25.0));
    e.mouse_up(p(100.0, 25.0));
    assert_eq!(e.selected_index(), Some(1));
    assert!(e.redo());
    assert_eq!(e.shapes().len(), 2);
    assert_eq!(e.selected_index(), None);
    assert!(!e.delete_selected());
}

#[test]
fn delete_during_move_cancels_drag_before_removing_shape() {
    let mut e = two_rectangles();
    select_first(&mut e);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_move(p(30.0, 25.0));

    assert!(e.delete_selected());
    e.mouse_up(p(30.0, 25.0));

    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.shapes()[0].bounds().x, 100.0);
    assert!(e.undo());
    assert_eq!(e.shapes().len(), 2);
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.shapes()[1].bounds().x, 100.0);
}

#[test]
fn delete_during_handle_resize_cancels_drag_before_removing_shape() {
    let mut e = two_rectangles();
    select_first(&mut e);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(5.0, 5.0));

    assert!(e.delete_selected());
    e.mouse_up(p(5.0, 5.0));

    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.shapes()[0].bounds().x, 100.0);
    assert_eq!(e.shapes()[0].bounds().y, 10.0);
}

#[test]
fn undo_during_move_cancels_drag_without_later_stale_edit() {
    let mut e = two_rectangles();
    select_first(&mut e);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(30.0, 25.0));
    assert_eq!(e.shapes()[0].bounds().x, 30.0);

    e.mouse_down(p(30.0, 25.0));
    e.mouse_move(p(45.0, 25.0));
    assert!(e.undo());
    e.mouse_up(p(45.0, 25.0));

    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.shapes()[1].bounds().x, 100.0);
}

#[test]
fn redo_during_move_cancels_drag_without_later_stale_edit() {
    let mut e = two_rectangles();
    select_first(&mut e);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(30.0, 25.0));
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);

    e.mouse_down(p(10.0, 25.0));
    e.mouse_move(p(20.0, 25.0));
    assert!(e.redo());
    e.mouse_up(p(20.0, 25.0));

    assert_eq!(e.shapes()[0].bounds().x, 30.0);
    assert_eq!(e.shapes()[1].bounds().x, 100.0);
}

#[test]
fn placing_text_during_move_cancels_stale_drag() {
    let mut e = two_rectangles();
    select_first(&mut e);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_move(p(30.0, 25.0));
    assert!(e.place_text(p(200.0, 200.0), "note"));
    e.mouse_up(p(30.0, 25.0));

    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert_eq!(e.shapes()[1].bounds().x, 100.0);
    assert!(matches!(e.shapes()[2].kind, ShapeKind::Text { .. }));
}

#[test]
fn new_shapes_and_placed_text_use_current_style() {
    let mut e = editor();
    let style = Style {
        color: Color::rgba(12, 34, 56, 200),
        width: 7.5,
        font_size: 31.0,
    };
    assert!(e.set_current_style(style.clone()));

    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(30.0, 30.0));
    assert_eq!(e.shapes()[0].style, style);

    assert!(e.place_text(p(40.0, 40.0), "styled"));
    assert_eq!(e.shapes()[1].style, style);
}

#[test]
fn selected_style_change_is_undoable_and_redoable() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(10.0, 25.0));
    let replacement = Style {
        color: Color::rgba(20, 80, 140, 255),
        width: 9.0,
        font_size: 42.0,
    };

    assert!(e.set_selected_style(replacement.clone()));
    assert_eq!(e.shapes()[0].style, replacement);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].style, Style::default());
    assert!(e.redo());
    assert_eq!(e.shapes()[0].style, replacement);
}

#[test]
fn selected_text_can_be_read_and_edited_with_undo() {
    let mut e = editor();
    assert!(e.place_text(p(5.0, 6.0), "before"));
    assert_eq!(e.selected_text(), Some("before"));

    assert!(e.set_selected_text("after"));
    assert_eq!(e.selected_text(), Some("after"));
    assert!(e.undo());
    assert_eq!(e.selected_text(), Some("before"));
    assert!(e.redo());
    assert_eq!(e.selected_text(), Some("after"));
}

#[test]
fn text_hit_uses_topmost_shape_and_returns_text_without_changing_selection() {
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(60.0, 50.0));
    assert!(e.place_text(p(20.0, 20.0), "front"));
    let original_selection = e.selected_index();

    // x=15 lies within the rectangle's hit band and the text bounds' hit
    // tolerance, so both objects overlap at this point.
    assert_eq!(e.text_at(p(15.0, 25.0)), Some((1, "front".to_owned())));
    assert_eq!(e.selected_index(), original_selection);
    assert_eq!(e.text_at(p(80.0, 80.0)), None);
}

#[test]
fn text_hit_on_topmost_non_text_does_not_edit_text_underneath() {
    let mut e = editor();
    assert!(e.place_text(p(10.0, 10.0), "behind"));
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(60.0, 50.0));

    assert_eq!(e.text_at(p(10.0, 25.0)), None);
}

#[test]
fn text_edit_is_undoable_and_no_change_adds_no_history() {
    let mut e = editor();
    assert!(e.place_text(p(5.0, 6.0), "before"));
    let (index, existing) = e.text_at(p(8.0, 8.0)).unwrap();
    assert_eq!(existing, "before");
    assert!(!e.set_text_at(index, existing));
    assert!(e.set_text_at(index, "after"));
    assert!(matches!(&e.shapes()[0].kind, ShapeKind::Text { text, .. } if text == "after"));
    assert!(e.undo());
    assert!(matches!(&e.shapes()[0].kind, ShapeKind::Text { text, .. } if text == "before"));
    assert!(e.redo());
    assert!(matches!(&e.shapes()[0].kind, ShapeKind::Text { text, .. } if text == "after"));
}

#[test]
fn text_tool_keeps_selected_shape_handle_priority() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(10.0, 10.0));
    e.set_tool(Tool::Text);
    assert!(e.selected_handle_hit(p(10.0, 10.0)));

    e.mouse_down(p(10.0, 10.0));
    e.mouse_move(p(4.0, 5.0));
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.preview().unwrap().bounds().x, 4.0);
    e.mouse_up(p(4.0, 5.0));
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.shapes()[0].bounds().x, 4.0);
    assert!(e.undo());
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
    assert!(e.redo());
    assert_eq!(e.shapes()[0].bounds().x, 4.0);
}

#[test]
fn invalid_style_parameters_are_rejected_without_changing_current_style() {
    let mut e = editor();
    let original = e.current_style().clone();
    assert!(!e.set_current_width(f32::NAN));
    assert!(!e.set_current_width(0.0));
    assert!(!e.set_current_width(300.0));
    assert!(!e.set_current_font_size(f32::INFINITY));
    assert!(!e.set_current_font_size(0.5));
    assert!(!e.set_current_font_size(600.0));
    assert!(!e.set_current_style(Style {
        width: f32::NAN,
        ..Style::default()
    }));
    assert_eq!(e.current_style(), &original);
}

#[test]
fn mixed_shape_and_crop_share_one_undo_redo_history() {
    let mut e = editor();
    let original_image = e.image_arc();
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(20.0, 20.0));
    e.mouse_up(p(40.0, 40.0));
    assert!(Arc::ptr_eq(&original_image, &e.image_arc()));

    e.crop(CropRect {
        x: 10,
        y: 10,
        width: 60,
        height: 70,
    })
    .unwrap();
    assert_eq!(
        (e.document().image().width, e.document().image().height),
        (60, 70)
    );
    assert_eq!(e.shapes()[0].bounds().x, 10.0);

    assert!(e.undo());
    assert_eq!(
        (e.document().image().width, e.document().image().height),
        (100, 100)
    );
    assert_eq!(e.shapes()[0].bounds().x, 20.0);
    assert!(Arc::ptr_eq(&original_image, &e.image_arc()));
    assert!(e.undo());
    assert!(e.shapes().is_empty());
    assert!(e.redo());
    assert_eq!(e.shapes().len(), 1);
    assert!(e.redo());
    assert_eq!(e.document().image().width, 60);
    assert_eq!(e.shapes()[0].bounds().x, 10.0);
}

#[test]
fn frame_is_undoable_with_shape_coordinates_and_image_arc() {
    let mut e = rect_editor();
    let original_image = e.image_arc();
    let before = e.shapes()[0].bounds();
    e.add_frame(FrameOptions {
        width: 5,
        color: Color::rgba(255, 255, 255, 255),
        shadow: None,
    })
    .unwrap();
    assert_eq!(e.document().image().width, 110);
    assert_eq!(e.shapes()[0].bounds().x, before.x + 5.0);
    assert!(!Arc::ptr_eq(&original_image, &e.image_arc()));
    assert!(e.undo());
    assert_eq!(e.document().image().width, 100);
    assert_eq!(e.shapes()[0].bounds().x, before.x);
    assert!(Arc::ptr_eq(&original_image, &e.image_arc()));
    assert!(e.redo());
    assert_eq!(e.document().image().width, 110);
}

#[test]
fn history_budget_errors_are_exposed_and_leave_document_unchanged() {
    let mut e = editor();
    e.document_mut().set_history_budget(0);
    e.set_tool(Tool::Rectangle);
    e.mouse_down(p(10.0, 10.0));
    e.mouse_up(p(30.0, 30.0));
    assert!(e.shapes().is_empty());
    assert_eq!(e.take_error(), Some(DocumentError::HistoryBudgetExceeded));
    assert!(e.take_error().is_none());
    assert_eq!(
        e.crop(CropRect {
            x: 0,
            y: 0,
            width: 50,
            height: 50
        }),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert_eq!(
        (e.document().image().width, e.document().image().height),
        (100, 100)
    );
}

#[test]
fn failed_delete_keeps_shape_and_selection_and_exposes_error() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(10.0, 25.0));
    e.document_mut().set_history_budget(0);

    assert!(!e.delete_selected());
    assert_eq!(e.shapes().len(), 1);
    assert_eq!(e.selected_index(), Some(0));
    assert_eq!(e.take_error(), Some(DocumentError::HistoryBudgetExceeded));
}

#[test]
fn undo_redo_keeps_selection_valid_across_crop() {
    let mut e = rect_editor();
    e.set_tool(Tool::Select);
    e.mouse_down(p(10.0, 25.0));
    e.mouse_up(p(10.0, 25.0));
    assert_eq!(e.selected_index(), Some(0));
    e.crop(CropRect {
        x: 0,
        y: 0,
        width: 80,
        height: 80,
    })
    .unwrap();
    assert!(e.undo());
    assert_eq!(e.selected_index(), Some(0));
    assert!(e.selection().is_some());
    assert!(e.redo());
    assert_eq!(e.selected_index(), Some(0));
    assert!(e.selection().is_some());
}

#[test]
fn redact_tool_creates_and_moves_a_redaction() {
    let mut e = editor();
    e.set_tool(Tool::Redact);
    e.mouse_down(p(10.0, 12.0));
    e.mouse_move(p(40.0, 35.0));
    assert!(matches!(
        e.preview().unwrap().kind,
        ShapeKind::Redact { .. }
    ));
    e.mouse_up(p(40.0, 35.0));
    assert!(matches!(e.shapes()[0].kind, ShapeKind::Redact { .. }));
    e.set_tool(Tool::Select);
    e.mouse_down(p(25.0, 23.0));
    e.mouse_up(p(25.0, 23.0));
    assert!(e.nudge_selected(2.0, 3.0));
    assert_eq!(e.shapes()[0].bounds().x, 12.0);
}
