use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Reverse-DNS id: the Wayland app id, the portal registration and the .desktop file name.
pub const APP_ID: &str = "io.github.interstella0.IotDesktop";
pub const DEFAULT_SERVER: &str = "http://stella-server:8123";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: String,
    /// PulseAudio/PipeWire source name. `None` records from the system default.
    pub mic: Option<String>,
    /// Stream the mic to HA's wake word engine all the time.
    pub wake_word: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: DEFAULT_SERVER.to_owned(),
            mic: None,
            wake_word: false,
        }
    }
}

/// `$XDG_CONFIG_HOME/iot-desktop`.
pub fn dir() -> PathBuf {
    directories::ProjectDirs::from("io.github", "interstella0", "iot-desktop")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".iot-desktop"))
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(dir().join("config.toml"))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("config.toml"), toml::to_string(self)?)?;
        Ok(())
    }
}
