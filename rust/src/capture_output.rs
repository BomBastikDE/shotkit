//! Output planning for a newly captured image.
//!
//! The state machine never takes ownership of the image. The caller must keep
//! it alive until the plan completes; if opening the editor itself fails, the
//! returned [`Advance::RetainImage`] explicitly requires preserving it for
//! recovery instead of dropping it.

use crate::settings::Settings;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureAction {
    AutoSave,
    CopyToClipboard,
    OpenEditor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionFailure {
    pub action: CaptureAction,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// Call [`CaptureOutputState::next_action`] to get the following step.
    Continue,
    /// All requested steps completed; the image can be released according to
    /// the application's normal ownership rules.
    Complete,
    /// Opening the editor failed. The caller still owns the source image and
    /// must keep it available for recovery.
    RetainImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnexpectedAction {
    pub expected: Option<CaptureAction>,
    pub received: CaptureAction,
}

impl fmt::Display for UnexpectedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "capture output action mismatch: expected {:?}, received {:?}",
            self.expected, self.received
        )
    }
}

impl std::error::Error for UnexpectedAction {}

#[derive(Clone, Debug)]
pub struct CaptureOutputState {
    actions: Vec<CaptureAction>,
    next: usize,
    failures: Vec<ActionFailure>,
    terminal: Option<Advance>,
}

impl CaptureOutputState {
    /// Plan enabled outputs in deterministic order: auto-save, clipboard,
    /// editor. If no output is enabled, opening the editor is the safe default.
    pub fn new(settings: &Settings) -> Self {
        let mut actions = Vec::with_capacity(3);
        if settings.auto_save {
            actions.push(CaptureAction::AutoSave);
        }
        if settings.copy_to_clipboard {
            actions.push(CaptureAction::CopyToClipboard);
        }
        if settings.open_editor || actions.is_empty() {
            actions.push(CaptureAction::OpenEditor);
        }
        Self {
            actions,
            next: 0,
            failures: Vec::new(),
            terminal: None,
        }
    }

    pub fn next_action(&self) -> Option<CaptureAction> {
        if self.terminal.is_some() {
            None
        } else {
            self.actions.get(self.next).copied()
        }
    }

    pub fn failures(&self) -> &[ActionFailure] {
        &self.failures
    }

    pub fn is_complete(&self) -> bool {
        self.terminal.is_some()
    }

    /// Report one action's result. Failed auto-save/clipboard steps do not stop
    /// the other requested outputs, but force the editor as the final fallback.
    /// If the editor step fails, `RetainImage` is returned and the source image
    /// remains the caller's responsibility.
    pub fn finish_action(
        &mut self,
        action: CaptureAction,
        result: Result<(), String>,
    ) -> Result<Advance, UnexpectedAction> {
        let expected = self.next_action();
        if expected != Some(action) {
            return Err(UnexpectedAction {
                expected,
                received: action,
            });
        }

        self.next += 1;
        if let Err(message) = result {
            self.failures.push(ActionFailure { action, message });
            if action == CaptureAction::OpenEditor {
                self.terminal = Some(Advance::RetainImage);
                return Ok(Advance::RetainImage);
            }
            if !self.actions[self.next..].contains(&CaptureAction::OpenEditor) {
                self.actions.push(CaptureAction::OpenEditor);
            }
        }

        if self.next == self.actions.len() {
            self.terminal = Some(Advance::Complete);
            Ok(Advance::Complete)
        } else {
            Ok(Advance::Continue)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn settings(open_editor: bool, copy_to_clipboard: bool, auto_save: bool) -> Settings {
        Settings {
            open_editor,
            copy_to_clipboard,
            auto_save,
            ..Settings::default()
        }
    }

    #[test]
    fn all_eight_setting_combinations_have_safe_plans() {
        let cases = [
            (false, false, false, vec![CaptureAction::OpenEditor]),
            (false, false, true, vec![CaptureAction::AutoSave]),
            (false, true, false, vec![CaptureAction::CopyToClipboard]),
            (
                false,
                true,
                true,
                vec![CaptureAction::AutoSave, CaptureAction::CopyToClipboard],
            ),
            (true, false, false, vec![CaptureAction::OpenEditor]),
            (
                true,
                false,
                true,
                vec![CaptureAction::AutoSave, CaptureAction::OpenEditor],
            ),
            (
                true,
                true,
                false,
                vec![CaptureAction::CopyToClipboard, CaptureAction::OpenEditor],
            ),
            (
                true,
                true,
                true,
                vec![
                    CaptureAction::AutoSave,
                    CaptureAction::CopyToClipboard,
                    CaptureAction::OpenEditor,
                ],
            ),
        ];

        for (open, copy, save, expected) in cases {
            let plan = CaptureOutputState::new(&settings(open, copy, save));
            let mut actual = Vec::new();
            let mut next = plan.next_action();
            let mut plan = plan;
            while let Some(action) = next {
                actual.push(action);
                let is_last = actual.len() == expected.len();
                assert_eq!(
                    plan.finish_action(action, Ok::<(), String>(())).unwrap(),
                    if is_last {
                        Advance::Complete
                    } else {
                        Advance::Continue
                    }
                );
                next = plan.next_action();
            }
            assert_eq!(actual, expected);
            assert!(plan.is_complete());
        }
    }

    #[test]
    fn failed_external_outputs_force_editor_after_remaining_actions() {
        let mut plan = CaptureOutputState::new(&settings(false, true, true));
        assert_eq!(
            plan.finish_action(CaptureAction::AutoSave, Err("disk full".into())),
            Ok(Advance::Continue)
        );
        assert_eq!(plan.next_action(), Some(CaptureAction::CopyToClipboard));
        assert_eq!(
            plan.finish_action(CaptureAction::CopyToClipboard, Err("clipboard busy".into())),
            Ok(Advance::Continue)
        );
        assert_eq!(plan.next_action(), Some(CaptureAction::OpenEditor));
        assert_eq!(
            plan.finish_action(CaptureAction::OpenEditor, Ok::<(), String>(())),
            Ok(Advance::Complete)
        );
        assert_eq!(plan.failures().len(), 2);
        assert_eq!(plan.failures()[0].action, CaptureAction::AutoSave);
        assert_eq!(plan.failures()[1].action, CaptureAction::CopyToClipboard);
    }

    #[test]
    fn failed_editor_open_requires_retaining_the_source_image() {
        let mut plan = CaptureOutputState::new(&settings(false, false, false));
        assert_eq!(
            plan.finish_action(
                CaptureAction::OpenEditor,
                Err("window creation failed".into()),
            ),
            Ok(Advance::RetainImage)
        );
        assert!(plan.is_complete());
        assert_eq!(plan.next_action(), None);
        assert_eq!(plan.failures()[0].message, "window creation failed");
    }

    #[test]
    fn successful_outputs_without_editor_complete_without_fallback() {
        let mut plan = CaptureOutputState::new(&settings(false, true, true));
        plan.finish_action(CaptureAction::AutoSave, Ok::<(), String>(()))
            .unwrap();
        assert_eq!(
            plan.finish_action(CaptureAction::CopyToClipboard, Ok::<(), String>(())),
            Ok(Advance::Complete)
        );
        assert!(plan.is_complete());
    }

    #[test]
    fn wrong_or_duplicate_action_is_reported_without_advancing() {
        let mut plan = CaptureOutputState::new(&settings(false, true, false));
        assert_eq!(
            plan.finish_action(CaptureAction::OpenEditor, Ok::<(), String>(())),
            Err(UnexpectedAction {
                expected: Some(CaptureAction::CopyToClipboard),
                received: CaptureAction::OpenEditor,
            })
        );
        assert_eq!(plan.next_action(), Some(CaptureAction::CopyToClipboard));
    }
}
