use iced::widget::{button, container};
use iced::{Border, Theme};

pub fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border {
            radius: 14.0.into(),
            width: 1.0,
            color: palette.background.strong.color,
        },
        ..container::Style::default()
    }
}

pub fn bar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            radius: 14.0.into(),
            width: 1.0,
            color: palette.background.strong.color,
        },
        ..container::Style::default()
    }
}

pub fn notice(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.danger.weak.color.into()),
        text_color: Some(palette.danger.weak.text),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The selected option in a row of choices.
pub fn choice(active: bool) -> fn(&Theme, button::Status) -> button::Style {
    if active {
        button::primary
    } else {
        button::secondary
    }
}
