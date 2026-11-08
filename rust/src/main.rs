#![windows_subsystem = "windows"]
#[cfg(windows)]
mod about_dialog;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod capture_targets;
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod element_picker;
#[cfg(windows)]
mod file_dialog;
#[cfg(windows)]
mod folder_dialog;
#[cfg(windows)]
mod last_region;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod scroll_end_uia;
#[cfg(windows)]
mod scroll_windows;
#[cfg(windows)]
mod text_dialog;
#[cfg(windows)]
mod toolbar;
#[cfg(windows)]
mod uia_picker;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ShotKit requires Windows.");
}
