//! Platform-independent annotation editor controller.
//!
//! This module only handles image-space input and document edits; a UI must
//! convert screen coordinates to image coordinates and render `preview()` and
//! `selection()`. Drags stay in a temporary draft until mouse-up, producing at
//! most one history entry. Text is committed through `place_text` because this
//! controller intentionally does not own a text-entry widget. Crop is not an
//! annotation tool and is not handled here.

use crate::shapes::{Color, Point, Shape, ShapeDocument, ShapeKind, Style, Tool};

const MIN_STYLE_WIDTH: f32 = 0.25;
const MAX_STYLE_WIDTH: f32 = 256.0;
const MIN_FONT_SIZE: f32 = 1.0;
const MAX_FONT_SIZE: f32 = 512.0;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

fn valid_style(style: &Style) -> bool {
    style.width.is_finite()
        && (MIN_STYLE_WIDTH..=MAX_STYLE_WIDTH).contains(&style.width)
        && style.font_size.is_finite()
        && (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&style.font_size)
}

#[derive(Clone, Debug)]
enum Drag {
    Create {
        shape: Shape,
        selection_before: Option<usize>,
    },
    Move {
        index: usize,
        original: Shape,
        origin: Point,
        selection_before: Option<usize>,
    },
    Handle {
        index: usize,
        original: Shape,
        handle: usize,
        selection_before: Option<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct EditorController {
    document: ShapeDocument,
    tool: Tool,
    selected: Option<usize>,
    drag: Option<Drag>,
    last_position: Option<Point>,
    hit_tolerance: f32,
    next_step: i32,
    current_style: Style,
}

impl Default for EditorController {
    fn default() -> Self {
        Self {
            document: ShapeDocument::default(),
            tool: Tool::Select,
            selected: None,
            drag: None,
            last_position: None,
            hit_tolerance: 6.0,
            next_step: 1,
            current_style: Style::default(),
        }
    }
}

impl EditorController {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn tool(&self) -> Tool {
        self.tool
    }
    pub fn set_tool(&mut self, tool: Tool) {
        self.cancel_drag();
        self.tool = tool;
    }
    pub fn set_hit_tolerance(&mut self, tolerance: f32) {
        if tolerance.is_finite() && tolerance >= 0.0 {
            self.hit_tolerance = tolerance;
        }
    }
    pub fn shapes(&self) -> &[Shape] {
        self.document.shapes()
    }
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }
    pub fn selection(&self) -> Option<&Shape> {
        self.selected.and_then(|i| self.document.shapes().get(i))
    }
    /// Style applied to shapes created from this controller from now on.
    pub fn current_style(&self) -> &Style {
        &self.current_style
    }
    /// Replace the current drawing style. Width and font size must be finite
    /// and within the renderer's supported range.
    pub fn set_current_style(&mut self, style: Style) -> bool {
        if !valid_style(&style) {
            return false;
        }
        self.cancel_drag();
        self.current_style = style;
        true
    }
    pub fn set_current_color(&mut self, color: Color) {
        self.cancel_drag();
        self.current_style.color = color;
    }
    pub fn set_current_width(&mut self, width: f32) -> bool {
        if !width.is_finite() || !(MIN_STYLE_WIDTH..=MAX_STYLE_WIDTH).contains(&width) {
            return false;
        }
        self.cancel_drag();
        self.current_style.width = width;
        true
    }
    pub fn set_current_font_size(&mut self, font_size: f32) -> bool {
        if !font_size.is_finite() || !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&font_size) {
            return false;
        }
        self.cancel_drag();
        self.current_style.font_size = font_size;
        true
    }
    /// Change the selected shape's full style as one undoable document edit.
    pub fn set_selected_style(&mut self, style: Style) -> bool {
        if !valid_style(&style) {
            return false;
        }
        self.cancel_drag();
        let Some(index) = self.selected else {
            return false;
        };
        if self
            .document
            .shapes()
            .get(index)
            .is_none_or(|shape| shape.style == style)
        {
            return false;
        }
        self.document.edit(index, |shape| shape.style = style)
    }
    /// Return the selected text object's contents, if the selection is text.
    pub fn selected_text(&self) -> Option<&str> {
        match &self.selection()?.kind {
            ShapeKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }
    /// Replace selected text as one undoable edit. Text is limited to 1 MiB.
    pub fn set_selected_text(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.len() > MAX_TEXT_BYTES {
            return false;
        }
        self.cancel_drag();
        let Some(index) = self.selected else {
            return false;
        };
        let Some(shape) = self.document.shapes().get(index) else {
            return false;
        };
        if !matches!(shape.kind, ShapeKind::Text { .. }) {
            return false;
        }
        if matches!(&shape.kind, ShapeKind::Text { text: current, .. } if current == &text) {
            return false;
        }
        self.document.edit(index, |shape| {
            if let ShapeKind::Text { text: current, .. } = &mut shape.kind {
                *current = text;
            }
        })
    }

    /// Current uncommitted drag result, without changing document or history.
    pub fn preview(&self) -> Option<Shape> {
        let drag = self.drag.as_ref()?;
        match drag {
            Drag::Create { shape, .. } => Some(shape.clone()),
            Drag::Move {
                index,
                original,
                origin,
                ..
            } => {
                let _ = index;
                let last = self.last_position()?;
                let mut result = original.clone();
                result.offset(last.x - origin.x, last.y - origin.y);
                Some(result)
            }
            Drag::Handle {
                index,
                original,
                handle,
                ..
            } => {
                let _ = index;
                let mut result = original.clone();
                result.set_handle(*handle, self.last_position()?);
                Some(result)
            }
        }
    }

    // Drag endpoints are updated in `mouse_move`; this is the latest image point.
    fn last_position(&self) -> Option<Point> {
        self.last_position
    }

    /// Start a pointer gesture in image coordinates.
    pub fn mouse_down(&mut self, at: Point) {
        if !at.x.is_finite() || !at.y.is_finite() {
            return;
        }
        self.cancel_drag();
        if self.tool == Tool::Select {
            let previous = self.selected;
            if let Some(index) = self.selected {
                if let Some(shape) = self.document.shapes().get(index) {
                    if let Some((handle, _)) = shape
                        .handles()
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (i, distance(*p, at)))
                        .filter(|(_, d)| *d <= self.hit_tolerance + 4.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                    {
                        let original = shape.clone();
                        self.drag = Some(Drag::Handle {
                            index,
                            original,
                            handle,
                            selection_before: previous,
                        });
                        self.last_position = Some(at);
                        return;
                    }
                }
            }
            let hit = self
                .document
                .shapes()
                .iter()
                .enumerate()
                .rev()
                .find(|(_, shape)| shape.hit_test(at, self.hit_tolerance))
                .map(|(i, _)| i);
            self.selected = hit;
            if let Some(index) = hit {
                let original = self.document.shapes()[index].clone();
                self.drag = Some(Drag::Move {
                    index,
                    original,
                    origin: at,
                    selection_before: previous,
                });
                self.last_position = Some(at);
            }
            return;
        }

        let kind = match self.tool {
            Tool::Line => ShapeKind::Line { start: at, end: at },
            Tool::Arrow => ShapeKind::Arrow { start: at, end: at },
            Tool::Rectangle => ShapeKind::Rectangle { start: at, end: at },
            Tool::Ellipse => ShapeKind::Ellipse { start: at, end: at },
            Tool::Highlight => ShapeKind::Highlight { start: at, end: at },
            Tool::Blur => ShapeKind::Blur { start: at, end: at },
            Tool::Pen => ShapeKind::Freehand { points: vec![at] },
            Tool::Step => ShapeKind::Step {
                center: at,
                number: self.next_step,
            },
            Tool::Select | Tool::Text | Tool::Crop => return,
        };
        let mut shape = Shape::new(kind);
        shape.style = self.current_style.clone();
        self.drag = Some(Drag::Create {
            shape,
            selection_before: self.selected,
        });
        self.last_position = Some(at);
    }

    pub fn mouse_move(&mut self, at: Point) {
        if !at.x.is_finite() || !at.y.is_finite() {
            return;
        }
        self.last_position = Some(at);
        match self.drag.as_mut() {
            Some(Drag::Create { shape, .. }) => match &mut shape.kind {
                ShapeKind::Line { end, .. }
                | ShapeKind::Arrow { end, .. }
                | ShapeKind::Rectangle { end, .. }
                | ShapeKind::Ellipse { end, .. }
                | ShapeKind::Highlight { end, .. }
                | ShapeKind::Blur { end, .. } => *end = at,
                ShapeKind::Freehand { points } => {
                    if points.last().map_or(true, |p| distance(*p, at) >= 1.0) {
                        points.push(at);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Commit a gesture as one undoable operation, or discard an empty shape.
    pub fn mouse_up(&mut self, at: Point) {
        self.mouse_move(at);
        let Some(drag) = self.drag.take() else {
            self.last_position = None;
            return;
        };
        match drag {
            Drag::Create { shape, .. } => {
                if !shape.is_empty() {
                    let is_step = matches!(shape.kind, ShapeKind::Step { .. });
                    self.document.push(shape);
                    self.selected = Some(self.document.shapes().len() - 1);
                    if is_step {
                        self.next_step = self.next_step.saturating_add(1);
                    }
                }
            }
            Drag::Move {
                index,
                original,
                origin,
                ..
            } => {
                let mut moved = original.clone();
                moved.offset(at.x - origin.x, at.y - origin.y);
                self.commit_replacement(index, original, moved);
            }
            Drag::Handle {
                index,
                original,
                handle,
                ..
            } => {
                let mut resized = original.clone();
                if resized.set_handle(handle, at) {
                    self.commit_replacement(index, original, resized);
                }
            }
        }
        self.last_position = None;
    }

    /// Cancel a pending gesture without changing the document or history.
    pub fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.selected = match drag {
                Drag::Create {
                    selection_before, ..
                }
                | Drag::Move {
                    selection_before, ..
                }
                | Drag::Handle {
                    selection_before, ..
                } => selection_before.filter(|i| *i < self.document.shapes().len()),
            };
        }
        self.last_position = None;
    }

    pub fn place_text(&mut self, at: Point, text: impl Into<String>) -> bool {
        if !at.x.is_finite() || !at.y.is_finite() {
            return false;
        }
        self.cancel_drag();
        let mut shape = Shape::new(ShapeKind::Text {
            location: at,
            text: text.into(),
        });
        shape.style = self.current_style.clone();
        self.document.push(shape);
        self.selected = Some(self.document.shapes().len() - 1);
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        // A drag stores a document index. Cancel it before removal so a later
        // mouse-up cannot apply that stale index to the next shape in the list.
        self.cancel_drag();
        let Some(index) = self.selected else {
            return false;
        };
        if self.document.remove(index).is_none() {
            self.selected = None;
            return false;
        }
        self.selected = None;
        true
    }
    pub fn undo(&mut self) -> bool {
        self.cancel_drag();
        let changed = self.document.undo();
        if self
            .selected
            .is_some_and(|i| i >= self.document.shapes().len())
        {
            self.selected = None;
        }
        changed
    }
    pub fn redo(&mut self) -> bool {
        self.cancel_drag();
        let changed = self.document.redo();
        if self
            .selected
            .is_some_and(|i| i >= self.document.shapes().len())
        {
            self.selected = None;
        }
        changed
    }

    fn commit_replacement(&mut self, index: usize, original: Shape, replacement: Shape) {
        if original != replacement {
            self.document.edit(index, |shape| *shape = replacement);
        }
    }
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }
    fn rect_editor() -> EditorController {
        let mut e = EditorController::new();
        e.set_tool(Tool::Rectangle);
        e.mouse_down(p(10.0, 10.0));
        e.mouse_up(p(50.0, 40.0));
        e
    }

    #[test]
    fn creates_with_preview_then_commits_once_and_undo_redoes() {
        let mut e = EditorController::new();
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
    fn moves_as_one_step_and_delete_undo_redo() {
        let mut e = rect_editor();
        e.set_tool(Tool::Select);
        e.mouse_down(p(10.0, 25.0));
        e.mouse_move(p(20.0, 30.0));
        assert_eq!(e.shapes()[0].bounds().x, 10.0);
        assert_eq!(e.preview().unwrap().bounds().x, 20.0);
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
        let mut e = EditorController::new();
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
        let mut e = EditorController::new();
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
        let mut e = EditorController::new();
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
        let mut e = EditorController::new();
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
    fn invalid_style_parameters_are_rejected_without_changing_current_style() {
        let mut e = EditorController::new();
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
}
