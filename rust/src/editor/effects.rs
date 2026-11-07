use super::*;

impl EditorController {
    /// Set a default and, when selected, change the spotlight in one undo step.
    pub fn set_spotlight_opacity(&mut self, opacity: f32) -> bool {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return false;
        }
        self.cancel_drag();
        self.spotlight_opacity = opacity;
        let Some(index) = self.selected else {
            return false;
        };
        let Some(original) = self.document.shapes().get(index).cloned() else {
            return false;
        };
        let mut replacement = original.clone();
        let ShapeKind::Spotlight { data } = &mut replacement.kind else {
            return false;
        };
        data.dim_opacity = opacity;
        self.commit_replacement(index, original, replacement)
    }

    /// Set a default and, when selected, change the image magnifier in one undo step.
    pub fn set_magnifier_zoom(&mut self, zoom: f32) -> bool {
        if !zoom.is_finite() || zoom < 1.0 {
            return false;
        }
        self.cancel_drag();
        self.magnifier_zoom = zoom;
        let Some(index) = self.selected else {
            return false;
        };
        let Some(original) = self.document.shapes().get(index).cloned() else {
            return false;
        };
        let mut replacement = original.clone();
        let ShapeKind::Magnifier { data } = &mut replacement.kind else {
            return false;
        };
        if data.set_zoom(zoom).is_err() {
            return false;
        }
        self.commit_replacement(index, original, replacement)
    }

    /// Choose round/oval or rectangular framing for new and selected magnifiers.
    pub fn set_magnifier_frame(&mut self, frame: crate::shapes::MagnifierFrame) -> bool {
        self.cancel_drag();
        self.magnifier_frame = frame;
        let Some(index) = self.selected else {
            return false;
        };
        let Some(original) = self.document.shapes().get(index).cloned() else {
            return false;
        };
        let mut replacement = original.clone();
        let ShapeKind::Magnifier { data } = &mut replacement.kind else {
            return false;
        };
        data.frame = frame;
        self.commit_replacement(index, original, replacement)
    }
}
