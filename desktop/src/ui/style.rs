//! The look: a near-black background with dark rounded tiles, after Apple's Home app and Home
//! Assistant's Mushroom cards. Each kind of device has an accent colour that lights up its
//! icon when it's on.

use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{button, container, pick_list, scrollable, text, text_input, toggler};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, border};

pub const BACKGROUND: Color = Color::from_rgb8(0x0B, 0x0B, 0x0E);
pub const SURFACE: Color = Color::from_rgb8(0x1C, 0x1C, 0x1F);
pub const RAISED: Color = Color::from_rgb8(0x2A, 0x2A, 0x2E);
pub const HOVER: Color = Color::from_rgb8(0x36, 0x36, 0x3B);
pub const LINE: Color = Color::from_rgb8(0x27, 0x27, 0x2B);
pub const TEXT: Color = Color::from_rgb8(0xF5, 0xF5, 0xF7);
pub const MUTED: Color = Color::from_rgb8(0x8E, 0x8E, 0x93);
pub const FAINT: Color = Color::from_rgb8(0x5C, 0x5C, 0x62);

pub const BLUE: Color = Color::from_rgb8(0x0A, 0x84, 0xFF);
pub const RED: Color = Color::from_rgb8(0xFF, 0x45, 0x3A);
pub const GREEN: Color = Color::from_rgb8(0x30, 0xD1, 0x58);
pub const ORANGE: Color = Color::from_rgb8(0xFF, 0x9F, 0x0A);
pub const CYAN: Color = Color::from_rgb8(0x64, 0xD2, 0xFF);
pub const TEAL: Color = Color::from_rgb8(0x40, 0xC8, 0xE0);
pub const PURPLE: Color = Color::from_rgb8(0xBF, 0x5A, 0xF2);
pub const YELLOW: Color = Color::from_rgb8(0xFF, 0xD6, 0x0A);
pub const INDIGO: Color = Color::from_rgb8(0x5E, 0x5C, 0xE6);

const INTER: Font = Font::with_name("Inter");
pub const REGULAR: Font = INTER;
pub const LIGHT: Font = Font {
    weight: Weight::Light,
    ..INTER
};
pub const MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..INTER
};
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..INTER
};
pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..INTER
};

pub const FONTS: [&[u8]; 5] = [
    include_bytes!("../../assets/fonts/Inter-Light.ttf"),
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/Inter-Bold.ttf"),
];

pub fn theme() -> Theme {
    Theme::custom(
        "Home",
        Palette {
            background: BACKGROUND,
            text: TEXT,
            primary: BLUE,
            success: GREEN,
            warning: ORANGE,
            danger: RED,
        },
    )
}

/// `color` at `alpha` opacity: for tinted backgrounds behind accent icons.
pub fn tint(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

fn lighten(color: Color, amount: f32) -> Color {
    Color {
        r: color.r + (1.0 - color.r) * amount,
        g: color.g + (1.0 - color.g) * amount,
        b: color.b + (1.0 - color.b) * amount,
        a: color.a,
    }
}

// Containers

// No shadows on tiles or the dock: the CPU renderer only repaints a shadow when its box is
// repainted, so scrolling past one leaves it flickering. On near-black they barely show anyway.
pub fn tile(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border::rounded(22).color(LINE).width(1),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn dock(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border::rounded(30).color(LINE).width(1),
        ..container::Style::default()
    }
}

/// A circle (or pill) filled with `color`.
pub fn badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border::rounded(999),
        ..container::Style::default()
    }
}

/// The track a segmented control's segments sit in.
pub fn track(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(RAISED.into()),
        border: border::rounded(14),
        ..container::Style::default()
    }
}

pub fn notice(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tint(RED, 0.14).into()),
        border: border::rounded(16).color(tint(RED, 0.35)).width(1),
        text_color: Some(lighten(RED, 0.45)),
        ..container::Style::default()
    }
}

// Text

pub fn muted(_theme: &Theme) -> text::Style {
    text::Style { color: Some(MUTED) }
}

pub fn faint(_theme: &Theme) -> text::Style {
    text::Style { color: Some(FAINT) }
}

// Buttons

fn button_style(background: Color, text_color: Color, radius: f32) -> button::Style {
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(radius),
        ..button::Style::default()
    }
}

/// A filled button in `color`, e.g. the primary action.
pub fn solid(color: Color, radius: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match status {
        button::Status::Active => button_style(color, Color::WHITE, radius),
        button::Status::Hovered => button_style(lighten(color, 0.12), Color::WHITE, radius),
        button::Status::Pressed => button_style(lighten(color, 0.22), Color::WHITE, radius),
        button::Status::Disabled => {
            button_style(tint(color, 0.35), tint(Color::WHITE, 0.5), radius)
        }
    }
}

/// A grey button that turns `accent` when `active` (a selected segment, a device that's on).
pub fn choice(
    active: bool,
    accent: Color,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, text_color) = if active {
            (accent, BACKGROUND)
        } else {
            (Color::TRANSPARENT, TEXT)
        };
        match status {
            button::Status::Active => button_style(background, text_color, radius),
            button::Status::Hovered | button::Status::Pressed if active => {
                button_style(lighten(accent, 0.15), text_color, radius)
            }
            button::Status::Hovered | button::Status::Pressed => {
                button_style(HOVER, text_color, radius)
            }
            button::Status::Disabled if active => {
                button_style(tint(accent, 0.3), tint(BACKGROUND, 0.7), radius)
            }
            button::Status::Disabled => button_style(Color::TRANSPARENT, FAINT, radius),
        }
    }
}

/// A raised grey button: round icon buttons, the − and + of a thermostat.
pub fn raised(radius: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match status {
        button::Status::Active => button_style(RAISED, TEXT, radius),
        button::Status::Hovered | button::Status::Pressed => button_style(HOVER, TEXT, radius),
        button::Status::Disabled => button_style(tint(RAISED, 0.5), FAINT, radius),
    }
}

/// A button that is only text until hovered.
pub fn ghost(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match status {
        button::Status::Active => button_style(Color::TRANSPARENT, color, 12.0),
        button::Status::Hovered | button::Status::Pressed => {
            button_style(tint(color, 0.12), color, 12.0)
        }
        button::Status::Disabled => button_style(Color::TRANSPARENT, FAINT, 12.0),
    }
}

// Inputs

pub fn input(radius: f32) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let border_color = match status {
            text_input::Status::Focused { .. } => tint(BLUE, 0.8),
            text_input::Status::Hovered => HOVER,
            _ => Color::TRANSPARENT,
        };
        text_input::Style {
            background: RAISED.into(),
            border: border::rounded(radius).color(border_color).width(1.5),
            icon: MUTED,
            placeholder: FAINT,
            value: TEXT,
            selection: tint(BLUE, 0.45),
        }
    }
}

pub fn select(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border_color = match status {
        pick_list::Status::Opened { .. } => tint(BLUE, 0.8),
        pick_list::Status::Hovered => HOVER,
        pick_list::Status::Active => Color::TRANSPARENT,
    };
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: FAINT,
        handle_color: MUTED,
        background: RAISED.into(),
        border: border::rounded(12).color(border_color).width(1.5),
    }
}

pub fn select_menu(_theme: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: RAISED.into(),
        border: border::rounded(12).color(LINE).width(1),
        text_color: TEXT,
        selected_text_color: Color::WHITE,
        selected_background: BLUE.into(),
        shadow: Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.5),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
    }
}

pub fn switch(_theme: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, disabled) = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Hovered { is_toggled } => {
            (is_toggled, false)
        }
        toggler::Status::Disabled { is_toggled } => (is_toggled, true),
    };
    let background = match (on, disabled) {
        (true, false) => GREEN,
        (true, true) => tint(GREEN, 0.4),
        (false, _) => HOVER,
    };
    toggler::Style {
        background: background.into(),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Color::WHITE.into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(TEXT),
        border_radius: None,
        padding_ratio: 0.1,
    }
}

pub fn scroller(_theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let active = matches!(
        status,
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        } | scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        }
    );
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: (if active { HOVER } else { RAISED }).into(),
            border: border::rounded(999),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: RAISED.into(),
            border: border::rounded(999),
            shadow: Shadow::default(),
            icon: TEXT,
        },
    }
}
