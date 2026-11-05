//! Platform-independent annotation editor controller.
//!
//! This module handles image-space input and document edits; a UI must
//! convert screen coordinates to image coordinates and render `preview()` and
//! `selection()`. Drags stay in a temporary draft until mouse-up, producing at
//! most one history entry. Text is committed through `place_text` because this
//! controller intentionally does not own a text-entry widget. Image operations
//! and annotation edits share the same `Document` history.

mod history;

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

/// Build the exact shape list used for a temporary drag preview and return
/// the first shape index that cannot be reused from the cached prefix.
/// Existing-shape drags replace in place; create drags append after the
/// document, even if the editor still has a previous selection.
pub fn compose_preview_shapes(
    document_shapes: &[Shape],
    preview: Option<(Option<usize>, Shape)>,
) -> Result<(Vec<Shape>, usize), &'static str> {
    let Some((target_index, draft)) = preview else {
        return Ok((document_shapes.to_vec(), document_shapes.len()));
    };

    let mut shapes =
        Vec::with_capacity(document_shapes.len() + usize::from(target_index.is_none()));
    let stable_count = match target_index {
        Some(index) if index < document_shapes.len() => index,
        Some(_) => return Err("Vorschau-Ziel liegt außerhalb der Dokumentformen."),
        None => document_shapes.len(),
    };
    shapes.extend(document_shapes.iter().cloned());
    if let Some(index) = target_index {
        shapes[index] = draft;
    } else {
        shapes.push(draft);
    }
    Ok((shapes, stable_count))
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
    /// Select a tool. Switching to the marker selects the reference yellow
    /// while the default palette red is active, and recolors a selected shape
    /// as one undoable document edit. Returns whether the document changed.
    pub fn set_tool(&mut self, tool: Tool) -> bool {
        self.cancel_drag();
        self.tool = tool;
        if tool != Tool::Highlight || self.current_style.color != Color::PALETTE_RED {
            return false;
        }
        self.current_style.color = Color::MARKER_YELLOW;
        let Some(mut replacement) = self.selection().map(|shape| shape.style.clone()) else {
            return false;
        };
        replacement.color = Color::MARKER_YELLOW;
        self.set_selected_style(replacement)
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

    /// Return the topmost hit text object's index and contents, in image space.
    /// The caller can use the index to edit the same object without changing
    /// the current selection, matching the canvas Text-tool behavior.
    pub fn text_at(&self, at: Point) -> Option<(usize, String)> {
        if !at.x.is_finite() || !at.y.is_finite() {
            return None;
        }
        self.document
            .shapes()
            .iter()
            .enumerate()
            .rev()
            .find(|(_, shape)| shape.hit_test(at, self.hit_tolerance))
            .and_then(|(index, shape)| match &shape.kind {
                ShapeKind::Text { text, .. } => Some((index, text.clone())),
                _ => None,
            })
    }

    /// Whether the pointer is on a handle of the selected shape. This lets a
    /// UI preserve handle priority before dispatching a Text-tool click.
    pub fn selected_handle_hit(&self, at: Point) -> bool {
        if !at.x.is_finite() || !at.y.is_finite() {
            return false;
        }
        self.selected
            .and_then(|index| self.document.shapes().get(index))
            .is_some_and(|shape| {
                shape
                    .handles()
                    .iter()
                    .any(|handle| distance(*handle, at) <= self.hit_tolerance + 4.0)
            })
    }

    /// Replace text at a document index as one undoable edit. Empty or
    /// whitespace-only input removes the object, like the original canvas.
    pub fn set_text_at(&mut self, index: usize, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.len() > MAX_TEXT_BYTES {
            return false;
        }
        self.cancel_drag();
        let Some(shape) = self.document.shapes().get(index) else {
            return false;
        };
        let ShapeKind::Text { text: current, .. } = &shape.kind else {
            return false;
        };
        if current == &text {
            return false;
        }
        if text.trim().is_empty() {
            match self.document.remove_shape(index) {
                Ok(_) => {
                    self.selected = None;
                    return true;
                }
                Err(error) => {
                    self.last_error = Some(error);
                    return false;
                }
            }
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

    /// Document shape replaced by the current preview, if the drag moves or
    /// resizes an existing shape. Create previews have no target index.
    pub fn preview_index(&self) -> Option<usize> {
        match self.drag.as_ref()? {
            Drag::Move { index, .. } | Drag::Handle { index, .. } => Some(*index),
            Drag::Create { .. } => None,
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
        // The original canvas checks the selected shape's handles before it
        // dispatches to the active tool. Keep that priority so drawing tools
        // can resize the current selection when the pointer starts on a handle.
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
                        selection_before: self.selected,
                    });
                    self.last_position = Some(at);
                    return;
                }
            }
        }
        if self.tool == Tool::Select {
            let previous = self.selected;
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
        self.mouse_move_with_shift(at, false);
    }

    /// Update a pointer gesture, applying the original editor's Shift constraint
    /// to two-point annotations while the modifier is held.
    pub fn mouse_move_with_shift(&mut self, at: Point, shift: bool) {
        if !at.x.is_finite() || !at.y.is_finite() {
            return;
        }
        self.last_position = Some(at);
        if let Some(Drag::Create { shape, .. }) = self.drag.as_mut() {
            match &mut shape.kind {
                ShapeKind::Line { start, end } | ShapeKind::Arrow { start, end } => {
                    *end = if shift {
                        constrain_endpoint(*start, at, false)
                    } else {
                        at
                    };
                }
                ShapeKind::Rectangle { start, end }
                | ShapeKind::Ellipse { start, end }
                | ShapeKind::Highlight { start, end }
                | ShapeKind::Blur { start, end } => {
                    *end = if shift {
                        constrain_endpoint(*start, at, true)
                    } else {
                        at
                    };
                }
                ShapeKind::Freehand { points }
                    if points.last().is_none_or(|p| distance(*p, at) >= 1.0) =>
                {
                    points.push(at);
                }
                _ => {}
            }
        }
    }

    /// Commit a gesture as one undoable operation, or discard an empty shape.
    pub fn mouse_up(&mut self, at: Point) {
        self.mouse_up_with_shift(at, false);
    }

    /// Commit a gesture using the modifier state at mouse-up as the final point.
    pub fn mouse_up_with_shift(&mut self, at: Point, shift: bool) {
        self.mouse_move_with_shift(at, shift);
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
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Match `EditorCanvas.Constrain`: boxes use the larger absolute delta, while
/// lines and arrows snap to the nearest 45-degree angle. C# Math.Round uses
/// midpoint-to-even, so keep that rule explicit for negative and positive ties.
fn constrain_endpoint(start: Point, at: Point, square: bool) -> Point {
    let dx = at.x - start.x;
    let dy = at.y - start.y;
    if square {
        let distance = dx.abs().max(dy.abs());
        let sign = |value: f32| {
            if value > 0.0 {
                1.0
            } else if value < 0.0 {
                -1.0
            } else {
                0.0
            }
        };
        return Point::new(start.x + sign(dx) * distance, start.y + sign(dy) * distance);
    }

    let angle = (dy as f64).atan2(dx as f64);
    let step = std::f64::consts::FRAC_PI_4;
    let snapped = round_ties_even(angle / step) * step;
    let length = (dx * dx + dy * dy).sqrt();
    Point::new(
        start.x + (snapped.cos() as f32) * length,
        start.y + (snapped.sin() as f32) * length,
    )
}

fn round_ties_even(value: f64) -> f64 {
    let lower = value.floor();
    let fraction = value - lower;
    if fraction < 0.5 {
        lower
    } else if fraction > 0.5 {
        lower + 1.0
    } else if (lower as i64) % 2 == 0 {
        lower
    } else {
        lower + 1.0
    }
}

#[cfg(test)]
mod tests;
