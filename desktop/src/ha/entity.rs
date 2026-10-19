//! Entity states and the registry data used to group them by area.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{Map, Value};

/// Domains that get a card. Scenes, scripts and helpers stay out of the way.
pub const CONTROLLABLE: [&str; 4] = ["climate", "fan", "light", "switch"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct State {
    pub entity_id: String,
    pub state: String,
    #[serde(default)]
    pub attributes: Map<String, Value>,
}

impl State {
    pub fn domain(&self) -> &str {
        self.entity_id.split('.').next().unwrap_or_default()
    }

    pub fn name(&self) -> &str {
        self.attributes
            .get("friendly_name")
            .and_then(Value::as_str)
            .unwrap_or(&self.entity_id)
    }

    /// An entity HA only remembers: its integration no longer provides it (e.g. a removed
    /// template). HA's own UI calls these "not provided".
    pub fn orphaned(&self) -> bool {
        self.attributes.get("restored").and_then(Value::as_bool) == Some(true)
    }

    /// A device the app shows a control for.
    pub fn controllable(&self) -> bool {
        CONTROLLABLE.contains(&self.domain()) && !self.orphaned()
    }

    pub fn available(&self) -> bool {
        !matches!(self.state.as_str(), "unavailable" | "unknown")
    }

    pub fn is_on(&self) -> bool {
        self.available() && self.state != "off"
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(Value::as_f64)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(Value::as_str)
    }

    pub fn list(&self, key: &str) -> Vec<String> {
        self.attributes
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Number of speeds a fan offers, from HA's `percentage_step`. `None` for on/off fans.
    pub fn fan_speed_count(&self) -> Option<u32> {
        let step = self.number("percentage_step")?;
        (step > 0.0 && step < 100.0).then(|| (100.0 / step).round() as u32)
    }

    /// The current fan speed, 1-based; `None` when off or unknown.
    pub fn fan_speed(&self) -> Option<u32> {
        let count = self.fan_speed_count()?;
        let percentage = self.number("percentage")?;
        (self.is_on() && percentage > 0.0).then(|| speed_for_percentage(percentage, count))
    }
}

/// HA's `ranged_value_to_percentage((1, count), speed)`.
pub fn percentage_for_speed(speed: u32, count: u32) -> u32 {
    speed * 100 / count
}

/// HA's `percentage_to_ranged_value((1, count), percentage)`, rounded up like HA's fans.
pub fn speed_for_percentage(percentage: f64, count: u32) -> u32 {
    (percentage * f64::from(count) / 100.0).ceil().max(1.0) as u32
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub areas: HashMap<String, String>,
    entity_areas: HashMap<String, String>,
    hidden: HashSet<String>,
}

#[derive(Deserialize)]
struct Area {
    area_id: String,
    name: String,
}

#[derive(Deserialize)]
struct Device {
    id: String,
    area_id: Option<String>,
}

#[derive(Deserialize)]
struct Entry {
    entity_id: String,
    area_id: Option<String>,
    device_id: Option<String>,
    hidden_by: Option<String>,
    disabled_by: Option<String>,
    entity_category: Option<String>,
}

impl Registry {
    pub fn from_lists(areas: Value, devices: Value, entities: Value) -> Self {
        let areas: Vec<Area> = serde_json::from_value(areas).unwrap_or_default();
        let devices: Vec<Device> = serde_json::from_value(devices).unwrap_or_default();
        let entities: Vec<Entry> = serde_json::from_value(entities).unwrap_or_default();

        let device_areas: HashMap<_, _> = devices
            .into_iter()
            .filter_map(|device| Some((device.id, device.area_id?)))
            .collect();
        let mut registry = Self {
            areas: areas
                .into_iter()
                .map(|area| (area.area_id, area.name))
                .collect(),
            ..Self::default()
        };
        for entry in entities {
            if entry.hidden_by.is_some()
                || entry.disabled_by.is_some()
                || entry.entity_category.is_some()
            {
                registry.hidden.insert(entry.entity_id.clone());
            }
            let area = entry.area_id.or_else(|| {
                entry
                    .device_id
                    .and_then(|id| device_areas.get(&id).cloned())
            });
            if let Some(area) = area {
                registry.entity_areas.insert(entry.entity_id, area);
            }
        }
        registry
    }

    pub fn is_hidden(&self, entity_id: &str) -> bool {
        self.hidden.contains(entity_id)
    }

    pub fn area_name(&self, entity_id: &str) -> Option<&str> {
        let area = self.entity_areas.get(entity_id)?;
        self.areas.get(area).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fan(percentage: f64, step: f64, state: &str) -> State {
        serde_json::from_value(json!({
            "entity_id": "fan.home_fan", "state": state,
            "attributes": {"percentage": percentage, "percentage_step": step}
        }))
        .unwrap()
    }

    #[test]
    fn four_speed_fan_maps_like_home_assistant() {
        let home_fan = fan(75.0, 25.0, "on");
        assert_eq!(home_fan.fan_speed_count(), Some(4));
        assert_eq!(home_fan.fan_speed(), Some(3));
        let percentages: Vec<_> = (1..=4)
            .map(|speed| percentage_for_speed(speed, 4))
            .collect();
        assert_eq!(percentages, [25, 50, 75, 100]);
        for speed in 1..=4 {
            assert_eq!(
                speed_for_percentage(f64::from(percentage_for_speed(speed, 4)), 4),
                speed
            );
        }
    }

    #[test]
    fn three_speed_fan_rounds_up() {
        assert_eq!(fan(33.0, 33.333_333, "on").fan_speed_count(), Some(3));
        assert_eq!(speed_for_percentage(33.0, 3), 1);
        assert_eq!(speed_for_percentage(34.0, 3), 2);
        assert_eq!(speed_for_percentage(100.0, 3), 3);
    }

    #[test]
    fn removed_entities_are_not_controllable() {
        let gone: State = serde_json::from_value(json!({
            "entity_id": "fan.second_house_fan", "state": "unavailable",
            "attributes": {"restored": true, "friendly_name": "Second house fan"}
        }))
        .unwrap();
        assert!(gone.orphaned() && !gone.controllable());
        assert!(fan(50.0, 25.0, "on").controllable());
    }

    #[test]
    fn off_fan_has_no_speed() {
        assert_eq!(fan(0.0, 25.0, "off").fan_speed(), None);
        assert_eq!(fan(50.0, 25.0, "unavailable").fan_speed(), None);
    }

    #[test]
    fn registry_groups_by_entity_then_device_area_and_hides_config_entities() {
        let registry = Registry::from_lists(
            json!([{"area_id": "bedroom", "name": "Bedroom"}, {"area_id": "living", "name": "Living room"}]),
            json!([{"id": "d1", "area_id": "bedroom"}]),
            json!([
                {"entity_id": "climate.bedroom_ac", "area_id": null, "device_id": "d1", "hidden_by": null, "disabled_by": null, "entity_category": null},
                {"entity_id": "fan.home_fan", "area_id": "living", "device_id": "d1", "hidden_by": null, "disabled_by": null, "entity_category": null},
                {"entity_id": "switch.child_lock", "area_id": null, "device_id": "d1", "hidden_by": null, "disabled_by": null, "entity_category": "config"}
            ]),
        );
        assert_eq!(registry.area_name("climate.bedroom_ac"), Some("Bedroom"));
        assert_eq!(registry.area_name("fan.home_fan"), Some("Living room"));
        assert!(registry.is_hidden("switch.child_lock"));
        assert!(!registry.is_hidden("fan.home_fan"));
        assert_eq!(registry.area_name("light.unknown"), None);
    }
}
