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
    /// API-equivalent daily cap for the proxy key, in USD.
    #[serde(default = "default_daily_quota_usd")]
    pub daily_quota_usd: f64,
    /// Time of day (UTC, "HH:MM") at which the proxy resets the daily cap.
    /// When set, the daily figure counts spend since the last reset instead of
    /// a rolling 24h window, which is what actually decides whether the next
    /// request gets a 429. None = rolling 24h (previous behaviour).
    #[serde(default)]
    pub daily_reset_utc: Option<String>,
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
            daily_reset_utc: None,
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
    cfg.daily_reset_utc = normalize_reset(cfg.daily_reset_utc.as_deref());
    cfg
}

/// Accepts "H:MM" / "HH:MM" (UTC). Returns the canonical "HH:MM" or None when
/// empty or unparsable.
pub fn normalize_reset(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (h, m) = raw.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(format!("{h:02}:{m:02}"))
}

/// (last_reset, next_reset) as unix seconds for a "HH:MM" UTC reset time.
pub fn reset_window(reset_utc: Option<&str>, now: i64) -> Option<(i64, i64)> {
    let canon = normalize_reset(reset_utc)?;
    let (h, m) = canon.split_once(':')?;
    let secs_of_day = h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60;
    let day_start = now - now.rem_euclid(86_400);
    let mut last = day_start + secs_of_day;
    if last > now {
        last -= 86_400;
    }
    Some((last, last + 86_400))
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
