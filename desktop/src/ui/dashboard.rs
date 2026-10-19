//! Device tiles, grouped by area. Tiles are generic by domain, so new devices just appear.

use std::collections::BTreeMap;

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Column, Row, button, column, container, responsive, row, scrollable, space, text,
};
use iced::{Alignment, Color, Element, Length, Padding};
use iot_desktop::ha::entity::{self, Registry, State};
use serde_json::{Value, json};

use super::icon::{Icon, icon};
use super::{Message, style};

const GAP: f32 = 16.0;
const MIN_TILE: f32 = 300.0;
const SCROLLBAR: f32 = 6.0;
const SCROLL_GAP: f32 = 14.0;
const OTHER: &str = "Other";

pub struct Section<'a> {
    pub area: &'a str,
    pub states: Vec<&'a State>,
}

/// Areas in name order with "Other" last. Within an area, bigger tiles (climate, fans) come
/// first so the small ones line up together.
pub fn sections<'a>(
    states: &'a BTreeMap<String, State>,
    registry: &'a Registry,
) -> Vec<Section<'a>> {
    let mut sections: BTreeMap<(bool, &str), Vec<&State>> = BTreeMap::new();
    for state in states.values() {
        if !state.controllable() || registry.is_hidden(&state.entity_id) {
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
            states.sort_by_key(|state| (rank(state.domain()), state.name().to_lowercase()));
            Section { area, states }
        })
        .collect()
}

fn rank(domain: &str) -> u8 {
    match domain {
        "climate" => 0,
        "fan" => 1,
        "light" => 2,
        _ => 3,
    }
}

pub fn view<'a>(
    sections: Vec<Section<'a>>,
    room: Option<&'a str>,
    loaded: bool,
) -> Element<'a, Message> {
    let sections: Vec<Section<'a>> = sections
        .into_iter()
        .filter(|section| room.is_none_or(|room| room == section.area))
        .collect();
    if sections.is_empty() {
        let (title, detail) = if loaded {
            (
                "No devices yet",
                "Air conditioners, fans, lights and switches in Home Assistant show up here.",
            )
        } else {
            ("Loading devices…", "")
        };
        return container(
            column![
                icon(Icon::House, 40.0, style::FAINT),
                text(title).font(style::SEMIBOLD).size(18),
                text(detail).style(style::muted),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into();
    }
    let titled = room.is_none();

    responsive(move |size| {
        let width = (size.width - SCROLLBAR - SCROLL_GAP).max(MIN_TILE);
        let columns = ((width + GAP) / (MIN_TILE + GAP)).floor().max(1.0) as usize;
        let tile_width = (width - GAP * (columns - 1) as f32) / columns as f32;

        let mut content = Column::new().spacing(30).padding(Padding {
            top: 4.0,
            bottom: 24.0,
            ..Padding::ZERO
        });
        for section in &sections {
            let rows = section.states.chunks(columns).map(|chunk| {
                Row::with_children(chunk.iter().map(|state| tile(state, tile_width)))
                    .spacing(GAP)
                    .into()
            });
            let grid = Column::with_children(rows).spacing(GAP);
            if titled {
                content = content.push(
                    column![
                        row![
                            text(section.area).font(style::SEMIBOLD).size(20),
                            text(section.states.len().to_string())
                                .style(style::faint)
                                .size(15),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                        grid,
                    ]
                    .spacing(14),
                );
            } else {
                content = content.push(grid);
            }
        }
        scrollable(content)
            .direction(Direction::Vertical(
                Scrollbar::new()
                    .width(SCROLLBAR)
                    .scroller_width(SCROLLBAR)
                    .spacing(SCROLL_GAP),
            ))
            .style(style::scroller)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    })
    .into()
}

/// The colour a device glows in when it's on.
fn accent(state: &State) -> Color {
    match state.domain() {
        "climate" => mode_accent(&state.state),
        "fan" => style::TEAL,
        "light" => light_color(state).unwrap_or(style::YELLOW),
        _ => style::INDIGO,
    }
}

fn mode_accent(mode: &str) -> Color {
    match mode {
        "heat" => style::ORANGE,
        "heat_cool" | "auto" => style::GREEN,
        "dry" => style::PURPLE,
        "fan_only" => style::TEAL,
        _ => style::CYAN,
    }
}

/// The light's own colour, brightened enough to read as a glow on a dark tile.
fn light_color(state: &State) -> Option<Color> {
    let rgb = state.attributes.get("rgb_color")?.as_array()?;
    let channel = |i: usize| {
        rgb.get(i)
            .and_then(Value::as_f64)
            .map(|v| (v / 255.0) as f32)
    };
    let (r, g, b) = (channel(0)?, channel(1)?, channel(2)?);
    let peak = r.max(g).max(b);
    if peak < 0.05 {
        return None;
    }
    // Scale up to full brightness, then wash slightly toward white so dim hues stay visible.
    let lift = |c: f32| (c / peak) * 0.85 + 0.15;
    Some(Color::from_rgb(lift(r), lift(g), lift(b)))
}

fn device_icon(state: &State) -> Icon {
    match state.domain() {
        "climate" => match state.state.as_str() {
            "off" => Icon::AirVent,
            mode => mode_icon(mode),
        },
        "fan" => Icon::Fan,
        "light" => Icon::Lightbulb,
        _ => Icon::Plug,
    }
}

fn mode_icon(mode: &str) -> Icon {
    match mode {
        "heat" => Icon::Flame,
        "dry" => Icon::Droplets,
        "fan_only" => Icon::Fan,
        "heat_cool" | "auto" => Icon::SunSnow,
        _ => Icon::Snowflake,
    }
}

fn status(state: &State) -> String {
    if !state.available() {
        return "Offline".into();
    }
    match state.domain() {
        "climate" if state.state != "off" => {
            let mut parts = vec![mode_name(&state.state)];
            if let Some(target) = state.number("temperature") {
                parts.push(format!("{}°", trim(target)));
            }
            parts.join(" · ")
        }
        "fan" => match state.fan_speed() {
            Some(speed) => format!("Speed {speed}"),
            None if state.is_on() => "On".into(),
            None => "Off".into(),
        },
        "light" if state.is_on() => match state.number("brightness") {
            Some(brightness) => format!("On · {}%", (brightness / 255.0 * 100.0).round()),
            None => "On".into(),
        },
        _ if state.is_on() => "On".into(),
        _ => "Off".into(),
    }
}

fn tile(state: &State, width: f32) -> Element<'_, Message> {
    let available = state.available();
    let on = state.is_on();
    let accent = accent(state);

    let (badge_fill, badge_icon) = match (available, on) {
        (true, true) => (style::tint(accent, 0.18), accent),
        (true, false) => (style::RAISED, style::MUTED),
        (false, _) => (style::RAISED, style::FAINT),
    };
    let badge = container(icon(
        if available {
            device_icon(state)
        } else {
            Icon::CloudOff
        },
        22.0,
        badge_icon,
    ))
    .center(Length::Fixed(46.0))
    .style(style::badge(badge_fill));

    let status_color = match (available, on) {
        (false, _) => style::tint(style::RED, 0.85),
        (true, true) => accent,
        (true, false) => style::MUTED,
    };
    let header = row![
        badge,
        column![
            text(state.name()).font(style::SEMIBOLD).size(16),
            text(status(state)).size(13).color(status_color),
        ]
        .spacing(3)
        .width(Length::Fill),
        power(state, accent),
    ]
    .spacing(14)
    .align_y(Alignment::Center);

    let body: Option<Element<'_, Message>> = match state.domain() {
        "climate" => Some(climate(state)),
        "fan" => fan(state),
        _ => None,
    };
    let content = match body {
        Some(body) => column![header, body].spacing(20),
        None => column![header],
    };
    container(content)
        .padding(18)
        .width(Length::Fixed(width))
        .style(style::tile)
        .into()
}

fn call(state: &State, service: &str, data: Value) -> Option<Message> {
    state
        .available()
        .then(|| Message::Call(state.entity_id.clone(), service.to_owned(), data))
}

/// A round button with an icon in the middle.
fn round<'a>(
    glyph: Icon,
    size: f32,
    glyph_color: Color,
    style: impl Fn(&iced::Theme, button::Status) -> button::Style + 'a,
    press: Option<Message>,
) -> Element<'a, Message> {
    button(container(icon(glyph, size * 0.45, glyph_color)).center(Length::Fill))
        .width(size)
        .height(size)
        .padding(0)
        .on_press_maybe(press)
        .style(style)
        .into()
}

fn power(state: &State, accent: Color) -> Element<'_, Message> {
    let on = state.is_on();
    let press = call(state, if on { "turn_off" } else { "turn_on" }, json!({}));
    match (state.available(), on) {
        (true, true) => round(
            Icon::Power,
            40.0,
            style::BACKGROUND,
            style::choice(true, accent, 20.0),
            press,
        ),
        (true, false) => round(Icon::Power, 40.0, style::TEXT, style::raised(20.0), press),
        (false, _) => round(Icon::Power, 40.0, style::FAINT, style::raised(20.0), None),
    }
}

/// Options side by side in a rounded track; the selected one fills with `accent`.
fn segmented<'a>(
    items: Vec<(Element<'a, Message>, bool, Option<Message>)>,
    accent: Color,
) -> Element<'a, Message> {
    let segments = items.into_iter().map(|(content, active, press)| {
        button(container(content).center(Length::Fill))
            .width(Length::Fill)
            .height(34)
            .padding(0)
            .on_press_maybe(press)
            .style(style::choice(active, accent, 11.0))
            .into()
    });
    container(Row::with_children(segments).spacing(4))
        .padding(4)
        .width(Length::Fill)
        .style(style::track)
        .into()
}

fn segment_label<'a>(label: String, active: bool, enabled: bool) -> Element<'a, Message> {
    let color = match (enabled, active) {
        (false, _) => style::FAINT,
        (true, true) => style::BACKGROUND,
        (true, false) => style::TEXT,
    };
    text(label)
        .size(13)
        .font(if active {
            style::SEMIBOLD
        } else {
            style::MEDIUM
        })
        .color(color)
        .into()
}

fn climate(state: &State) -> Element<'_, Message> {
    let available = state.available();
    let on = state.is_on();
    let accent = mode_accent(&state.state);
    let mut body = Column::new().spacing(16);

    if let Some(target) = state.number("temperature") {
        let step = state.number("target_temp_step").unwrap_or(1.0);
        let min = state.number("min_temp").unwrap_or(16.0);
        let max = state.number("max_temp").unwrap_or(30.0);
        let number_color = if available && on {
            style::TEXT
        } else {
            style::MUTED
        };
        let caption = match state.number("current_temperature") {
            Some(room) => format!("Room {}°", trim(room)),
            None => "Target".into(),
        };
        let step_color = if available { style::TEXT } else { style::FAINT };
        let set = |temperature: f64| {
            call(
                state,
                "set_temperature",
                json!({"temperature": temperature}),
            )
        };
        let readout = column![
            row![
                text(trim(target))
                    .font(style::LIGHT)
                    .size(52)
                    .color(number_color),
                text("°").font(style::LIGHT).size(28).color(number_color),
            ]
            .align_y(Alignment::Start),
            text(caption).size(12).style(style::muted),
        ]
        .align_x(Alignment::Center);
        body = body.push(
            row![
                round(
                    Icon::Minus,
                    44.0,
                    step_color,
                    style::raised(22.0),
                    if target > min {
                        set((target - step).max(min))
                    } else {
                        None
                    },
                ),
                space::horizontal(),
                readout,
                space::horizontal(),
                round(
                    Icon::Plus,
                    44.0,
                    step_color,
                    style::raised(22.0),
                    if target < max {
                        set((target + step).min(max))
                    } else {
                        None
                    },
                ),
            ]
            .align_y(Alignment::Center),
        );
    }

    let modes: Vec<String> = state
        .list("hvac_modes")
        .into_iter()
        .filter(|mode| mode != "off")
        .collect();
    if !modes.is_empty() {
        let items = modes
            .into_iter()
            .map(|mode| {
                let active = on && state.state == mode;
                let color = match (available, active) {
                    (false, _) => style::FAINT,
                    (true, true) => style::BACKGROUND,
                    (true, false) => style::MUTED,
                };
                let press = call(state, "set_hvac_mode", json!({"hvac_mode": mode}));
                let glyph: Element<'_, Message> = icon(mode_icon(&mode), 18.0, color).into();
                (glyph, active, press)
            })
            .collect();
        body = body.push(segmented(items, accent));
    }

    let fan_modes = state.list("fan_modes");
    if !fan_modes.is_empty() {
        let current = state.text("fan_mode").unwrap_or_default().to_owned();
        let picker: Element<'_, Message> = if fan_modes.len() <= 4 {
            let items = fan_modes
                .into_iter()
                .map(|mode| {
                    let active = mode == current;
                    let label = segment_label(short(&mode), active, available);
                    let press = call(state, "set_fan_mode", json!({"fan_mode": mode}));
                    (label, active, press)
                })
                .collect();
            segmented(items, style::TEAL)
        } else {
            let pills = fan_modes.into_iter().map(|mode| {
                let active = mode == current;
                button(segment_label(label(&mode), active, available))
                    .padding([7, 12])
                    .on_press_maybe(call(state, "set_fan_mode", json!({"fan_mode": mode})))
                    .style(style::choice(active, style::TEAL, 11.0))
                    .into()
            });
            Row::with_children(pills)
                .spacing(6)
                .wrap()
                .vertical_spacing(6)
                .into()
        };
        body = body.push(
            column![
                row![
                    icon(Icon::Wind, 14.0, style::MUTED),
                    text("Fan speed").size(12).style(style::muted),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                picker,
            ]
            .spacing(8),
        );
    }
    body.into()
}

fn fan(state: &State) -> Option<Element<'_, Message>> {
    let count = state.fan_speed_count()?;
    let available = state.available();
    let current = state.fan_speed();
    let off = available && !state.is_on();
    let mut items = vec![(
        segment_label("Off".into(), off, available),
        off,
        call(state, "turn_off", json!({})),
    )];
    for speed in 1..=count {
        let active = current == Some(speed);
        let percentage = entity::percentage_for_speed(speed, count);
        items.push((
            segment_label(speed.to_string(), active, available),
            active,
            call(state, "set_percentage", json!({"percentage": percentage})),
        ));
    }
    Some(segmented(items, style::TEAL))
}

fn mode_name(mode: &str) -> String {
    match mode {
        "heat_cool" => "Heat/Cool".into(),
        "fan_only" => "Fan".into(),
        other => label(other),
    }
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

/// A label short enough for a segment: "medium" → "Med".
fn short(value: &str) -> String {
    match value {
        "medium" | "middle" => "Med".into(),
        other => label(other),
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

/// How many devices are on and offline, for the header.
pub fn summary(sections: &[Section<'_>]) -> String {
    let (mut on, mut offline) = (0, 0);
    for state in sections.iter().flat_map(|section| section.states.iter()) {
        if !state.available() {
            offline += 1;
        } else if state.is_on() {
            on += 1;
        }
    }
    let on = match on {
        0 => "Everything is off".to_owned(),
        1 => "1 device on".to_owned(),
        n => format!("{n} devices on"),
    };
    match offline {
        0 => on,
        n => format!("{on} · {n} offline"),
    }
}
