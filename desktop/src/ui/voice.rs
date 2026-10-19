//! The voice bar: the talk button, what was heard and the reply, and a box for typed commands.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};
use iot_desktop::ha::assist::VoiceEvent;

use super::{Message, style};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Listening,
    Thinking,
    Speaking,
}

#[derive(Debug, Default)]
pub struct Voice {
    pub phase: Phase,
    heard: Option<String>,
    reply: Option<String>,
    error: Option<String>,
}

impl Voice {
    /// Applies an event; returns text worth a notification when the window is hidden.
    pub fn apply(&mut self, event: VoiceEvent) -> Option<String> {
        match event {
            VoiceEvent::Woke => {
                self.heard = None;
                self.reply = None;
                self.error = None;
            }
            VoiceEvent::Listening => {
                self.phase = Phase::Listening;
                self.heard = None;
                self.reply = None;
                self.error = None;
            }
            VoiceEvent::Heard(heard) => {
                self.heard = Some(heard);
                self.reply = None;
                self.error = None;
            }
            VoiceEvent::Thinking => self.phase = Phase::Thinking,
            VoiceEvent::Reply(reply) => {
                self.reply = Some(reply.clone());
                return (!reply.is_empty()).then_some(reply);
            }
            VoiceEvent::Speaking => self.phase = Phase::Speaking,
            VoiceEvent::Done => self.phase = Phase::Idle,
            VoiceEvent::Error(error) => {
                self.phase = Phase::Idle;
                self.error = Some(error.clone());
                return Some(error);
            }
        }
        None
    }

    pub fn fail(&mut self, error: String) {
        self.phase = Phase::Idle;
        self.error = Some(error);
    }
}

pub fn view<'a>(voice: &'a Voice, command: &'a str, online: bool) -> Element<'a, Message> {
    let talk_label = match voice.phase {
        Phase::Listening => "Stop",
        _ => "Talk",
    };
    let talk = button(
        text(talk_label)
            .size(18)
            .align_x(Alignment::Center)
            .width(Length::Fill),
    )
    .on_press_maybe(online.then_some(Message::Talk))
    .style(if voice.phase == Phase::Listening {
        button::danger
    } else {
        button::primary
    })
    .padding([12, 18])
    .width(Length::Fixed(110.0));

    let status = match (voice.phase, online) {
        (_, false) => "Not connected",
        (Phase::Listening, _) => "Listening…",
        (Phase::Thinking, _) => "Thinking…",
        (Phase::Speaking, _) => "Speaking…",
        (Phase::Idle, _) => "Press Talk or your shortcut, then say a command",
    };
    let mut lines = column![text(status).style(text::secondary)].spacing(4);
    if let Some(heard) = &voice.heard {
        lines = lines.push(text(format!("You: {heard}")));
    }
    if let Some(reply) = &voice.reply {
        lines = lines.push(text(format!("Home: {reply}")).style(text::primary));
    }
    if let Some(error) = &voice.error {
        lines = lines.push(text(error.as_str()).style(text::danger));
    }

    let typed = text_input("Or type a command", command)
        .on_input_maybe(online.then_some(Message::CommandChanged as fn(String) -> Message))
        .on_submit(Message::SendCommand)
        .padding(10)
        .width(Length::Fixed(300.0));

    container(
        row![talk, lines.width(Length::Fill), typed]
            .spacing(16)
            .align_y(Alignment::Center),
    )
    .padding(14)
    .style(style::bar)
    .into()
}
