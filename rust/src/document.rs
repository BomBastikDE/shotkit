//! Shared image-and-annotation document with one bounded undo/redo history.
//!
//! History snapshots hold image pixels in `ImageHandle` so shape-only edits do
//! not duplicate the pixel allocation. Image operations create a new `Image`;
//! prior images remain available through the same history. A transaction can
//! stage a sequence of edits, then commit it as one history step or abort it
//! back to the saved state.

use crate::{
    image_edit::{self, CropRect, FrameOptions, ImageEditError},
    image_handle::ImageHandle,
    import_admission::TrackedImage,
    resource_budget::{AdmissionClass, ResourceBudget},
    shapes::{Color, Point, Shape},
    stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::Arc,
};

pub const DEFAULT_HISTORY_BUDGET: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentError {
    Image(ImageEditError),
    InvalidImage,
    HistoryBudgetExceeded,
    HistoryNavigationBudgetExceeded,
    TransactionAlreadyOpen,
    NoTransaction,
    InvalidShapeIndex,
    RandomUnavailable,
    AllocationFailed,
    ImageBudgetExceeded,
    HistoryEvictionNotApproved,
    IrreversibleEditNotApproved,
    StaleHistoryPlan,
}
impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(error) => write!(f, "{error}"),
            Self::InvalidImage => f.write_str("Das Bild ist ungültig"),
            Self::HistoryBudgetExceeded => f.write_str(
                "Diese Änderung überschreitet das Speicherlimit für die Rückgängig-Historie",
            ),
            Self::HistoryNavigationBudgetExceeded => f.write_str(
                "Diese Aktion wurde nicht ausgeführt, weil das Speicherlimit für die Rückgängig-Historie nicht ausreicht. Bild und Verlauf bleiben unverändert.",
            ),
            Self::TransactionAlreadyOpen => f.write_str("Eine Bearbeitung läuft bereits"),
            Self::NoTransaction => f.write_str("Es läuft keine Bearbeitung"),
            Self::InvalidShapeIndex => {
                f.write_str("Das ausgewählte Objekt ist nicht mehr vorhanden")
            }
            Self::RandomUnavailable => f.write_str("Zufallswerte für die Verpixelung sind nicht verfügbar. Die Form wurde nicht erstellt."),
            Self::AllocationFailed => f.write_str("Es steht nicht genügend Speicher zur Verfügung"),
            Self::ImageBudgetExceeded => f.write_str(
                "Das Speicherlimit für Bilddaten reicht für diese Leinwandänderung nicht aus",
            ),
            Self::HistoryEvictionNotApproved => f.write_str(
                "Das Entfernen von Rückgängig- oder Wiederherstellen-Schritten wurde nicht bestätigt",
            ),
            Self::IrreversibleEditNotApproved => f.write_str(
                "Diese Bearbeitung kann nicht rückgängig gemacht werden. Dafür fehlt die ausdrückliche Bestätigung.",
            ),
            Self::StaleHistoryPlan => f.write_str(
                "Das Dokument wurde seit der Speicherprüfung verändert. Bitte erneut prüfen.",
            ),
        }
    }
}
impl std::error::Error for DocumentError {}
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
    image: ImageHandle,
    shapes: Vec<Shape>,
    revision: Arc<()>,
}

struct NavigationPlan {
    undo: VecDeque<State>,
    redo: Vec<State>,
}

impl NavigationPlan {
    fn reserve(undo_len: usize, redo_len: usize) -> Result<Self, DocumentError> {
        let mut plan = Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
        };
        reserve_history(|| plan.undo.try_reserve_exact(undo_len))?;
        reserve_history(|| plan.redo.try_reserve_exact(redo_len))?;
        Ok(plan)
    }

    fn container_bytes(&self) -> usize {
        container_bytes(self.undo.capacity(), size_of::<State>())
            .saturating_add(container_bytes(self.redo.capacity(), size_of::<State>()))
    }
}

/// Whether a prepared edit can retain its newest undo step under the budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryCommitMode {
    Undoable,
    WithoutUndo,
}

/// Explicit upper bound on history loss authorized by the caller. Approval of
/// ordinary eviction never authorizes an edit that cannot be undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitApproval {
    PreserveHistory,
    DiscardOldHistory,
    CommitWithoutUndo,
}

/// Prepared transaction commit. Planning never changes the document or grants
/// permission to discard undo steps. Consume this exact plan to use the
/// replacement container whose actual capacity was checked against the budget.
#[derive(Debug)]
pub struct HistorySnapshotPlan {
    generation: Arc<()>,
    snapshot: Option<State>,
    suspended_after: Option<State>,
    undo: VecDeque<State>,
    undo_entries_to_evict: usize,
    redo_entries_to_discard: usize,
    history_bytes: usize,
    history_budget: usize,
}

impl HistorySnapshotPlan {
    pub fn mode(&self) -> HistoryCommitMode {
        if self.snapshot.is_some() {
            HistoryCommitMode::Undoable
        } else {
            HistoryCommitMode::WithoutUndo
        }
    }
    pub fn undo_entries_to_evict(&self) -> usize {
        self.undo_entries_to_evict
    }
    /// Redo belongs to the abandoned branch, independently of budget eviction.
    pub fn redo_entries_to_discard(&self) -> usize {
        self.redo_entries_to_discard
    }
    pub fn history_bytes(&self) -> usize {
        self.history_bytes
    }
    pub fn history_budget(&self) -> usize {
        self.history_budget
    }
}

#[derive(Debug)]
pub struct Document {
    current: State,
    undo: VecDeque<State>,
    redo: Vec<State>,
    staged_before: Option<State>,
    saved_revision: Option<Arc<()>>,
    history_budget: usize,
    history_generation: Arc<()>,
}

impl Clone for Document {
    fn clone(&self) -> Self {
        Self {
            current: self.current.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            staged_before: self.staged_before.clone(),
            saved_revision: self.saved_revision.clone(),
            history_budget: self.history_budget,
            // Cloning may change allocation capacities. A plan belongs only
            // to the original document, even when logical contents are equal.
            history_generation: Arc::new(()),
        }
    }
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
        Ok(Self::from_validated_image_handle(
            ImageHandle::from_image(image),
            history_budget_bytes,
        ))
    }

    /// Reuse an existing allocation and its import reservation. Validation
    /// failure returns the original handle without copying or losing ownership.
    pub fn from_image_handle(image: ImageHandle) -> Result<Self, (DocumentError, ImageHandle)> {
        Self::with_image_handle_history_budget(image, DEFAULT_HISTORY_BUDGET)
    }

    pub fn with_image_handle_history_budget(
        image: ImageHandle,
        history_budget_bytes: usize,
    ) -> Result<Self, (DocumentError, ImageHandle)> {
        if let Err(error) = validate_image(&image) {
            return Err((error, image));
        }
        Ok(Self::from_validated_image_handle(
            image,
            history_budget_bytes,
        ))
    }

    fn from_validated_image_handle(image: ImageHandle, history_budget_bytes: usize) -> Self {
        Self {
            current: State {
                image,
                shapes: Vec::new(),
                revision: Arc::new(()),
            },
            undo: VecDeque::new(),
            redo: Vec::new(),
            staged_before: None,
            saved_revision: None,
            history_budget: history_budget_bytes,
            history_generation: Arc::new(()),
        }
    }

    pub fn image(&self) -> &Image {
        &self.current.image
    }
    pub fn image_arc(&self) -> ImageHandle {
        self.current.image.clone()
    }
    pub fn undo_image_arc(&self) -> Option<ImageHandle> {
        self.undo.back().map(|s| s.image.clone())
    }
    pub fn redo_image_arc(&self) -> Option<ImageHandle> {
        self.redo.last().map(|s| s.image.clone())
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
        self.invalidate_history_plans();
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
        seen.insert(self.current.image.identity());
        self.undo
            .iter()
            .chain(self.redo.iter())
            .fold(0usize, |total, state| {
                let image_bytes = if seen.insert(state.image.identity()) {
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
        self.invalidate_history_plans();
        self.history_budget = bytes;
        self.trim_history();
    }

    /// Start a rollback-safe group of edits. Nested transactions are rejected.
    pub fn begin_edit(&mut self) -> Result<(), DocumentError> {
        if self.staged_before.is_some() {
            return Err(DocumentError::TransactionAlreadyOpen);
        }
        self.staged_before = Some(self.current.clone());
        self.invalidate_history_plans();
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
            self.invalidate_history_plans();
            return Ok(false);
        }
        self.record_snapshot(before.clone(), self.current.image.clone())?;
        self.current.revision = Arc::new(());
        self.staged_before = None;
        self.trim_history();
        Ok(true)
    }

    /// Prepare a staged transaction without changing pixels, shapes, dirty
    /// state, either history, or the transaction. Returns `None` for a no-op.
    /// Both the commit and its immediate undo/redo must fit; otherwise planning
    /// fails without clearing redo or evicting anything. Image allocations are
    /// deduplicated by Arc identity, excluding each destination's current image,
    /// just as in
    /// `history_bytes`. Existing snapshot capacities and the actual cloned
    /// snapshot/replacement-container capacities are charged conservatively.
    ///
    /// This API is opt-in: legacy `commit_edit` and direct edits retain their
    /// existing automatic-eviction behavior until callers integrate this plan.
    pub fn plan_commit_edit(&self) -> Result<Option<HistorySnapshotPlan>, DocumentError> {
        self.plan_commit_edit_internal(false)
    }

    /// Additionally offer a separately approved irreversible commit when even
    /// the newest undo step and its immediate inverse cannot fit after all old
    /// entries are removed. Allocation failures are propagated unchanged; only
    /// an exhausted, successfully allocated budget check permits this fallback.
    /// A `WithoutUndo` plan requires confirmation even with no old history.
    pub fn plan_commit_edit_with_fallback(
        &self,
    ) -> Result<Option<HistorySnapshotPlan>, DocumentError> {
        self.plan_commit_edit_internal(true)
    }

    fn plan_commit_edit_internal(
        &self,
        allow_without_undo_plan: bool,
    ) -> Result<Option<HistorySnapshotPlan>, DocumentError> {
        let before = self
            .staged_before
            .as_ref()
            .ok_or(DocumentError::NoTransaction)?;
        if states_equal(before, &self.current) {
            return Ok(None);
        }
        let snapshot = before.clone();
        let mut committed = HistoryStorage::new(
            self.undo.iter().chain(std::iter::once(&snapshot)),
            &self.current.image,
        )?;
        // Undo makes `snapshot` current and stores the actual new state in redo.
        // Its shape/image capacities can be much larger than the old snapshot.
        let mut inverse = HistoryStorage::new(
            self.undo.iter().chain(std::iter::once(&self.current)),
            &snapshot.image,
        )?;
        let mut evictions = 0;
        loop {
            let mut undo = VecDeque::new();
            reserve_history(|| undo.try_reserve_exact(self.undo.len() - evictions + 1))?;
            let history_bytes = committed
                .bytes
                .checked_add(container_bytes(undo.capacity(), size_of::<State>()))
                .ok_or(DocumentError::HistoryBudgetExceeded)?;
            let retained = self.undo.len() - evictions;
            // Check actual destination capacities, then release these temporary
            // containers. They are not retained or silently charged elsewhere.
            // Navigation still rechecks its own allocations and can safely fail
            // on later OOM or different allocator capacities without any trim.
            let inverse_containers = NavigationPlan::reserve(retained, 1)?;
            let inverse_bytes = inverse
                .bytes
                .checked_add(inverse_containers.container_bytes())
                .ok_or(DocumentError::HistoryBudgetExceeded)?;
            let replay_containers = NavigationPlan::reserve(retained + 1, 0)?;
            let replay_bytes = committed
                .bytes
                .checked_add(replay_containers.container_bytes())
                .ok_or(DocumentError::HistoryBudgetExceeded)?;
            if history_bytes.max(inverse_bytes).max(replay_bytes) <= self.history_budget {
                return Ok(Some(HistorySnapshotPlan {
                    generation: Arc::clone(&self.history_generation),
                    snapshot: Some(snapshot),
                    suspended_after: None,
                    undo,
                    undo_entries_to_evict: evictions,
                    redo_entries_to_discard: self.redo.len(),
                    history_bytes,
                    history_budget: self.history_budget,
                }));
            }
            let Some(oldest) = self.undo.get(evictions) else {
                if allow_without_undo_plan {
                    // This branch is reached only after every old undo state
                    // was considered for eviction and the actual minimum
                    // commit/undo/redo storage still exceeded the budget.
                    // Keep neither the cloned snapshot nor any container.
                    return Ok(Some(HistorySnapshotPlan {
                        generation: Arc::clone(&self.history_generation),
                        snapshot: None,
                        suspended_after: None,
                        undo: VecDeque::new(),
                        undo_entries_to_evict: self.undo.len(),
                        redo_entries_to_discard: self.redo.len(),
                        history_bytes: 0,
                        history_budget: self.history_budget,
                    }));
                }
                return Err(DocumentError::HistoryBudgetExceeded);
            };
            committed.remove(oldest);
            inverse.remove(oldest);
            evictions += 1;
        }
    }

    /// Restore the pre-transaction state while a caller asks for permission.
    /// The staged result moves into the returned plan, so paints and other
    /// readers see the original document during a modal dialog. Dropping the
    /// returned plan cancels the edit. Any intervening mutation makes it stale.
    pub fn suspend_edit_with_plan(
        &mut self,
        mut plan: HistorySnapshotPlan,
    ) -> Result<HistorySnapshotPlan, DocumentError> {
        if !Arc::ptr_eq(&plan.generation, &self.history_generation) {
            return Err(DocumentError::StaleHistoryPlan);
        }
        let before = self
            .staged_before
            .take()
            .ok_or(DocumentError::NoTransaction)?;
        plan.suspended_after = Some(std::mem::replace(&mut self.current, before));
        self.invalidate_history_plans();
        plan.generation = Arc::clone(&self.history_generation);
        Ok(plan)
    }

    /// Commit a previously checked transaction. A stale plan or unapproved
    /// history discard leaves the transaction and all document state untouched.
    /// `false` never authorizes dropping undo or redo steps, including redo
    /// discarded by a new branch. The plan reports both counts separately.
    /// The preallocated container is used without reallocation.
    pub fn commit_edit_with_plan(
        &mut self,
        plan: HistorySnapshotPlan,
        allow_history_discard: bool,
    ) -> Result<bool, DocumentError> {
        self.commit_edit_with_approval(
            plan,
            if allow_history_discard {
                CommitApproval::DiscardOldHistory
            } else {
                CommitApproval::PreserveHistory
            },
        )
    }

    /// Apply a checked edit with explicit scope of consent. An irreversible
    /// plan requires `CommitWithoutUndo`, independently of its discard counts.
    /// On success it clears both histories including their allocated capacity;
    /// future edits may start a new history from this irreversible baseline.
    /// `CommitWithoutUndo` does not force history loss for an undoable plan.
    pub fn commit_edit_with_approval(
        &mut self,
        mut plan: HistorySnapshotPlan,
        approval: CommitApproval,
    ) -> Result<bool, DocumentError> {
        if !Arc::ptr_eq(&plan.generation, &self.history_generation) {
            return Err(DocumentError::StaleHistoryPlan);
        }
        if plan.mode() == HistoryCommitMode::WithoutUndo
            && approval != CommitApproval::CommitWithoutUndo
        {
            return Err(DocumentError::IrreversibleEditNotApproved);
        }
        if (plan.undo_entries_to_evict > 0 || plan.redo_entries_to_discard > 0)
            && approval == CommitApproval::PreserveHistory
        {
            return Err(DocumentError::HistoryEvictionNotApproved);
        }
        let revision = Arc::new(());
        self.invalidate_history_plans();
        if let Some(snapshot) = plan.snapshot {
            for _ in 0..plan.undo_entries_to_evict {
                self.undo.pop_front();
            }
            plan.undo.extend(self.undo.drain(..));
            plan.undo.push_back(snapshot);
            self.undo = plan.undo;
        } else {
            self.undo = VecDeque::new();
        }
        self.redo = Vec::new();
        if let Some(after) = plan.suspended_after {
            self.current = after;
        }
        self.current.revision = revision;
        self.staged_before = None;
        debug_assert_eq!(self.history_bytes(), plan.history_bytes);
        Ok(true)
    }

    /// Discard all staged changes and restore image and shapes atomically.
    pub fn abort_edit(&mut self) -> Result<(), DocumentError> {
        let Some(before) = self.staged_before.take() else {
            return Err(DocumentError::NoTransaction);
        };
        self.current = before;
        self.invalidate_history_plans();
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
        next.image = ImageHandle::from_image(image);
        self.install(next)
    }

    /// Crop pixels and shift every annotation by the crop origin in one step.
    pub fn crop(&mut self, rect: CropRect) -> Result<(), DocumentError> {
        let image = ImageHandle::from_image(image_edit::crop(&self.current.image, rect)?);
        let mut next = self.current.clone();
        next.image = image;
        for shape in &mut next.shapes {
            shape.offset(-(rect.x as f32), -(rect.y as f32));
        }
        self.install(next)
    }

    /// Crop into a shared-pool tracked allocation. As with `crop`, a staged
    /// transaction can be aborted if a later bitmap/history step is rejected.
    /// Unstaged use retains the existing direct-edit history policy.
    pub fn crop_with_budget(
        &mut self,
        rect: CropRect,
        budget: &crate::resource_budget::ResourceBudget,
    ) -> Result<(), DocumentError> {
        let image = image_edit::crop_with_budget(&self.current.image, rect, budget)?;
        let mut next = self.current.clone();
        next.image = image;
        for shape in &mut next.shapes {
            shape.offset(-(rect.x as f32), -(rect.y as f32));
        }
        self.install(next)
    }

    /// Add frame/shadow and move annotations to the new image-content origin.
    pub fn add_frame(&mut self, options: FrameOptions) -> Result<(), DocumentError> {
        let image = ImageHandle::from_image(image_edit::add_frame(&self.current.image, options)?);
        self.install_frame(image, options)
    }

    /// Add tracked frame/shadow output; aborting a staged edit releases only
    /// that output while preserving the source and its history ownership.
    pub fn add_frame_with_budget(
        &mut self,
        options: FrameOptions,
        budget: &crate::resource_budget::ResourceBudget,
    ) -> Result<(), DocumentError> {
        let image = image_edit::add_frame_with_budget(&self.current.image, options, budget)?;
        self.install_frame(image, options)
    }

    /// Add a tracked torn edge (#98). Annotations move with the shadow margin
    /// exactly as for a frame of width zero with the same shadow.
    pub fn add_torn_edge_with_budget(
        &mut self,
        options: image_edit::TornEdgeOptions,
        budget: &crate::resource_budget::ResourceBudget,
    ) -> Result<(), DocumentError> {
        let image = image_edit::add_torn_edge_with_budget(&self.current.image, options, budget)?;
        self.install_frame(image, image_edit::torn_edge_frame_equivalent(options))
    }

    fn install_frame(
        &mut self,
        image: ImageHandle,
        options: FrameOptions,
    ) -> Result<(), DocumentError> {
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
        self.resize_canvas_inner(width, height, horizontal, vertical, fill, None)
    }

    /// Resize the canvas while registering the replacement pixel allocation
    /// in the caller's shared image budget. The lease follows the image through
    /// current state and undo/redo history.
    pub fn resize_canvas_with_budget(
        &mut self,
        width: usize,
        height: usize,
        horizontal: CanvasAlignment,
        vertical: CanvasAlignment,
        fill: Color,
        budget: &ResourceBudget,
    ) -> Result<(), DocumentError> {
        self.resize_canvas_inner(width, height, horizontal, vertical, fill, Some(budget))
    }

    fn resize_canvas_inner(
        &mut self,
        width: usize,
        height: usize,
        horizontal: CanvasAlignment,
        vertical: CanvasAlignment,
        fill: Color,
        budget: Option<&ResourceBudget>,
    ) -> Result<(), DocumentError> {
        let bytes = checked_edit_size(width, height)?;
        let source = &self.current.image;
        if source.width == width && source.height == height {
            return Ok(());
        }
        // Admission precedes the large allocation. The temporary lease is
        // dropped on every failure path and transferred into the image below.
        let mut lease = budget
            .map(|budget| {
                budget
                    .try_acquire(bytes, AdmissionClass::Reserved)
                    .map_err(|_| DocumentError::ImageBudgetExceeded)
            })
            .transpose()?;
        let (source_x, dest_x, copy_width) = canvas_axis(source.width, width, horizontal);
        let (source_y, dest_y, copy_height) = canvas_axis(source.height, height, vertical);
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bytes)
            .map_err(|_| DocumentError::AllocationFailed)?;
        if let Some(lease) = &mut lease {
            lease
                .try_resize(pixels.capacity())
                .map_err(|_| DocumentError::ImageBudgetExceeded)?;
        }
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
        let image = Image {
            width,
            height,
            pixels,
        };
        next.image = match lease {
            Some(lease) => {
                ImageHandle::from_imported(Arc::new(TrackedImage::from_reserved(image, lease)))
            }
            None => ImageHandle::from_image(image),
        };
        let dx = dest_x as f32 - source_x as f32;
        let dy = dest_y as f32 - source_y as f32;
        for shape in &mut next.shapes {
            shape.offset(dx, dy);
        }
        self.install(next)
    }

    /// Check whether undo can preserve all existing history within the budget.
    pub fn check_undo(&self) -> Result<bool, DocumentError> {
        Ok(self.plan_navigation(false)?.is_some())
    }

    /// Check whether redo can preserve all existing history within the budget.
    pub fn check_redo(&self) -> Result<bool, DocumentError> {
        Ok(self.plan_navigation(true)?.is_some())
    }

    pub fn try_undo(&mut self) -> Result<bool, DocumentError> {
        let Some(mut plan) = self.plan_navigation(false)? else {
            return Ok(false);
        };
        let previous = self
            .undo
            .pop_back()
            .expect("navigation plan has an undo target");
        while let Some(state) = self.undo.pop_front() {
            plan.undo.push_back(state);
        }
        for state in self.redo.drain(..) {
            plan.redo.push(state);
        }
        plan.redo
            .push(std::mem::replace(&mut self.current, previous));
        self.undo = plan.undo;
        self.redo = plan.redo;
        self.invalidate_history_plans();
        Ok(true)
    }

    /// Compatibility wrapper; use `try_undo` to distinguish a budget or
    /// allocation failure from an empty history.
    pub fn undo(&mut self) -> bool {
        self.try_undo().unwrap_or(false)
    }

    pub fn try_redo(&mut self) -> Result<bool, DocumentError> {
        let Some(mut plan) = self.plan_navigation(true)? else {
            return Ok(false);
        };
        let next = self.redo.pop().expect("navigation plan has a redo target");
        while let Some(state) = self.undo.pop_front() {
            plan.undo.push_back(state);
        }
        plan.undo
            .push_back(std::mem::replace(&mut self.current, next));
        for state in self.redo.drain(..) {
            plan.redo.push(state);
        }
        self.undo = plan.undo;
        self.redo = plan.redo;
        self.invalidate_history_plans();
        Ok(true)
    }

    /// Compatibility wrapper; use `try_redo` to distinguish a budget or
    /// allocation failure from an empty history.
    pub fn redo(&mut self) -> bool {
        self.try_redo().unwrap_or(false)
    }

    fn install(&mut self, mut next: State) -> Result<(), DocumentError> {
        if states_equal(&next, &self.current) {
            return Ok(());
        }
        if self.staged_before.is_none() {
            self.record_snapshot(self.current.clone(), next.image.clone())?;
            next.revision = Arc::new(());
        }
        self.current = next;
        self.invalidate_history_plans();
        if self.staged_before.is_none() {
            self.trim_history();
        }
        Ok(())
    }

    fn record_snapshot(
        &mut self,
        snapshot: State,
        next_image: ImageHandle,
    ) -> Result<(), DocumentError> {
        if self.snapshot_bytes_after_redo_clear(&snapshot, &next_image) > self.history_budget {
            return Err(DocumentError::HistoryBudgetExceeded);
        }
        self.redo.clear();
        self.redo.shrink_to_fit();
        self.undo.push_back(snapshot);
        self.invalidate_history_plans();
        Ok(())
    }

    fn invalidate_history_plans(&mut self) {
        self.history_generation = Arc::new(());
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

    // Reserve both replacement containers first, and account their actual
    // capacities and all retained states against the destination image. No
    // user-visible state changes or evictions occur on failure.
    fn plan_navigation(&self, redo: bool) -> Result<Option<NavigationPlan>, DocumentError> {
        if self.staged_before.is_some() {
            return Ok(None);
        }
        let target = if redo {
            self.redo.last()
        } else {
            self.undo.back()
        };
        let Some(target) = target else {
            return Ok(None);
        };
        let undo_len = if redo {
            self.undo.len() + 1
        } else {
            self.undo.len() - 1
        };
        let redo_len = if redo {
            self.redo.len() - 1
        } else {
            self.redo.len() + 1
        };
        let plan = NavigationPlan::reserve(undo_len, redo_len)?;
        let retained_undo = if redo {
            self.undo.len()
        } else {
            self.undo.len() - 1
        };
        let retained_redo = if redo {
            self.redo.len() - 1
        } else {
            self.redo.len()
        };
        let states = self
            .undo
            .iter()
            .take(retained_undo)
            .chain(self.redo.iter().take(retained_redo))
            .chain(std::iter::once(&self.current));
        let count = retained_undo
            .saturating_add(retained_redo)
            .saturating_add(1);
        let mut seen = HashSet::new();
        seen.try_reserve(count.saturating_add(1))
            .map_err(|_| DocumentError::AllocationFailed)?;
        seen.insert(target.image.identity());
        let mut bytes = container_bytes(plan.undo.capacity(), size_of::<State>())
            .saturating_add(container_bytes(plan.redo.capacity(), size_of::<State>()));
        for state in states {
            let image_bytes = if seen.insert(state.image.identity()) {
                image_allocation_bytes(&state.image)
            } else {
                0
            };
            bytes = bytes
                .saturating_add(size_of::<State>())
                .saturating_add(revision_allocation_bytes())
                .saturating_add(image_bytes)
                .saturating_add(shapes_allocation_bytes(&state.shapes));
        }
        if bytes > self.history_budget {
            return Err(DocumentError::HistoryNavigationBudgetExceeded);
        }
        Ok(Some(plan))
    }

    fn snapshot_bytes_after_redo_clear(&self, snapshot: &State, next_image: &ImageHandle) -> usize {
        // Older snapshots may all need eviction, so only sharing with the
        // resulting current image can reduce this snapshot's minimum cost.
        let image_bytes = if !next_image.ptr_eq(&snapshot.image) {
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

#[cfg(test)]
thread_local! {
    static HISTORY_RESERVATION_FAILURE_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static HISTORY_RESERVATION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Keep allocation failures on their own Result path. The test hook exercises
// propagation at every reservation, including the last minimum-budget check.
fn reserve_history<T>(
    reserve: impl FnOnce() -> Result<T, std::collections::TryReserveError>,
) -> Result<T, DocumentError> {
    #[cfg(test)]
    {
        HISTORY_RESERVATION_COUNT.with(|count| count.set(count.get() + 1));
        let fail = HISTORY_RESERVATION_FAILURE_AFTER.with(|after| match after.get() {
            Some(0) => true,
            Some(remaining) => {
                after.set(Some(remaining - 1));
                false
            }
            None => false,
        });
        if fail {
            return Err(DocumentError::AllocationFailed);
        }
    }
    reserve().map_err(|_| DocumentError::AllocationFailed)
}

fn snapshot_storage_bytes(state: &State) -> usize {
    size_of::<State>()
        .saturating_add(revision_allocation_bytes())
        .saturating_add(shapes_allocation_bytes(&state.shapes))
}

// Maintain both candidate destination totals in one pass. Removing the oldest
// state updates sharing counts in constant expected time; planning does not
// repeatedly rescan all retained states for every possible eviction count.
struct HistoryStorage {
    current_image: usize,
    images: HashMap<usize, (usize, usize)>,
    bytes: usize,
}

impl HistoryStorage {
    fn new<'a>(
        states: impl Iterator<Item = &'a State>,
        current_image: &ImageHandle,
    ) -> Result<Self, DocumentError> {
        let mut storage = Self {
            current_image: current_image.identity(),
            images: HashMap::new(),
            bytes: 0,
        };
        reserve_history(|| {
            storage
                .images
                .try_reserve(states.size_hint().1.unwrap_or(states.size_hint().0))
        })?;
        for state in states {
            storage.bytes = storage
                .bytes
                .checked_add(snapshot_storage_bytes(state))
                .ok_or(DocumentError::HistoryBudgetExceeded)?;
            let image = state.image.identity();
            if image != storage.current_image {
                if let Some(entry) = storage.images.get_mut(&image) {
                    entry.0 += 1;
                } else {
                    let image_bytes = image_allocation_bytes(&state.image);
                    storage.bytes = storage
                        .bytes
                        .checked_add(image_bytes)
                        .ok_or(DocumentError::HistoryBudgetExceeded)?;
                    storage.images.insert(image, (1, image_bytes));
                }
            }
        }
        Ok(storage)
    }

    fn remove(&mut self, state: &State) {
        self.bytes -= snapshot_storage_bytes(state);
        let image = state.image.identity();
        if image != self.current_image {
            let entry = self
                .images
                .get_mut(&image)
                .expect("accounted history image");
            entry.0 -= 1;
            if entry.0 == 0 {
                self.bytes -= entry.1;
            }
        }
    }
}

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
                crate::shapes::ShapeKind::Callout { data } => {
                    // Every snapshot owns a deep-cloned payload, in addition to
                    // the text and per-line measurements owned by that payload.
                    size_of::<crate::shapes::CalloutData>()
                        .saturating_add(ALLOCATION_OVERHEAD)
                        .saturating_add(container_bytes(data.text.capacity(), size_of::<u8>()))
                        .saturating_add(container_bytes(
                            data.layout.line_widths.capacity(),
                            size_of::<f32>(),
                        ))
                }
                crate::shapes::ShapeKind::Cursor { data } => {
                    use crate::cursor_snapshot::CursorPixels;
                    let buffers = match data.pixels.as_ref() {
                        CursorPixels::PremultipliedRgba(bytes) => {
                            container_bytes(bytes.capacity(), 1)
                        }
                        CursorPixels::Raster { and_mask, xor_rgb } => {
                            container_bytes(and_mask.capacity(), 1)
                                .saturating_add(container_bytes(xor_rgb.capacity(), 1))
                        }
                    };
                    buffers
                        .saturating_add(size_of::<CursorPixels>())
                        .saturating_add(ALLOCATION_OVERHEAD)
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
    (left.image.ptr_eq(&right.image) || left.image.as_ref() == right.image.as_ref())
        && left.shapes == right.shapes
}

#[cfg(test)]
mod tests;
