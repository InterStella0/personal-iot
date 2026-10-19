//! Home: a desktop app for Home Assistant, with device buttons and voice.
//!
//! `iot-desktop [--hidden]`. `--hidden` starts in the tray (used by autostart).

mod shortcut;
mod tray;
mod ui;

use std::path::PathBuf;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,iot_desktop=info".into()),
        )
        .init();

    let hidden = std::env::args().any(|arg| arg == "--hidden");
    // For checking the layout: IOT_DESKTOP_SCREENSHOT=/tmp/home.png saves the window and quits.
    let screenshot = std::env::var_os("IOT_DESKTOP_SCREENSHOT").map(PathBuf::from);
    // IOT_DESKTOP_SCREENSHOT_VIEW=settings, or room:<area> to show one area.
    let view = std::env::var("IOT_DESKTOP_SCREENSHOT_VIEW").unwrap_or_default();
    let start_in_settings = view == "settings";
    let start_room = view.strip_prefix("room:").map(str::to_owned);

    iced::daemon(
        move || {
            ui::App::new(ui::Options {
                hidden: hidden && screenshot.is_none(),
                screenshot: screenshot.clone(),
                start_in_settings,
                start_room: start_room.clone(),
            })
        },
        ui::App::update,
        ui::App::view,
    )
    .font(ui::style::FONTS[0])
    .font(ui::style::FONTS[1])
    .font(ui::style::FONTS[2])
    .font(ui::style::FONTS[3])
    .font(ui::style::FONTS[4])
    .default_font(ui::style::REGULAR)
    .title(ui::App::title)
    .theme(ui::App::theme)
    .subscription(ui::App::subscription)
    .run()
}
