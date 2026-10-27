// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod android;
mod app;
mod config;
mod credentials;
mod design;
mod devices;
mod history;
mod i18n;
mod journal;
mod jobs;
mod lg;
mod log;
mod mdns;
mod net;
mod platform;
mod prefs;
mod samsung;
mod scrcpy;
mod sidebar;
mod speedtests;
mod tabs;
mod ui;
mod update;
mod ws;

pub fn main() -> iced::Result {
    let prefs = prefs::load();
    i18n::set(i18n::pick(prefs.language.as_deref()));
    let window = app::window_settings(&prefs);
    iced::application(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .scale_factor(app::App::scale_factor)
        .subscription(app::App::subscription)
        .window(window)
        .font(design::INTER_BYTES)
        .font(design::MONO_BYTES)
        .default_font(design::SANS)
        .run()
}
