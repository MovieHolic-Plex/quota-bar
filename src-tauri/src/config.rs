use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "dev.quotabar.desktop";
/// Credential-store user of the single key older builds kept.
pub const LEGACY_KEY_ID: &str = "api-key";
/// Pseudo id for a key that only exists in this process's environment.
pub const ENV_KEY_ID: &str = "env";

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
    /// Stored keys in priority order. Secrets live in the credential store
    /// under each key's id; only the label and routing live here.
    #[serde(default)]
    pub keys: Vec<KeyMeta>,
    /// Key the bar and Claude settings are pointed at. None = first enabled.
    #[serde(default)]
    pub active_key: Option<String>,
    #[serde(default)]
    pub failover: FailoverConfig,
    /// Claude Code settings.json files this app may point at the active key.
    #[serde(default = "default_targets")]
    pub claude_targets: Vec<ClaudeTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeta {
    pub id: String,
    pub label: String,
    /// Proxy for this key. None = the global base URL.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverConfig {
    /// Move to the next healthy key on its own. Off = the active key only
    /// changes when you pick one.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// All-model used % at which a key counts as exhausted.
    #[serde(default = "default_threshold")]
    pub threshold_pct: f64,
    /// Go back to a higher-priority key once it has headroom again.
    #[serde(default = "yes")]
    pub fail_back: bool,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_pct: default_threshold(),
            fail_back: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeTarget {
    pub id: String,
    /// "local" or "ssh".
    pub kind: String,
    /// user@host (or an ssh_config alias) for kind = "ssh".
    #[serde(default)]
    pub host: Option<String>,
    /// settings.json path. None = ~/.claude/settings.json on that machine.
    #[serde(default)]
    pub path: Option<String>,
    /// Rewrite this file's key and base URL whenever the active key changes.
    #[serde(default)]
    pub sync: bool,
}

fn yes() -> bool {
    true
}

fn default_threshold() -> f64 {
    98.0
}

fn default_targets() -> Vec<ClaudeTarget> {
    vec![ClaudeTarget {
        id: "local".into(),
        kind: "local".into(),
        host: None,
        path: None,
        sync: false,
    }]
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
            keys: vec![],
            active_key: None,
            failover: FailoverConfig::default(),
            claude_targets: default_targets(),
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
    340
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
    if cfg.bar_width > 900 {
        cfg.bar_width = 900;
    }
    // Previous defaults that no longer match the compact layout.
    if cfg.bar_width == 580 || cfg.bar_width == 720 || cfg.bar_width == 460 {
        cfg.bar_width = default_bar_width();
    }
    if cfg.pro_usd <= 0.0 {
        cfg.pro_usd = default_pro_usd();
    }
    if cfg.daily_quota_usd <= 0.0 {
        cfg.daily_quota_usd = default_daily_quota_usd();
    }
    cfg.daily_reset_utc = normalize_reset(cfg.daily_reset_utc.as_deref());
    cfg.failover.threshold_pct = cfg.failover.threshold_pct.clamp(50.0, 100.0);
    // Builds before multi-key kept a single secret under the legacy entry.
    // Adopt it as the first key in place, so nothing is copied or re-stored.
    if cfg.keys.is_empty() && load_secret(LEGACY_KEY_ID).is_some() {
        cfg.keys.push(KeyMeta {
            id: LEGACY_KEY_ID.into(),
            label: "Primary".into(),
            base_url: None,
            enabled: true,
        });
    }
    if !cfg.claude_targets.iter().any(|t| t.kind == "local") {
        cfg.claude_targets.insert(0, default_targets().remove(0));
    }
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

fn keyring_entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, id).map_err(|e| e.to_string())
}

pub fn load_secret(id: &str) -> Option<String> {
    let value = keyring_entry(id).ok()?.get_password().ok()?;
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub fn store_secret(id: &str, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("key is empty".into());
    }
    keyring_entry(id)?
        .set_password(key)
        .map_err(|e| e.to_string())
}

pub fn delete_secret(id: &str) -> Result<(), String> {
    match keyring_entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Read-only fallback when no key is stored. Copying the environment's key
/// into the Windows credential store behind the user's back persists a
/// secret they only meant to expose to this process — Settings is where a
/// key gets saved, and only when it is typed in.
pub fn env_key() -> Option<String> {
    ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

pub fn new_key_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("key-{:x}", nanos)
}

pub fn preview(k: &str) -> String {
    let chars: Vec<char> = k.chars().collect();
    if chars.len() <= 12 {
        "••••".into()
    } else {
        let head: String = chars[..8].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}
