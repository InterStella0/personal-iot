//! Device cards, grouped by area. Cards are generic by domain, so new devices just appear.

use std::collections::BTreeMap;

use iced::widget::{Row, button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding};
use iot_desktop::ha::entity::{self, CONTROLLABLE, Registry, State};
use serde_json::{Value, json};

use super::{Message, style};

const CARD_WIDTH: f32 = 320.0;
const OTHER: &str = "Other";

/// Areas in name order with "Other" last, each with its devices in name order.
fn sections<'a>(
    states: &'a BTreeMap<String, State>,
    registry: &'a Registry,
) -> Vec<(&'a str, Vec<&'a State>)> {
    let mut sections: BTreeMap<(bool, &str), Vec<&State>> = BTreeMap::new();
    for state in states.values() {
        if !CONTROLLABLE.contains(&state.domain()) || registry.is_hidden(&state.entity_id) {
            continue;
        }
        let area = registry.area_name(&state.entity_id);
        sections
            .entry((area.is_none(), area.unwrap_or(OTHER)))
            .or_default()
            .push(state);
    }
    sections
        .into_iter()
        .map(|((_, area), mut states)| {
            states.sort_by_key(|state| state.name().to_lowercase());
            (area, states)
        })
        .collect()
}

pub fn view<'a>(
    states: &'a BTreeMap<String, State>,
    registry: &'a Registry,
    loaded: bool,
) -> Element<'a, Message> {
    let sections = sections(states, registry);
    if sections.is_empty() {
        let message = if loaded {
            "No devices to show. Climate, fan, light and switch entities appear here."
        } else {
            "Loading devices…"
        };
        return container(text(message).style(text::secondary))
            .center(Length::Fill)
            .into();
    }
    // Room on the right so the scrollbar doesn't cover the last column of cards.
    let mut content = column![].spacing(22).width(Length::Fill).padding(Padding {
        top: 4.0,
        right: 20.0,
        bottom: 4.0,
        left: 4.0,
    });
    for (area, states) in sections {
        // Fill the width, or the cards wrap at whatever width the widest row happened to need.
        let cards = Row::with_children(states.into_iter().map(card))
            .spacing(16)
            .width(Length::Fill)
            .wrap()
            .vertical_spacing(16);
        content = content.push(column![text(area).size(20), cards].spacing(10));
    }
    scrollable(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn status(state: &State) -> String {
    if !state.available() {
        return "Offline".into();
    }
    match state.domain() {
        "climate" if state.state != "off" => {
            let mut parts = vec![label(&state.state)];
            if let Some(target) = state.number("temperature") {
                parts.push(format!("{}°", trim(target)));
            }
            if let Some(current) = state.number("current_temperature") {
                parts.push(format!("room {}°", trim(current)));
            }
            parts.join(" · ")
        }
        "fan" => match state.fan_speed() {
            Some(speed) => format!("Speed {speed}"),
            None if state.is_on() => "On".into(),
            None => "Off".into(),
        },
        _ => label(&state.state),
    }
}

fn card(state: &State) -> Element<'_, Message> {
    let header = row![
        text(state.name()).size(18).width(Length::Fill),
        text(status(state)).style(if state.available() {
            text::secondary
        } else {
            text::danger
        }),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let controls: Element<'_, Message> = match state.domain() {
        "climate" => climate(state),
        "fan" => fan(state),
        _ => on_off(state),
    };
    container(column![header, controls].spacing(14))
        .padding(16)
        .width(Length::Fixed(CARD_WIDTH))
        .style(style::card)
        .into()
}

/// A button that calls a service, disabled while the device is offline.
fn action<'a>(
    state: &'a State,
    caption: String,
    active: bool,
    service: &str,
    data: Value,
) -> Element<'a, Message> {
    let press = state
        .available()
        .then(|| Message::Call(state.entity_id.clone(), service.to_owned(), data));
    button(text(caption).align_x(Alignment::Center))
        .on_press_maybe(press)
        .style(style::choice(active))
        .padding([6, 12])
        .into()
}

fn climate(state: &State) -> Element<'_, Message> {
    let on = state.is_on();
    let mut controls = column![].spacing(10);

    let mut modes = Row::new().spacing(6).push(action(
        state,
        if on { "Turn off" } else { "Turn on" }.into(),
        false,
        if on { "turn_off" } else { "turn_on" },
        json!({}),
    ));
    for mode in state
        .list("hvac_modes")
        .into_iter()
        .filter(|mode| mode != "off")
    {
        let active = on && state.state == mode;
        modes = modes.push(action(
            state,
            label(&mode),
            active,
            "set_hvac_mode",
            json!({"hvac_mode": mode}),
        ));
    }
    controls = controls.push(modes.wrap().vertical_spacing(6));

    if let Some(target) = state.number("temperature") {
        let step = state.number("target_temp_step").unwrap_or(1.0);
        let min = state.number("min_temp").unwrap_or(16.0);
        let max = state.number("max_temp").unwrap_or(30.0);
        let lower = (target - step).max(min);
        let higher = (target + step).min(max);
        let unit = if state.text("temperature_unit") == Some("°F") {
            "°F"
        } else {
            "°"
        };
        controls = controls.push(
            row![
                action(
                    state,
                    "−".into(),
                    false,
                    "set_temperature",
                    json!({"temperature": lower})
                ),
                text(format!("{}{unit}", trim(target)))
                    .size(28)
                    .width(Length::Fixed(90.0))
                    .align_x(Alignment::Center),
                action(
                    state,
                    "+".into(),
                    false,
                    "set_temperature",
                    json!({"temperature": higher})
                ),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
    }

    let fan_modes = state.list("fan_modes");
    if !fan_modes.is_empty() {
        let current = state.text("fan_mode").unwrap_or_default().to_owned();
        let mut fans = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(text("Fan").style(text::secondary))
            .push(space().width(4));
        for mode in fan_modes {
            let active = mode == current;
            fans = fans.push(action(
                state,
                label(&mode),
                active,
                "set_fan_mode",
                json!({"fan_mode": mode}),
            ));
        }
        controls = controls.push(fans.wrap().vertical_spacing(6));
    }
    controls.into()
}

fn fan(state: &State) -> Element<'_, Message> {
    let Some(count) = state.fan_speed_count() else {
        return on_off(state);
    };
    let current = state.fan_speed();
    let mut speeds = Row::new().spacing(6).push(action(
        state,
        "Off".into(),
        !state.is_on(),
        "turn_off",
        json!({}),
    ));
    for speed in 1..=count {
        let percentage = entity::percentage_for_speed(speed, count);
        speeds = speeds.push(action(
            state,
            speed.to_string(),
            current == Some(speed),
            "set_percentage",
            json!({"percentage": percentage}),
        ));
    }
    speeds.wrap().vertical_spacing(6).into()
}

fn on_off(state: &State) -> Element<'_, Message> {
    row![
        action(
            state,
            "Off".into(),
            state.available() && !state.is_on(),
            "turn_off",
            json!({})
        ),
        action(state, "On".into(), state.is_on(), "turn_on", json!({})),
    ]
    .spacing(6)
    .into()
}

/// `fan_only` → "Fan only".
pub fn label(value: &str) -> String {
    let words = value.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// 24.0 → "24", 24.5 → "24.5".
fn trim(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
