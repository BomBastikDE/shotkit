//! Shared image-and-annotation document with one bounded undo/redo history.
//!
//! History snapshots hold image pixels in `Arc<Image>` so shape-only edits do
//! not duplicate the pixel allocation. Image operations create a new `Image`;
//! prior images remain available through the same history. A transaction can
//! stage a sequence of edits, then commit it as one history step or abort it
//! back to the saved state.

use crate::{
    image_edit::{self, CropRect, FrameOptions, ImageEditError},
    shapes::{Color, Point, Shape},
    stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION},
};
use std::{
    collections::{HashSet, VecDeque},
    mem::size_of,
    sync::Arc,
};

pub const DEFAULT_HISTORY_BUDGET: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentError {
    Image(ImageEditError),
    InvalidImage,
    HistoryBudgetExceeded,
    TransactionAlreadyOpen,
    NoTransaction,
    InvalidShapeIndex,
    AllocationFailed,
}
impl From<ImageEditError> for DocumentError {
    fn from(value: ImageEditError) -> Self {
        Self::Image(value)
    }
}

/// Alignment of the existing image within a resized canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasAlignment {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
struct State {
    image: Arc<Image>,
    shapes: Vec<Shape>,
    revision: Arc<()>,
}

#[derive(Clone, Debug)]
pub struct Document {
    current: State,
    undo: VecDeque<State>,
    redo: Vec<State>,
    staged_before: Option<State>,
    saved_revision: Option<Arc<()>>,
    history_budget: usize,
}

impl Document {
    pub fn new(image: Image) -> Result<Self, DocumentError> {
        Self::new_recover(image).map_err(|(error, _image)| error)
    }

    /// Construct a document while returning the original image if validation
    /// fails, so callers can retain or inspect rejected input.
    pub fn new_recover(image: Image) -> Result<Self, (DocumentError, Image)> {
        Self::with_history_budget_recover(image, DEFAULT_HISTORY_BUDGET)
    }

    pub fn with_history_budget(
        image: Image,
        history_budget_bytes: usize,
    ) -> Result<Self, DocumentError> {
        Self::with_history_budget_recover(image, history_budget_bytes)
            .map_err(|(error, _image)| error)
    }

    fn with_history_budget_recover(
        image: Image,
        history_budget_bytes: usize,
    ) -> Result<Self, (DocumentError, Image)> {
        if let Err(error) = validate_image(&image) {
            return Err((error, image));
        }
        Ok(Self {
            current: State {
                image: Arc::new(image),
                shapes: Vec::new(),
                revision: Arc::new(()),
            },
            undo: VecDeque::new(),
            redo: Vec::new(),
            staged_before: None,
            saved_revision: None,
            history_budget: history_budget_bytes,
        })
    }

    pub fn image(&self) -> &Image {
        &self.current.image
    }
    pub fn image_arc(&self) -> Arc<Image> {
        Arc::clone(&self.current.image)
    }
    pub fn undo_image_arc(&self) -> Option<Arc<Image>> {
        self.undo.back().map(|s| Arc::clone(&s.image))
    }
    pub fn redo_image_arc(&self) -> Option<Arc<Image>> {
        self.redo.last().map(|s| Arc::clone(&s.image))
    }
    pub fn shapes(&self) -> &[Shape] {
        &self.current.shapes
    }
    /// Record a successfully saved or opened committed document state.
    pub fn mark_saved(&mut self) -> bool {
        if self.staged_before.is_some() {
            return false;
        }
        self.saved_revision = Some(Arc::clone(&self.current.revision));
        true
    }
    pub fn is_dirty(&self) -> bool {
        self.saved_revision.as_ref().is_none_or(|saved| {
            !Arc::ptr_eq(saved, &self.current.revision)
                || self
                    .staged_before
                    .as_ref()
                    .is_some_and(|before| !states_equal(before, &self.current))
        })
    }
    pub fn can_undo(&self) -> bool {
        self.staged_before.is_none() && !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        self.staged_before.is_none() && !self.redo.is_empty()
    }
    pub fn history_budget(&self) -> usize {
        self.history_budget
    }

    /// Conservative estimate of undo/redo storage. Pixel buffers shared with
    /// the current state are not charged; other image allocations are counted
    /// once. Snapshot slots, vector capacities, Arc headers, and allocator
    /// overhead are included.
    pub fn history_bytes(&self) -> usize {
        let mut seen = HashSet::new();
        seen.insert(Arc::as_ptr(&self.current.image) as usize);
        self.undo
            .iter()
            .chain(self.redo.iter())
            .fold(0usize, |total, state| {
                let image_bytes = if seen.insert(Arc::as_ptr(&state.image) as usize) {
                    image_allocation_bytes(&state.image)
                } else {
                    0
                };
                total
                    .saturating_add(size_of::<State>())
                    .saturating_add(revision_allocation_bytes())
                    .saturating_add(image_bytes)
                    .saturating_add(shapes_allocation_bytes(&state.shapes))
            })
            .saturating_add(container_bytes(self.undo.capacity(), size_of::<State>()))
            .saturating_add(container_bytes(self.redo.capacity(), size_of::<State>()))
    }

    pub fn set_history_budget(&mut self, bytes: usize) {
        self.history_budget = bytes;
        self.trim_history();
    }

    /// Start a rollback-safe group of edits. Nested transactions are rejected.
    pub fn begin_edit(&mut self) -> Result<(), DocumentError> {
        if self.staged_before.is_some() {
            return Err(DocumentError::TransactionAlreadyOpen);
        }
        self.staged_before = Some(self.current.clone());
        Ok(())
    }

    /// Commit all staged changes as one undo step. If the saved state cannot fit
    /// the history budget, the transaction stays open so the caller may abort.
    pub fn commit_edit(&mut self) -> Result<bool, DocumentError> {
        let Some(before) = self.staged_before.as_ref() else {
            return Err(DocumentError::NoTransaction);
        };
        if states_equal(before, &self.current) {
            self.staged_before = None;
            return Ok(false);
        }
        self.record_snapshot(before.clone(), Arc::clone(&self.current.image))?;
        self.current.revision = Arc::new(());
        self.staged_before = None;
        self.trim_history();
        Ok(true)
    }

    /// Discard all staged changes and restore image and shapes atomically.
    pub fn abort_edit(&mut self) -> Result<(), DocumentError> {
        let Some(before) = self.staged_before.take() else {
            return Err(DocumentError::NoTransaction);
        };
        self.current = before;
        Ok(())
    }

    pub fn add_shape(&mut self, shape: Shape) -> Result<(), DocumentError> {
        let mut next = self.current.clone();
        next.shapes.push(shape);
        self.install(next)
    }

    pub fn remove_shape(&mut self, index: usize) -> Result<Shape, DocumentError> {
        if index >= self.current.shapes.len() {
            return Err(DocumentError::InvalidShapeIndex);
        }
        let mut next = self.current.clone();
        let removed = next.shapes.remove(index);
        self.install(next)?;
        Ok(removed)
    }

    pub fn edit_shape<F>(&mut self, index: usize, edit: F) -> Result<(), DocumentError>
    where
        F: FnOnce(&mut Shape),
    {
        if index >= self.current.shapes.len() {
            return Err(DocumentError::InvalidShapeIndex);
        }
        let mut next = self.current.clone();
        edit(&mut next.shapes[index]);
        self.install(next)
    }

    /// Replace only image pixels while keeping annotation coordinates unchanged.
    pub fn replace_image(&mut self, image: Image) -> Result<(), DocumentError> {
        validate_image(&image)?;
        let mut next = self.current.clone();
        next.image = Arc::new(image);
        self.install(next)
    }

    /// Crop pixels and shift every annotation by the crop origin in one step.
    pub fn crop(&mut self, rect: CropRect) -> Result<(), DocumentError> {
        let image = Arc::new(image_edit::crop(&self.current.image, rect)?);
        let mut next = self.current.clone();
        next.image = image;
        for shape in &mut next.shapes {
            shape.offset(-(rect.x as f32), -(rect.y as f32));
        }
        self.install(next)
    }

    /// Add frame/shadow and move annotations to the new image-content origin.
    pub fn add_frame(&mut self, options: FrameOptions) -> Result<(), DocumentError> {
        let image = Arc::new(image_edit::add_frame(&self.current.image, options)?);
        let shadow = options.shadow;
        let margin_x = shadow.map_or(0usize, |s| {
            s.blur_radius
                + if s.offset_x < 0 {
                    s.offset_x.unsigned_abs() as usize
                } else {
                    0
                }
        });
        let margin_y = shadow.map_or(0usize, |s| {
            s.blur_radius
                + if s.offset_y < 0 {
                    s.offset_y.unsigned_abs() as usize
                } else {
                    0
                }
        });
        let dx = (margin_x + options.width) as f32;
        let dy = (margin_y + options.width) as f32;
        let mut next = self.current.clone();
        next.image = image;
        for shape in &mut next.shapes {
            shape.offset(dx, dy);
        }
        self.install(next)
    }

    /// Resize the canvas without resampling. Pixels outside a smaller canvas
    /// are clipped; shapes remain editable and are shifted by the same origin.
    /// Center alignment assigns an odd extra pixel to the end side.
    pub fn resize_canvas(
        &mut self,
        width: usize,
        height: usize,
        horizontal: CanvasAlignment,
        vertical: CanvasAlignment,
        fill: Color,
    ) -> Result<(), DocumentError> {
        let bytes = checked_edit_size(width, height)?;
        let source = &self.current.image;
        if source.width == width && source.height == height {
            return Ok(());
        }
        let (source_x, dest_x, copy_width) = canvas_axis(source.width, width, horizontal);
        let (source_y, dest_y, copy_height) = canvas_axis(source.height, height, vertical);
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bytes)
            .map_err(|_| DocumentError::AllocationFailed)?;
        let bgra = [fill.b, fill.g, fill.r, fill.a];
        for _ in 0..bytes / 4 {
            pixels.extend_from_slice(&bgra);
        }
        for y in 0..copy_height {
            let src = ((source_y + y) * source.width + source_x) * 4;
            let dst = ((dest_y + y) * width + dest_x) * 4;
            let count = copy_width * 4;
            pixels[dst..dst + count].copy_from_slice(&source.pixels[src..src + count]);
        }
        let mut next = self.current.clone();
        next.image = Arc::new(Image {
            width,
            height,
            pixels,
        });
        let dx = dest_x as f32 - source_x as f32;
        let dy = dest_y as f32 - source_y as f32;
        for shape in &mut next.shapes {
            shape.offset(dx, dy);
        }
        self.install(next)
    }

    pub fn undo(&mut self) -> bool {
        if self.staged_before.is_some() {
            return false;
        }
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        self.trim_history();
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.staged_before.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push_back(std::mem::replace(&mut self.current, next));
        self.trim_history();
        true
    }

    fn install(&mut self, mut next: State) -> Result<(), DocumentError> {
        if states_equal(&next, &self.current) {
            return Ok(());
        }
        if self.staged_before.is_none() {
            self.record_snapshot(self.current.clone(), Arc::clone(&next.image))?;
            next.revision = Arc::new(());
        }
        self.current = next;
        if self.staged_before.is_none() {
            self.trim_history();
        }
        Ok(())
    }

    fn record_snapshot(
        &mut self,
        snapshot: State,
        next_image: Arc<Image>,
    ) -> Result<(), DocumentError> {
        if self.snapshot_bytes_after_redo_clear(&snapshot, &next_image) > self.history_budget {
            return Err(DocumentError::HistoryBudgetExceeded);
        }
        self.redo.clear();
        self.redo.shrink_to_fit();
        self.undo.push_back(snapshot);
        Ok(())
    }

    fn trim_history(&mut self) {
        while self.history_bytes() > self.history_budget {
            if !self.undo.is_empty() {
                self.undo.pop_front();
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
            self.undo.shrink_to_fit();
            self.redo.shrink_to_fit();
        }
        self.undo.shrink_to_fit();
        self.redo.shrink_to_fit();
    }

    fn snapshot_bytes_after_redo_clear(&self, snapshot: &State, next_image: &Arc<Image>) -> usize {
        // Older snapshots may all need eviction, so only sharing with the
        // resulting current image can reduce this snapshot's minimum cost.
        let image_bytes = if !Arc::ptr_eq(next_image, &snapshot.image) {
            image_allocation_bytes(&snapshot.image)
        } else {
            0
        };
        size_of::<State>()
            .saturating_add(revision_allocation_bytes())
            .saturating_add(shapes_allocation_bytes(&snapshot.shapes))
            .saturating_add(image_bytes)
            .saturating_add(container_bytes(4, size_of::<State>()))
    }
}

const ALLOCATION_OVERHEAD: usize = 4 * size_of::<usize>();

fn revision_allocation_bytes() -> usize {
    2 * size_of::<usize>() + ALLOCATION_OVERHEAD
}

fn container_bytes(capacity: usize, element_size: usize) -> usize {
    if capacity == 0 {
        0
    } else {
        capacity
            .saturating_mul(element_size)
            .saturating_add(ALLOCATION_OVERHEAD)
    }
}

fn image_allocation_bytes(image: &Image) -> usize {
    size_of::<Image>()
        .saturating_add(2 * size_of::<usize>())
        .saturating_add(container_bytes(image.pixels.capacity(), size_of::<u8>()))
}

fn shapes_allocation_bytes(shapes: &Vec<Shape>) -> usize {
    shapes.iter().fold(
        container_bytes(shapes.capacity(), size_of::<Shape>()),
        |sum, shape| {
            let dynamic = match &shape.kind {
                crate::shapes::ShapeKind::Freehand { points } => {
                    container_bytes(points.capacity(), size_of::<Point>())
                }
                crate::shapes::ShapeKind::Text { text, .. } => {
                    container_bytes(text.capacity(), size_of::<u8>())
                }
                _ => 0,
            };
            sum.saturating_add(dynamic)
        },
    )
}

fn checked_edit_size(width: usize, height: usize) -> Result<usize, DocumentError> {
    if width == 0
        || height == 0
        || width > MAX_EDITABLE_IMAGE_DIMENSION
        || height > MAX_EDITABLE_IMAGE_DIMENSION
    {
        return Err(DocumentError::InvalidImage);
    }
    let bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(DocumentError::InvalidImage)?;
    if bytes > 512 * 1024 * 1024 {
        return Err(DocumentError::InvalidImage);
    }
    Ok(bytes)
}

fn canvas_axis(
    source: usize,
    destination: usize,
    alignment: CanvasAlignment,
) -> (usize, usize, usize) {
    let difference = source.abs_diff(destination);
    let offset = match alignment {
        CanvasAlignment::Start => 0,
        CanvasAlignment::Center => difference / 2,
        CanvasAlignment::End => difference,
    };
    if source > destination {
        (offset, 0, destination)
    } else {
        (0, offset, source)
    }
}

fn validate_image(image: &Image) -> Result<(), DocumentError> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(DocumentError::InvalidImage)?;
    if image.width == 0
        || image.height == 0
        || bytes != image.pixels.len()
        || bytes > 512 * 1024 * 1024
    {
        return Err(DocumentError::InvalidImage);
    }
    Ok(())
}

fn states_equal(left: &State, right: &State) -> bool {
    (Arc::ptr_eq(&left.image, &right.image) || left.image.as_ref() == right.image.as_ref())
        && left.shapes == right.shapes
}

#[cfg(test)]
mod tests;
