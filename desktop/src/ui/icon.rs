//! Lucide icons (ISC licence, assets/icons), drawn as single-colour SVGs.

use iced::widget::{Svg, svg};
use iced::{Color, Theme};

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon { $($name,)* }

        impl Icon {
            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$name => include_bytes!(concat!("../../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    AirVent => "air-vent",
    AudioLines => "audio-lines",
    CircleAlert => "circle-alert",
    CloudOff => "cloud-off",
    Droplets => "droplets",
    Fan => "fan",
    Flame => "flame",
    House => "house",
    Keyboard => "keyboard",
    Lightbulb => "lightbulb",
    Loader => "loader-circle",
    LogOut => "log-out",
    Mic => "mic",
    Minus => "minus",
    Plug => "plug",
    Plus => "plus",
    Power => "power",
    Refresh => "refresh-cw",
    Send => "send-horizontal",
    Settings => "settings",
    ShieldCheck => "shield-check",
    Snowflake => "snowflake",
    Sparkles => "sparkles",
    Square => "square",
    SunSnow => "sun-snow",
    User => "user",
    Volume => "volume-2",
    Wind => "wind",
    X => "x",
}

/// `icon` at `size` pixels, in `color`.
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Svg<'a, Theme> {
    svg(svg::Handle::from_memory(icon.bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}
