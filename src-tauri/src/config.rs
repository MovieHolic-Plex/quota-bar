use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "dev.quotabar.desktop";
const KEYRING_USER: &str = "api-key";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_model")]
    pub probe_model: String,
    #[serde(default = "default_bar_width")]
    pub bar_width: u32,
    /// Deprecated. Use pro_usd.
    #[serde(default)]
    pub paid_usd: f64,
    /// Claude Pro monthly USD. Savings = API-equivalent cost minus this.
    #[serde(default = "default_pro_usd")]
    pub pro_usd: f64,
    /// Rolling 24h API-equivalent cap for the Pro proxy key, in USD.
    #[serde(default = "default_daily_quota_usd")]
    pub daily_quota_usd: f64,
    /// Offset from the taskbar's left/top edge. None = auto (left of tray cluster).
    #[serde(default)]
    pub bar_offset_x: Option<i32>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            poll_interval_secs: default_interval(),
            probe_model: default_model(),
            bar_width: default_bar_width(),
            paid_usd: 0.0,
            pro_usd: default_pro_usd(),
            daily_quota_usd: default_daily_quota_usd(),
            bar_offset_x: None,
        }
    }
}

fn default_base_url() -> String {
    std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://claude.nekos.me".into())
}

fn default_interval() -> u64 {
    60
}

fn default_model() -> String {
    "claude-haiku-4-5-20251001".into()
}

fn default_bar_width() -> u32 {
    520
}

fn default_pro_usd() -> f64 {
    20.0
}

fn default_daily_quota_usd() -> f64 {
    6400.0
}

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "quotabar", "quota-bar")
        .map(|p| p.config_dir().to_path_buf())
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.json"))
}

pub fn load_config() -> AppConfig {
    let mut cfg = AppConfig::default();
    if let Some(path) = config_path() {
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(parsed) = serde_json::from_str::<AppConfig>(&raw) {
                cfg = parsed;
            }
        }
    }
    cfg.poll_interval_secs = cfg.poll_interval_secs.max(15);
    cfg.base_url = cfg.base_url.trim_end_matches('/').to_string();
    if cfg.probe_model.trim().is_empty() {
        cfg.probe_model = default_model();
    }
    if cfg.bar_width < 280 {
        cfg.bar_width = 280;
    }
    if cfg.pro_usd <= 0.0 {
        cfg.pro_usd = default_pro_usd();
    }
    if cfg.daily_quota_usd <= 0.0 {
        cfg.daily_quota_usd = default_daily_quota_usd();
    }
    cfg
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    let dir = config_dir().ok_or("could not resolve config directory")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

pub fn load_api_key() -> Option<String> {
    if let Ok(entry) = keyring_entry() {
        if let Ok(value) = entry.get_password() {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    for name in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                let _ = store_api_key(&trimmed);
                return Some(trimmed);
            }
        }
    }
    None
}

pub fn store_api_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(());
    }
    keyring_entry()?
        .set_password(key)
        .map_err(|e| e.to_string())
}

pub fn has_api_key() -> bool {
    load_api_key().is_some()
}

pub fn key_preview() -> Option<String> {
    load_api_key().map(|k| {
        if k.len() <= 8 {
            "••••".into()
        } else {
            format!("{}…{}", &k[..6], &k[k.len() - 4..])
        }
    })
}
