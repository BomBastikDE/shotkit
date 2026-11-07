//! Platform-independent hotkey registrations and their capture actions.

/// Win32 modifier bits other than `MOD_NOREPEAT` (added by the caller).
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Screen,
    AllScreens,
    ActiveWindow,
    Scroll,
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    pub id: i32,
    pub modifiers: u32,
    pub virtual_key: u32,
    pub action: Action,
    pub label: &'static str,
}

/// Rust's registered shortcuts. IDs 1–3 are Rust-only F-key fallbacks; IDs
/// 4–9 and 10–12 implement the C# Print Screen shortcuts and alternatives.
pub const REGISTRATIONS: [Registration; 12] = [
    Registration {
        id: 1,
        modifiers: 0,
        virtual_key: 0x78,
        action: Action::Screen,
        label: "F9",
    },
    Registration {
        id: 2,
        modifiers: 0,
        virtual_key: 0x79,
        action: Action::Scroll,
        label: "F10",
    },
    Registration {
        id: 3,
        modifiers: 0,
        virtual_key: 0x7a,
        action: Action::Region,
        label: "F11",
    },
    Registration {
        id: 4,
        modifiers: 0,
        virtual_key: 0x2c,
        action: Action::Region,
        label: "Druck",
    },
    Registration {
        id: 5,
        modifiers: MOD_CONTROL | MOD_SHIFT,
        virtual_key: 0x31,
        action: Action::Region,
        label: "Strg+Umschalt+1",
    },
    Registration {
        id: 6,
        modifiers: MOD_CONTROL,
        virtual_key: 0x2c,
        action: Action::Screen,
        label: "Strg+Druck",
    },
    Registration {
        id: 7,
        modifiers: MOD_CONTROL | MOD_SHIFT,
        virtual_key: 0x2c,
        action: Action::AllScreens,
        label: "Strg+Umschalt+Druck",
    },
    Registration {
        id: 8,
        modifiers: MOD_SHIFT,
        virtual_key: 0x2c,
        action: Action::ActiveWindow,
        label: "Umschalt+Druck",
    },
    Registration {
        id: 9,
        modifiers: MOD_CONTROL | MOD_ALT,
        virtual_key: 0x2c,
        action: Action::Scroll,
        label: "Strg+Alt+Druck",
    },
    Registration {
        id: 10,
        modifiers: MOD_CONTROL | MOD_SHIFT,
        virtual_key: 0x32,
        action: Action::Screen,
        label: "Strg+Umschalt+2",
    },
    Registration {
        id: 11,
        modifiers: MOD_CONTROL | MOD_SHIFT,
        virtual_key: 0x33,
        action: Action::ActiveWindow,
        label: "Strg+Umschalt+3",
    },
    Registration {
        id: 12,
        modifiers: MOD_CONTROL | MOD_SHIFT,
        virtual_key: 0x34,
        action: Action::Scroll,
        label: "Strg+Umschalt+4",
    },
];

pub fn registration_for_id(id: i32) -> Option<Registration> {
    REGISTRATIONS
        .iter()
        .copied()
        .find(|registration| registration.id == id)
}

pub fn action_for_id(id: usize) -> Option<Action> {
    i32::try_from(id)
        .ok()
        .and_then(registration_for_id)
        .map(|registration| registration.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_screen_matrix_matches_csharp_reference_and_dispatch_actions() {
        // C# reference: Program.cs registers Druck, Ctrl+Druck,
        // Shift+Druck, Ctrl+Shift+Druck, Ctrl+Alt+Druck, then Ctrl+Shift+1..4.
        let expected = [
            (4, 0, 0x2c, Action::Region, "Druck"),
            (6, MOD_CONTROL, 0x2c, Action::Screen, "Strg+Druck"),
            (8, MOD_SHIFT, 0x2c, Action::ActiveWindow, "Umschalt+Druck"),
            (
                7,
                MOD_CONTROL | MOD_SHIFT,
                0x2c,
                Action::AllScreens,
                "Strg+Umschalt+Druck",
            ),
            (
                9,
                MOD_CONTROL | MOD_ALT,
                0x2c,
                Action::Scroll,
                "Strg+Alt+Druck",
            ),
            (
                5,
                MOD_CONTROL | MOD_SHIFT,
                0x31,
                Action::Region,
                "Strg+Umschalt+1",
            ),
            (
                10,
                MOD_CONTROL | MOD_SHIFT,
                0x32,
                Action::Screen,
                "Strg+Umschalt+2",
            ),
            (
                11,
                MOD_CONTROL | MOD_SHIFT,
                0x33,
                Action::ActiveWindow,
                "Strg+Umschalt+3",
            ),
            (
                12,
                MOD_CONTROL | MOD_SHIFT,
                0x34,
                Action::Scroll,
                "Strg+Umschalt+4",
            ),
        ];
        for (id, modifiers, virtual_key, action, label) in expected {
            let registration = registration_for_id(id).unwrap();
            assert_eq!(
                (
                    registration.modifiers,
                    registration.virtual_key,
                    registration.action,
                    registration.label
                ),
                (modifiers, virtual_key, action, label)
            );
            assert_eq!(action_for_id(id as usize), Some(action));
        }
    }

    #[test]
    fn rust_only_function_keys_and_unknown_ids_keep_their_routes() {
        assert_eq!(registration_for_id(1).unwrap().virtual_key, 0x78);
        assert_eq!(action_for_id(1), Some(Action::Screen));
        assert_eq!(registration_for_id(2).unwrap().virtual_key, 0x79);
        assert_eq!(action_for_id(2), Some(Action::Scroll));
        assert_eq!(registration_for_id(3).unwrap().virtual_key, 0x7a);
        assert_eq!(action_for_id(3), Some(Action::Region));
        assert_eq!(action_for_id(0), None);
        assert_eq!(action_for_id(13), None);
    }
}
