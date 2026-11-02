//! Shared, platform-independent parts of the Rust ShotKit work in progress.
pub mod alpha_composite;
pub mod alpha_render;
pub mod capture_output;
pub mod document;
pub mod editor;
pub mod image_edit;
pub mod image_io;
pub mod output_windows;
#[cfg(windows)]
pub mod render;
pub mod scroll_capture;
pub mod scroll_match;
pub mod settings;
pub mod shapes;
pub mod stitch;
pub mod view;
