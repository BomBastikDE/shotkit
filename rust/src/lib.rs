//! Shared, platform-independent parts of the Rust ShotKit work in progress.
pub mod editor;
pub mod image_edit;
pub mod image_io;
#[cfg(windows)]
pub mod render;
pub mod scroll_capture;
pub mod scroll_match;
pub mod shapes;
pub mod stitch;
pub mod view;
