//! Settings: which assistant and mic to use, always-listen, the hotkey, and signing out.

use std::fmt;

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Column, button, column, container, pick_list, row, scrollable, text, toggler};
use iced::{Alignment, Color, Element, Length, Padding};
use iot_desktop::audio::mic::Mic;
use iot_desktop::config::Config;
use iot_desktop::ha::assist::Assistant;

use super::icon::{Icon, icon};
use super::{Message, style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantChoice {
    /// `None` follows HA's preferred assistant.
    pub id: Option<String>,
    label: String,
}

impl fmt::Display for AssistantChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

fn assistant_choices(assistants: &[Assistant], preferred: Option<&str>) -> Vec<AssistantChoice> {
    let preferred_name = assistants
        .iter()
        .find(|assistant| Some(assistant.id.as_str()) == preferred)
        .map(|assistant| assistant.name.as_str());
    let mut choices = vec![AssistantChoice {
        id: None,
        label: match preferred_name {
            Some(name) => format!("Default ({name})"),
            None => "Default".into(),
        },
    }];
    choices.extend(assistants.iter().map(|assistant| AssistantChoice {
        id: Some(assistant.id.clone()),
        label: assistant.name.clone(),
    }));
    choices
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicChoice {
    /// `None` is the system default.
    pub name: Option<String>,
    label: String,
}

impl fmt::Display for MicChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

fn choices(mics: &[Mic], current: Option<&str>) -> Vec<MicChoice> {
    let mut choices = vec![MicChoice {
        name: None,
        label: "System default".into(),
    }];
    choices.extend(mics.iter().map(|mic| MicChoice {
        name: Some(mic.name.clone()),
        label: if mic.bluetooth {
            format!("{} (Bluetooth)", mic.description)
        } else {
            mic.description.clone()
        },
    }));
    // A saved mic that's unplugged right now still shows as selected.
    if let Some(current) = current
        && !mics.iter().any(|mic| mic.name == current)
    {
        choices.push(MicChoice {
            name: Some(current.to_owned()),
            label: format!("{current} (not connected)"),
        });
    }
    choices
}

pub fn view<'a>(
    config: &'a Config,
    mics: &'a [Mic],
    assistants: &'a [Assistant],
    preferred: Option<&'a str>,
    shortcut: Option<&'a str>,
) -> Element<'a, Message> {
    let assistant_options = assistant_choices(assistants, preferred);
    let assistant_selected = assistant_options
        .iter()
        .find(|choice| choice.id == config.assistant)
        .cloned();
    let mic_options = choices(mics, config.mic.as_deref());
    let mic_selected = mic_options
        .iter()
        .find(|choice| choice.name == config.mic)
        .cloned();
    let bluetooth = config
        .mic
        .as_deref()
        .is_some_and(|name| mics.iter().any(|mic| mic.name == name && mic.bluetooth));

    let voice = group(
        "Voice",
        vec![
            setting(
                Icon::Sparkles,
                style::PURPLE,
                "Assistant",
                "Default follows the assistant preferred in Home Assistant.".into(),
                None,
                Some(select(assistant_options, assistant_selected, Message::AssistantSelected)),
            ),
            setting(
                Icon::Mic,
                style::RED,
                "Microphone",
                "Used for the mic button, your shortcut and the wake word.".into(),
                bluetooth.then_some(
                    "Bluetooth mics switch the device to call audio while in use, so its sound gets worse.",
                ),
                Some(
                    row![
                        select(mic_options, mic_selected, Message::MicSelected),
                        button(container(icon(Icon::Refresh, 16.0, style::TEXT)).center(Length::Fill))
                            .width(40)
                            .height(40)
                            .padding(0)
                            .on_press(Message::RefreshMics)
                            .style(style::raised(12.0)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
                ),
            ),
            setting(
                Icon::AudioLines,
                style::GREEN,
                "Always listen",
                "Say “okay nabu” instead of pressing anything. Streams the mic to Home Assistant \
                 all the time; needs openWakeWord on the assistant."
                    .into(),
                None,
                Some(
                    toggler(config.wake_word)
                        .on_toggle(Message::WakeWord)
                        .size(26)
                        .style(style::switch)
                        .into(),
                ),
            ),
            setting(
                Icon::Keyboard,
                style::INDIGO,
                "Talk shortcut",
                match shortcut {
                    Some(problem) => format!("Not available: {problem}"),
                    None => "Ctrl+Alt+H unless you changed it. Change it in System Settings → \
                             Keyboard → Shortcuts → Home."
                        .into(),
                },
                None,
                None,
            ),
        ],
    );

    let account = group(
        "Account",
        vec![setting(
            Icon::User,
            style::BLUE,
            "Home Assistant",
            format!("Signed in to {}", config.server),
            None,
            Some(
                button(
                    row![
                        icon(Icon::LogOut, 16.0, style::RED),
                        text("Sign out").font(style::MEDIUM).size(14),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([9, 14])
                .on_press(Message::SignOut)
                .style(style::ghost(style::RED))
                .into(),
            ),
        )],
    );

    let content = column![
        row![
            text("Settings")
                .font(style::BOLD)
                .size(28)
                .width(Length::Fill),
            button(text("Done").font(style::SEMIBOLD).size(14))
                .padding([9, 18])
                .on_press(Message::ToggleSettings)
                .style(style::solid(style::BLUE, 18.0)),
        ]
        .align_y(Alignment::Center),
        voice,
        account,
    ]
    .spacing(26)
    .max_width(760)
    .padding(Padding {
        bottom: 24.0,
        ..Padding::ZERO
    });
    scrollable(container(content).center_x(Length::Fill))
        .direction(Direction::Vertical(
            Scrollbar::new().width(6).scroller_width(6).spacing(14),
        ))
        .style(style::scroller)
        .into()
}

fn select<'a, T: Clone + PartialEq + std::fmt::Display + 'a>(
    options: Vec<T>,
    selected: Option<T>,
    on_select: fn(T) -> Message,
) -> Element<'a, Message> {
    pick_list(options, selected, on_select)
        .width(Length::Fixed(280.0))
        .padding([10, 14])
        .text_size(14)
        .style(style::select)
        .menu_style(style::select_menu)
        .into()
}

/// A titled group of rows on one tile, like iOS Settings.
fn group<'a>(title: &'a str, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut list = Column::new();
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        list = list.push(row);
        if index + 1 < count {
            list = list.push(
                container(text(""))
                    .height(1)
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(style::LINE.into()),
                        ..container::Style::default()
                    }),
            );
        }
    }
    column![
        text(title.to_uppercase())
            .font(style::SEMIBOLD)
            .size(12)
            .style(style::faint),
        container(list).padding([4, 18]).style(style::tile),
    ]
    .spacing(10)
    .into()
}

fn setting<'a>(
    glyph: Icon,
    color: Color,
    title: &'a str,
    detail: String,
    warning: Option<&'a str>,
    control: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let badge = container(icon(glyph, 18.0, Color::WHITE))
        .center(Length::Fixed(34.0))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: iced::border::rounded(10),
            ..container::Style::default()
        });
    let mut words = column![
        text(title).font(style::SEMIBOLD).size(15),
        text(detail).size(13).style(style::muted),
    ]
    .spacing(3)
    .width(Length::Fill);
    if let Some(warning) = warning {
        words = words.push(text(warning).size(13).color(style::ORANGE));
    }
    let mut line = row![badge, words].spacing(14).align_y(Alignment::Center);
    if let Some(control) = control {
        line = line.push(control);
    }
    container(line).padding([16, 0]).into()
}
