//! Shared image-and-annotation document with one bounded undo/redo history.
//!
//! History snapshots hold image pixels in `Arc<Image>` so shape-only edits do
//! not duplicate the pixel allocation. Image operations create a new `Image`;
//! prior images remain available through the same history. A transaction can
//! stage a sequence of edits, then commit it as one history step or abort it
//! back to the saved state.

use crate::{
    image_edit::{self, CropRect, FrameOptions, ImageEditError},
    shapes::{Point, Shape},
    stitch::Image,
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
}
impl From<ImageEditError> for DocumentError {
    fn from(value: ImageEditError) -> Self {
        Self::Image(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct State {
    image: Arc<Image>,
    shapes: Vec<Shape>,
}

#[derive(Clone, Debug)]
pub struct Document {
    current: State,
    undo: VecDeque<State>,
    redo: Vec<State>,
    staged_before: Option<State>,
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
            },
            undo: VecDeque::new(),
            redo: Vec::new(),
            staged_before: None,
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
        self.record_snapshot(before.clone())?;
        self.staged_before = None;
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

    fn install(&mut self, next: State) -> Result<(), DocumentError> {
        if states_equal(&next, &self.current) {
            return Ok(());
        }
        if self.staged_before.is_none() {
            self.record_snapshot(self.current.clone())?;
        }
        self.current = next;
        Ok(())
    }

    fn record_snapshot(&mut self, snapshot: State) -> Result<(), DocumentError> {
        if self.snapshot_bytes_after_redo_clear(&snapshot) > self.history_budget {
            return Err(DocumentError::HistoryBudgetExceeded);
        }
        self.redo.clear();
        self.redo.shrink_to_fit();
        self.undo.push_back(snapshot);
        self.trim_history();
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

    fn snapshot_bytes_after_redo_clear(&self, snapshot: &State) -> usize {
        let mut seen = HashSet::new();
        seen.insert(Arc::as_ptr(&self.current.image) as usize);
        for state in &self.undo {
            seen.insert(Arc::as_ptr(&state.image) as usize);
        }
        let image_bytes = if seen.insert(Arc::as_ptr(&snapshot.image) as usize) {
            image_allocation_bytes(&snapshot.image)
        } else {
            0
        };
        size_of::<State>()
            .saturating_add(shapes_allocation_bytes(&snapshot.shapes))
            .saturating_add(image_bytes)
            .saturating_add(container_bytes(4, size_of::<State>()))
    }
}

const ALLOCATION_OVERHEAD: usize = 4 * size_of::<usize>();

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
mod tests {
    use super::*;
    use crate::{
        image_edit::{FrameOptions, ShadowOptions},
        shapes::{Point, ShapeKind},
    };

    fn image() -> Image {
        Image::new(
            3,
            2,
            vec![
                0, 0, 0, 0, 1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15,
                0,
            ],
        )
        .unwrap()
    }
    fn shape() -> Shape {
        Shape::new(ShapeKind::Line {
            start: Point::new(2.0, 3.0),
            end: Point::new(8.0, 9.0),
        })
    }

    #[test]
    fn peek_image_follows_undo_and_redo_order() {
        let mut d = Document::new(image()).unwrap();
        assert!(d.undo_image_arc().is_none());
        let original = d.image_arc();
        d.crop(CropRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        })
        .unwrap();
        let cropped = d.image_arc();
        assert!(Arc::ptr_eq(&original, &d.undo_image_arc().unwrap()));
        assert!(d.undo());
        assert!(Arc::ptr_eq(&cropped, &d.redo_image_arc().unwrap()));
    }
    #[test]
    fn shared_history_orders_shape_and_image_changes() {
        let mut d = Document::new(image()).unwrap();
        d.add_shape(shape()).unwrap();
        assert_eq!(d.shapes().len(), 1);
        d.crop(CropRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        })
        .unwrap();
        assert_eq!((d.image().width, d.image().height), (2, 2));
        assert_eq!(d.shapes()[0].bounds().x, 1.0);
        assert!(d.undo()); // undo image crop and shape offset together
        assert_eq!((d.image().width, d.image().height), (3, 2));
        assert_eq!(d.shapes()[0].bounds().x, 2.0);
        assert!(d.undo());
        assert!(d.shapes().is_empty());
        assert!(d.redo());
        assert_eq!(d.shapes().len(), 1);
        assert!(d.redo());
        assert_eq!((d.image().width, d.image().height), (2, 2));
    }

    #[test]
    fn frame_keeps_transparent_source_pixel_and_offsets_shapes() {
        let mut d = Document::new(image()).unwrap();
        d.add_shape(shape()).unwrap();
        d.add_frame(FrameOptions {
            width: 1,
            color: crate::shapes::Color::rgba(255, 0, 0, 255),
            shadow: Some(ShadowOptions {
                offset_x: 2,
                offset_y: 0,
                blur_radius: 1,
                opacity: 80,
            }),
        })
        .unwrap();
        assert_eq!(d.shapes()[0].bounds().x, 4.0); // margin left is 1 plus frame width
        let content_x = 1 + 1;
        let content_y = 1 + 1;
        let offset = (content_y * d.image().width + content_x) * 4;
        assert_eq!(&d.image().pixels[offset..offset + 4], &image().pixels[..4]);
        assert!(d.undo());
        assert_eq!(d.shapes()[0].bounds().x, 2.0);
        assert_eq!((d.image().width, d.image().height), (3, 2));
    }

    #[test]
    fn transaction_abort_restores_image_and_shapes_and_adds_no_history() {
        let mut d = Document::new(image()).unwrap();
        d.begin_edit().unwrap();
        d.add_shape(shape()).unwrap();
        d.crop(CropRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        })
        .unwrap();
        assert_eq!(d.shapes().len(), 1);
        assert_eq!(d.image().width, 2);
        d.abort_edit().unwrap();
        assert!(d.shapes().is_empty());
        assert_eq!((d.image().width, d.image().height), (3, 2));
        assert!(!d.can_undo());
        assert!(!d.undo());
    }

    #[test]
    fn budget_is_enforced_and_oldest_snapshots_are_evicted() {
        let mut d = Document::with_history_budget(image(), 8192).unwrap();
        for i in 0..100 {
            d.add_shape(Shape::new(ShapeKind::Step {
                center: Point::new(i as f32, 0.0),
                number: i,
            }))
            .unwrap();
        }
        assert!(d.history_bytes() <= d.history_budget());
        let mut count = 0;
        while d.undo() {
            count += 1;
        }
        assert!(count > 0 && count < 100);
        let mut tight = Document::with_history_budget(image(), 1).unwrap();
        assert_eq!(
            tight.add_shape(shape()),
            Err(DocumentError::HistoryBudgetExceeded)
        );
        assert!(tight.shapes().is_empty());
        assert!(!tight.can_undo());
    }

    #[test]
    fn large_current_image_is_not_charged_again_for_shape_history() {
        let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
        let mut d = Document::with_history_budget(large, 512).unwrap();
        d.add_shape(shape()).unwrap();
        assert!(d.history_bytes() <= d.history_budget());
        assert!(d.undo());
        assert!(d.shapes().is_empty());
    }

    #[test]
    fn undo_evicts_redo_state_that_cannot_fit_budget() {
        let mut d = Document::with_history_budget(image(), 1024).unwrap();
        d.replace_image(Image::new(16, 16, vec![9; 16 * 16 * 4]).unwrap())
            .unwrap();
        assert!(d.undo());
        assert_eq!((d.image().width, d.image().height), (3, 2));
        assert!(!d.can_redo());
        assert!(d.history_bytes() <= d.history_budget());
    }

    #[test]
    fn redo_is_preserved_when_result_fits_and_stays_within_budget() {
        let mut d = Document::with_history_budget(image(), 4096).unwrap();
        let replacement = Image::new(4, 4, vec![11; 4 * 4 * 4]).unwrap();
        d.replace_image(replacement.clone()).unwrap();
        assert!(d.undo());
        assert!(d.can_redo());
        assert!(d.history_bytes() <= d.history_budget());
        assert!(d.redo());
        assert_eq!(d.image(), &replacement);
        assert!(d.history_bytes() <= d.history_budget());
    }

    #[test]
    fn tiny_image_history_counts_snapshot_and_allocation_overhead() {
        let mut d = Document::with_history_budget(image(), 4096).unwrap();
        for value in 0..100u8 {
            d.replace_image(Image::new(1, 1, vec![value; 4]).unwrap())
                .unwrap();
            assert!(d.history_bytes() <= d.history_budget());
        }
        assert!(d.undo.len() < 100);
        assert!(d.history_bytes() >= d.undo.len() * size_of::<State>());
        assert!(d.history_bytes() > d.undo.len() * 4);
    }

    #[test]
    fn invalid_images_are_rejected_without_replacing_current_state() {
        let invalid = Image {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 4],
        };
        assert!(matches!(
            Document::new(invalid.clone()),
            Err(DocumentError::InvalidImage)
        ));

        let mut d = Document::new(image()).unwrap();
        assert_eq!(d.replace_image(invalid), Err(DocumentError::InvalidImage));
        assert_eq!((d.image().width, d.image().height), (3, 2));
        assert_eq!(d.image().pixels, image().pixels);
        assert!(!d.can_undo());
    }

    #[test]
    fn recover_constructor_returns_invalid_image_unchanged() {
        let invalid = Image {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 4],
        };
        let (error, recovered) = Document::new_recover(invalid.clone()).unwrap_err();
        assert_eq!(error, DocumentError::InvalidImage);
        assert_eq!(recovered, invalid);

        let valid = image();
        assert_eq!(
            Document::new_recover(valid.clone()).unwrap().image(),
            &valid
        );
    }

    #[test]
    fn failed_transaction_commit_keeps_state_open_until_abort() {
        let mut d = Document::with_history_budget(image(), 512).unwrap();
        d.begin_edit().unwrap();
        d.add_shape(shape()).unwrap();
        d.set_history_budget(1);

        assert_eq!(d.commit_edit(), Err(DocumentError::HistoryBudgetExceeded));
        assert_eq!(d.shapes().len(), 1);
        assert_eq!(d.begin_edit(), Err(DocumentError::TransactionAlreadyOpen));
        d.abort_edit().unwrap();
        assert!(d.shapes().is_empty());
        assert_eq!(d.image().pixels, image().pixels);
        assert!(!d.can_undo());
    }
}
