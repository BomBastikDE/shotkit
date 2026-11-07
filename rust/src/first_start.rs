//! Decision for the introductory message on an untouched user profile.

/// A configured autostart suppresses the message, including existing entries
/// created by older versions that have no distinct command-line argument.
pub fn show_notice(
    settings_missing: bool,
    settings_writable: bool,
    has_arguments: bool,
    autostart_enabled: Option<bool>,
) -> bool {
    settings_missing && settings_writable && !has_arguments && autostart_enabled == Some(false)
}

#[cfg(test)]
mod tests {
    use super::show_notice;

    #[test]
    fn notice_requires_an_unconfigured_writable_profile_and_manual_start() {
        assert!(show_notice(true, true, false, Some(false)));
        assert!(!show_notice(false, true, false, Some(false)));
        assert!(!show_notice(true, false, false, Some(false)));
        assert!(!show_notice(true, true, true, Some(false)));
        assert!(!show_notice(true, true, false, Some(true)));
        assert!(!show_notice(true, true, false, None));
    }
}
