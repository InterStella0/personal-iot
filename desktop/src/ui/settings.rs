//! Settings: which mic to use, always-listen, the hotkey, and signing out.

use std::fmt;

use iced::widget::{button, column, container, pick_list, row, scrollable, text, toggler};
use iced::{Alignment, Element, Length};
use iot_desktop::audio::mic::Mic;
use iot_desktop::config::Config;

use super::{Message, style};

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
    shortcut: Option<&'a str>,
) -> Element<'a, Message> {
    let choices = choices(mics, config.mic.as_deref());
    let selected = choices
        .iter()
        .find(|choice| choice.name == config.mic)
        .cloned();
    let bluetooth = config
        .mic
        .as_deref()
        .is_some_and(|name| mics.iter().any(|mic| mic.name == name && mic.bluetooth));

    let mut microphone = column![
        text("Microphone").size(18),
        row![
            pick_list(choices, selected, Message::MicSelected).width(Length::Fill),
            button("Refresh")
                .on_press(Message::RefreshMics)
                .style(button::secondary),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(8);
    if bluetooth {
        microphone = microphone.push(
            text("Bluetooth mics switch the device to call audio while they're in use, so its sound gets worse.")
                .style(text::danger),
        );
    }

    let wake = column![
        toggler(config.wake_word)
            .label("Always listen for the wake word")
            .on_toggle(Message::WakeWord),
        text(
            "Streams the microphone to your Home Assistant all the time. Needs a wake word \
             engine (openWakeWord) set on the assistant in Home Assistant.",
        )
        .style(text::secondary),
    ]
    .spacing(8);

    let hotkey = column![
        text("Talk shortcut").size(18),
        text(match shortcut {
            Some(problem) => format!("Not available: {problem}"),
            None => "Change it in System Settings → Keyboard → Shortcuts → Home.".into(),
        })
        .style(text::secondary),
    ]
    .spacing(8);

    let account = column![
        text("Home Assistant").size(18),
        text(format!("Signed in to {}", config.server)).style(text::secondary),
        button("Sign out")
            .on_press(Message::SignOut)
            .style(button::danger),
    ]
    .spacing(8);

    let content = column![
        row![
            text("Settings").size(26).width(Length::Fill),
            button("Done")
                .on_press(Message::ToggleSettings)
                .style(button::primary),
        ]
        .align_y(Alignment::Center),
        section(microphone),
        section(wake),
        section(hotkey),
        section(account),
    ]
    .spacing(16)
    .max_width(640);
    scrollable(container(content).center_x(Length::Fill)).into()
}

fn section<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(18)
        .width(Length::Fill)
        .style(style::card)
        .into()
}
