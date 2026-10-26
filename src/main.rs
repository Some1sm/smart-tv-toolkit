// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod android;
mod app;
mod config;
mod devices;
mod history;
mod jobs;
mod log;
mod mdns;
mod net;
mod platform;
mod prefs;
mod scrcpy;
mod sidebar;
mod tabs;
mod theme;
mod ui;
mod update;

pub fn main() -> iced::Result {
    let window = app::window_settings(&prefs::load());
    iced::application(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window(window)
        .run()
}
