use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU32};

use crate::curve::{BrightnessCurve, MonitorOverride};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/Vianpyro/sunrise-brightness";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub update_interval_secs: u64,
    pub start_on_startup: bool,
    pub global_curve: BrightnessCurve,
    pub monitors: Vec<MonitorOverride>,
    pub weather_adaptive: bool,
    pub cloud_attenuation: f64,
    pub dim_inactive: bool,
    pub dim_inactive_factor: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            latitude: None,
            longitude: None,
            update_interval_secs: 180,
            start_on_startup: true,
            global_curve: BrightnessCurve::default(),
            monitors: Vec::new(),
            weather_adaptive: false,
            cloud_attenuation: 0.5,
            dim_inactive: false,
            dim_inactive_factor: 0.3,
        }
    }
}

pub struct SharedState {
    pub config: RwLock<Config>,
    pub needs_refetch: AtomicBool,
    pub status: RwLock<String>,
    pub current_brightness: AtomicU32,
    pub current_elevation: RwLock<f64>,
    pub current_day_progress: RwLock<f64>,
    pub sunrise_str: RwLock<String>,
    pub noon_str: RwLock<String>,
    pub sunset_str: RwLock<String>,
    pub detected_monitors: RwLock<Vec<String>>,
    pub latest_version: RwLock<Option<String>>,
    pub location_str: RwLock<String>,
    pub weather_forecast: RwLock<Vec<(f64, f64)>>,
    pub current_cloud_cover: RwLock<f64>,
    /// Brightness each physical monitor would get if it were active, before the
    /// inactive-dim factor. Written by the sun loop, read when re-pushing.
    pub base_targets: RwLock<HashMap<String, u32>>,
    /// Display holding the foreground window, e.g. `\.\DISPLAY1`.
    pub active_display: RwLock<Option<String>>,
}

impl SharedState {
    pub fn new(config: Config) -> Self {
        Self {
            config: RwLock::new(config),
            needs_refetch: AtomicBool::new(true),
            status: RwLock::new("Starting...".into()),
            current_brightness: AtomicU32::new(50),
            current_elevation: RwLock::new(0.0),
            current_day_progress: RwLock::new(0.0),
            sunrise_str: RwLock::new(String::new()),
            noon_str: RwLock::new(String::new()),
            sunset_str: RwLock::new(String::new()),
            detected_monitors: RwLock::new(Vec::new()),
            latest_version: RwLock::new(None),
            location_str: RwLock::new(String::new()),
            weather_forecast: RwLock::new(Vec::new()),
            current_cloud_cover: RwLock::new(0.0),
            base_targets: RwLock::new(HashMap::new()),
            active_display: RwLock::new(None),
        }
    }

    pub fn set_status(&self, s: &str) {
        if let Ok(mut w) = self.status.write() {
            *w = s.to_string();
        }
    }
}

pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "sunrise-brightness")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("config.toml")
}

pub fn load_config() -> Config {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(config)?)?;
    Ok(())
}
