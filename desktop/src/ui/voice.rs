//! The voice dock: the mic button, what was heard and the reply, and a box for typed commands.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Color, Element, Length};
use iot_desktop::ha::assist::VoiceEvent;

use super::icon::{Icon, icon};
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
    let (glyph, fill, glyph_color) = match voice.phase {
        Phase::Idle => (Icon::Mic, style::BLUE, Color::WHITE),
        Phase::Listening => (Icon::Square, style::RED, Color::WHITE),
        Phase::Thinking => (Icon::Loader, style::RAISED, style::TEXT),
        Phase::Speaking => (Icon::Volume, style::TEAL, style::BACKGROUND),
    };
    let mic = button(container(icon(glyph, 24.0, glyph_color)).center(Length::Fill))
        .width(56)
        .height(56)
        .padding(0)
        .on_press_maybe(online.then_some(Message::Talk))
        .style(style::solid(fill, 28.0));

    // The first line says what's happening; the second, what was heard or answered.
    let (title, title_color, detail, detail_color) = match (voice.phase, online) {
        (_, false) => (
            "Not connected".to_owned(),
            style::MUTED,
            "Voice comes back when Home Assistant does.".to_owned(),
            style::FAINT,
        ),
        (Phase::Listening, _) => (
            "Listening…".to_owned(),
            style::TEXT,
            "Say a command. It stops when you do.".to_owned(),
            style::MUTED,
        ),
        _ => match (&voice.heard, &voice.reply, &voice.error) {
            (heard, _, Some(error)) => (
                heard
                    .as_ref()
                    .map_or("Something went wrong".to_owned(), |heard| {
                        format!("“{heard}”")
                    }),
                style::TEXT,
                error.clone(),
                style::tint(style::RED, 0.9),
            ),
            (Some(heard), reply, None) => (
                format!("“{heard}”"),
                style::TEXT,
                match (voice.phase, reply) {
                    (Phase::Thinking, _) => "Thinking…".to_owned(),
                    (_, Some(reply)) => reply.clone(),
                    (_, None) => String::new(),
                },
                style::MUTED,
            ),
            (None, _, None) => (
                "Ask Home".to_owned(),
                style::TEXT,
                "Click the mic or press your shortcut, then talk.".to_owned(),
                style::MUTED,
            ),
        },
    };
    let lines = column![
        text(title)
            .font(style::SEMIBOLD)
            .size(15)
            .color(title_color),
        text(detail).size(13).color(detail_color),
    ]
    .spacing(3)
    .width(Length::FillPortion(3));

    let typed = text_input("Type a command…", command)
        .on_input_maybe(online.then_some(Message::CommandChanged as fn(String) -> Message))
        .on_submit(Message::SendCommand)
        .padding([13, 18])
        .size(14)
        .style(style::input(22.0))
        .width(Length::FillPortion(2));
    let ready = online && !command.trim().is_empty();
    let send = button(
        container(icon(
            Icon::Send,
            18.0,
            if ready { Color::WHITE } else { style::FAINT },
        ))
        .center(Length::Fill),
    )
    .width(44)
    .height(44)
    .padding(0)
    .on_press_maybe(ready.then_some(Message::SendCommand))
    .style(style::solid(style::BLUE, 22.0));

    container(
        row![mic, lines, typed, send]
            .spacing(14)
            .align_y(Alignment::Center),
    )
    .padding([10, 12])
    .style(style::dock)
    .into()
}
