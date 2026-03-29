use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Persisted user configuration.
///
/// Saved to `~/.config/blackshark/config.toml` on every change (debounced).
/// Restored to the headset on every device connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sidetone: u8,
    pub eq_preset: u8,
    pub thx_enabled: bool,
    pub anc_enabled: bool,
    pub anc_level: u8,             // 1–4
    pub power_savings_minutes: u8, // 0=off, 15/30/45/60
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sidetone: 0,
            eq_preset: 0,
            thx_enabled: false,
            anc_enabled: false,
            anc_level: 1,
            power_savings_minutes: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine config directory")?;
    Ok(base.join("blackshark").join("config.toml"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).context("failed to parse config.toml")
}

pub fn save(config: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(config).context("failed to serialise config")?;
    std::fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))
}
