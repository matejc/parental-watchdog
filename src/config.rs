use anyhow::Result;
use serde::{Deserialize, Serialize};
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

pub fn load_config(path: &PathBuf) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&content)?;

    // Validate that at least one pattern is provided
    if config.cmd_pattern.is_none() && config.title_pattern.is_none() {
        anyhow::bail!("At least one of 'cmd_pattern' or 'title_pattern' must be specified in config");
    }

    Ok(config)
}
