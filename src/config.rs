use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// Username that owns the graphical session (mandatory)
    pub user: String,

    /// Hard time‑limit in seconds (default 7200 ≈ 2 h)
    #[serde(default = "default_limit")]
    pub limit: i64,

    /// Seconds before the limit when a warning is shown (default 900 ≈ 15 min)
    #[serde(default = "default_warn_before")]
    pub warn_before: i64,

    /// Interval between scans, in seconds
    #[serde(default = "default_interval")]
    pub interval: u64,

    /// Regex that must match the command name
    pub cmd_pattern: Option<String>,

    /// Regex that must match the window title
    pub title_pattern: Option<String>,

    /// Which backend to use: "kdotool", "niri" or "xdotool"
    #[serde(default = "default_backend")]
    pub backend: String,

    #[serde(default = "default_backend_path")]
    pub backend_path: String,

    /// Begin time for the day (outside of the begin and end time, windows with patterns will be terminated immediately)
    #[serde(default = "default_time_begin")]
    pub time_begin: String,

    /// End time for the day
    #[serde(default = "default_time_end")]
    pub time_end: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct ConfigOverride {
    pub user: Option<String>,
    pub limit: Option<i64>,
    pub warn_before: Option<i64>,
    pub interval: Option<u64>,
    pub cmd_pattern: Option<String>,
    pub title_pattern: Option<String>,
    pub backend: Option<String>,
    pub backend_path: Option<String>,
    pub time_begin: Option<String>,
    pub time_end: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ScheduledConfig {
    default: Config,
    #[serde(default)]
    days: HashMap<String, ConfigOverride>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum ConfigFile {
    Flat(Config),
    Scheduled(ScheduledConfig),
}

fn default_limit() -> i64 {
    7200
}

fn default_warn_before() -> i64 {
    900
}

fn default_interval() -> u64 {
    10
}

fn default_backend() -> String {
    "kdotool".to_string()
}

fn default_backend_path() -> String {
    "".to_string()
}

fn default_time_begin() -> String {
    "12:00".to_string()
}

fn default_time_end() -> String {
    "21:00".to_string()
}

impl Config {
    fn apply_override(&mut self, config_override: ConfigOverride) {
        if let Some(user) = config_override.user {
            self.user = user;
        }
        if let Some(limit) = config_override.limit {
            self.limit = limit;
        }
        if let Some(warn_before) = config_override.warn_before {
            self.warn_before = warn_before;
        }
        if let Some(interval) = config_override.interval {
            self.interval = interval;
        }
        if let Some(cmd_pattern) = config_override.cmd_pattern {
            self.cmd_pattern = Some(cmd_pattern);
        }
        if let Some(title_pattern) = config_override.title_pattern {
            self.title_pattern = Some(title_pattern);
        }
        if let Some(backend) = config_override.backend {
            self.backend = backend;
        }
        if let Some(backend_path) = config_override.backend_path {
            self.backend_path = backend_path;
        }
        if let Some(time_begin) = config_override.time_begin {
            self.time_begin = time_begin;
        }
        if let Some(time_end) = config_override.time_end {
            self.time_end = time_end;
        }
    }
}

pub fn load_config(path: &PathBuf) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config = match serde_yaml::from_str::<ConfigFile>(&content)? {
        ConfigFile::Flat(config) => config,
        ConfigFile::Scheduled(mut scheduled) => {
            let today = chrono::Local::now().format("%A").to_string();
            if let Some(day_override) = scheduled.days.remove(&today) {
                scheduled.default.apply_override(day_override);
            }
            scheduled.default
        }
    };

    // Validate that at least one pattern is provided
    if config.cmd_pattern.is_none() && config.title_pattern.is_none() {
        anyhow::bail!(
            "At least one of 'cmd_pattern' or 'title_pattern' must be specified in config"
        );
    }

    Ok(config)
}
