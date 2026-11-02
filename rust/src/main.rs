#![windows_subsystem = "windows"]
mod autostart;
mod about_dialog;
mod capture_targets;
mod clipboard;
mod element_picker;
mod file_dialog;
mod folder_dialog;
mod last_region;
mod scroll_windows;
mod overlay;
mod text_dialog;
mod toolbar;
mod uia_picker;
mod app;

fn main() {
    app::run();
}
