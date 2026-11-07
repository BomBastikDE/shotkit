//! Frozen v1 schema for historical reset receipts. Do not extend this schema
//! with the live Settings type; add a new receipt protocol for future formats.
use serde::Deserialize;
use std::path::PathBuf;

// Frozen receipt-protocol schema, intentionally independent of Settings and
// its future defaults/fields/enums. These fields are used for deserialization
// validation only; omitted historical preferences were legal v1 defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptSettingsV1 {
    version: u32,
    #[serde(default)]
    save_folder: Option<PathBuf>,
    #[serde(default)]
    open_editor: bool,
    #[serde(default)]
    copy_to_clipboard: bool,
    #[serde(default)]
    exclude_from_clipboard_history: bool,
    #[serde(default)]
    auto_save: bool,
    #[serde(default)]
    scroll_direction: ReceiptDirectionV1,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReceiptDirectionV1 {
    #[default]
    Down,
    Right,
    Left,
}

pub(super) fn valid_v1_settings(bytes: &[u8]) -> bool {
    super::recovery_version::validate_v1_source(bytes).is_ok()
        && serde_json::from_slice::<ReceiptSettingsV1>(bytes).is_ok_and(|value| {
            let ReceiptSettingsV1 {
                version,
                save_folder,
                open_editor,
                copy_to_clipboard,
                exclude_from_clipboard_history,
                auto_save,
                scroll_direction,
            } = value;
            // Deserialize has checked every preference against its frozen
            // type. Consume the entire validated payload; only the version
            // participates in the remaining receipt admission decision.
            drop((
                save_folder,
                open_editor,
                copy_to_clipboard,
                exclude_from_clipboard_history,
                auto_save,
                scroll_direction,
            ));
            version == 1
        })
}

/// Both the live loader and the historical receipt schema must reject the
/// source. Otherwise a newer loader could reset a formerly valid v1 document
/// and generate completion evidence that its own historical checker rejects.
pub(super) fn admit_reset_source(bytes: &[u8], live_rejected: bool) -> bool {
    live_rejected
        && super::recovery_version::validate_v1_source(bytes).is_ok()
        && !valid_v1_settings(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_v1_schema_is_independent_of_live_defaults() {
        for bytes in [
            br#"{"version":1}"#.as_slice(),
            br#"{"version":1,"open_editor":false,"auto_save":true,"scroll_direction":"left"}"#,
            br#"{"version":1,"save_folder":"C:\\old\\folder","exclude_from_clipboard_history":false}"#,
        ] {
            assert!(valid_v1_settings(bytes));
        }
        for bytes in [
            br#"[1]"#.as_slice(),
            br#"{"version":2}"#,
            br#"{"version":1,"version":1}"#,
            br#"{"version":1,"open_editor":false,"open_editor":true}"#,
            br#"{"version":1,"future_added_preference":false}"#,
            br#"{"version":1,"scroll_direction":"future_direction"}"#,
            br#"{"version":1,"auto_save":"false"}"#,
        ] {
            assert!(!valid_v1_settings(bytes));
        }
    }

    #[test]
    fn future_live_rejection_of_historically_valid_v1_never_authorizes_reset() {
        let formerly_valid = br#"{"version":1,"open_editor":false}"#;
        assert!(valid_v1_settings(formerly_valid));
        // Model a future loader rejecting this old document. The actual
        // admission function used before intent creation must still refuse it.
        assert!(!admit_reset_source(formerly_valid, true));
        assert!(!admit_reset_source(formerly_valid, false));

        let invalid_in_both = br#"{"version":1,"open_editor":"wrong type"}"#;
        assert!(admit_reset_source(invalid_in_both, true));
        assert!(!admit_reset_source(invalid_in_both, false));
        assert!(!admit_reset_source(br#"{"version":2}"#, true));
    }
}
