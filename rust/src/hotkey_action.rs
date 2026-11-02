//! Platform-independent mapping of registered hotkey IDs to capture actions.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Screen,
    AllScreens,
    ActiveWindow,
    Scroll,
    Region,
}

pub fn action_for_id(id: usize) -> Option<Action> {
    match id {
        1 | 6 | 10 => Some(Action::Screen),
        7 => Some(Action::AllScreens),
        8 | 11 => Some(Action::ActiveWindow),
        2 | 9 | 12 => Some(Action::Scroll),
        3..=5 => Some(Action::Region),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_hotkeys_route_to_expected_capture_modes() {
        let expected = [
            Action::Screen,
            Action::Scroll,
            Action::Region,
            Action::Region,
            Action::Region,
            Action::Screen,
            Action::AllScreens,
            Action::ActiveWindow,
            Action::Scroll,
            Action::Screen,
            Action::ActiveWindow,
            Action::Scroll,
        ];
        for (index, action) in expected.into_iter().enumerate() {
            assert_eq!(action_for_id(index + 1), Some(action));
        }
        assert_eq!(action_for_id(0), None);
        assert_eq!(action_for_id(13), None);
    }
}
