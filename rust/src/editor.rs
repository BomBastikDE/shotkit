//! Platform-independent annotation editor controller.
//!
//! This module handles image-space input and document edits; a UI must
//! convert screen coordinates to image coordinates and render `preview()` and
//! `selection()`. Drags stay in a temporary draft until mouse-up, producing at
//! most one history entry. Text is committed through `place_text` because this
//! controller intentionally does not own a text-entry widget. Image operations
//! and annotation edits share the same `Document` history.

use crate::{
    document::{Document, DocumentError},
    image_edit::{CropRect, FrameOptions},
    shapes::{Color, Point, Shape, ShapeKind, Style, Tool},
    stitch::Image,
};
use std::sync::Arc;

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
    document: Document,
    tool: Tool,
    selected: Option<usize>,
    drag: Option<Drag>,
    last_position: Option<Point>,
    hit_tolerance: f32,
    next_step: i32,
    current_style: Style,
    last_error: Option<DocumentError>,
}

impl Default for EditorController {
    fn default() -> Self {
        // Compatibility for existing UI construction sites. The real image
        // should be supplied with `with_image` when the controller is installed.
        Self::with_image(Image::new(1, 1, vec![0; 4]).expect("valid fallback image"))
            .expect("fallback image is valid")
    }
}

impl EditorController {
    pub fn with_image(image: Image) -> Result<Self, DocumentError> {
        Self::with_image_recover(image).map_err(|(error, _image)| error)
    }

    /// Construct a controller while returning the original image if document
    /// validation fails.
    pub fn with_image_recover(image: Image) -> Result<Self, (DocumentError, Image)> {
        let document = Document::new_recover(image)?;
        Ok(Self {
            document,
            tool: Tool::Select,
            selected: None,
            drag: None,
            last_position: None,
            hit_tolerance: 6.0,
            next_step: 1,
            current_style: Style::default(),
            last_error: None,
        })
    }
    /// Compatibility constructor. Prefer `with_image` for a controller tied
    /// to the actual canvas image.
    pub fn new() -> Self {
        Self::default()
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }
    pub fn image_arc(&self) -> Arc<Image> {
        self.document.image_arc()
    }
    /// Return and clear the last failed document mutation. Boolean/void UI
    /// methods retain their existing signatures while making failures visible.
    pub fn take_error(&mut self) -> Option<DocumentError> {
        self.last_error.take()
    }
    pub fn crop(&mut self, rect: CropRect) -> Result<(), DocumentError> {
        self.cancel_drag();
        self.document.crop(rect)
    }
    pub fn add_frame(&mut self, options: FrameOptions) -> Result<(), DocumentError> {
        self.cancel_drag();
        self.document.add_frame(options)
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
    /// Clear the current selection and discard any in-progress gesture.
    pub fn clear_selection(&mut self) {
        self.cancel_drag();
        self.selected = None;
    }
    /// Move the selected shape by an image-space offset as one undoable edit.
    pub fn nudge_selected(&mut self, dx: f32, dy: f32) -> bool {
        if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
            return false;
        }
        // The UI owns the corresponding pointer capture. Refuse a keyboard
        // nudge during a pointer gesture instead of cancelling only the model
        // side and leaving that capture active.
        if self.drag.is_some() {
            return false;
        }
        let Some(index) = self.selected else {
            return false;
        };
        let Some(original) = self.document.shapes().get(index).cloned() else {
            self.selected = None;
            return false;
        };
        let mut moved = original.clone();
        moved.offset(dx, dy);
        self.commit_replacement(index, original, moved)
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
        self.edit_shape(index, |shape| shape.style = style)
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
        self.edit_shape(index, |shape| {
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
                    let selection_before = self.selected;
                    match self.document.add_shape(shape) {
                        Ok(()) => {
                            self.selected = Some(self.document.shapes().len() - 1);
                            if is_step {
                                self.next_step = self.next_step.saturating_add(1);
                            }
                        }
                        Err(error) => {
                            self.last_error = Some(error);
                            self.selected = selection_before;
                        }
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
        let selection_before = self.selected;
        match self.document.add_shape(shape) {
            Ok(()) => {
                self.selected = Some(self.document.shapes().len() - 1);
                true
            }
            Err(error) => {
                self.last_error = Some(error);
                self.selected = selection_before;
                false
            }
        }
    }

    pub fn delete_selected(&mut self) -> bool {
        // A drag stores a document index. Cancel it before removal so a later
        // mouse-up cannot apply that stale index to the next shape in the list.
        self.cancel_drag();
        let Some(index) = self.selected else {
            return false;
        };
        match self.document.remove_shape(index) {
            Ok(_) => {}
            Err(error) => {
                self.last_error = Some(error);
                self.selected = Some(index);
                return false;
            }
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

    fn commit_replacement(&mut self, index: usize, original: Shape, replacement: Shape) -> bool {
        if original != replacement {
            self.edit_shape(index, |shape| *shape = replacement)
        } else {
            false
        }
    }

    fn edit_shape<F>(&mut self, index: usize, edit: F) -> bool
    where
        F: FnOnce(&mut Shape),
    {
        match self.document.edit_shape(index, edit) {
            Ok(()) => true,
            Err(error) => {
                self.last_error = Some(error);
                false
            }
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
    fn editor() -> EditorController {
        EditorController::with_image(Image::new(100, 100, vec![128; 100 * 100 * 4]).unwrap())
            .unwrap()
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
}
