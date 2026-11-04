//! Shared, platform-independent parts of the Rust ShotKit work in progress.
pub mod alpha_composite;
pub mod alpha_document;
pub mod alpha_render;
pub mod capture_output;
pub mod clipboard_retry;
pub mod delayed_capture;
pub mod document;
pub mod editor;
pub mod hotkey_action;
pub mod image_edit;
pub mod image_io;
pub mod mouse_history;
pub mod network_path;
pub mod output_windows;
pub mod pixelate;
#[cfg(windows)]
pub mod render;
pub mod save_path;
pub mod scroll_capture;
pub mod scroll_completion;
pub mod scroll_match;
pub mod selection_rect;
pub mod settings;
pub mod shapes;
pub mod stitch;
pub mod toolbar_layout;
pub mod view;
