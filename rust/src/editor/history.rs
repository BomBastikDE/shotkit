use super::*;

impl EditorController {
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

    pub(super) fn commit_replacement(
        &mut self,
        index: usize,
        original: Shape,
        replacement: Shape,
    ) -> bool {
        if original != replacement {
            self.edit_shape(index, |shape| *shape = replacement)
        } else {
            false
        }
    }

    pub(super) fn edit_shape<F>(&mut self, index: usize, edit: F) -> bool
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
