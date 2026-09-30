mod claude_cfg;
mod config;
mod db;
mod keys;
mod quota;
mod taskbar;

use claude_cfg::{ClaudeEnv, ClaudePatch, TargetInfo};
use config::{
    delete_secret, env_key, load_config, load_secret, new_key_id, normalize_reset, preview,
    reset_window, save_config, store_secret, AppConfig, ClaudeTarget, FailoverConfig, KeyMeta,
    ENV_KEY_ID,
};
use db::{BucketRow, UsageStats};
use keys::{Health, KeyStatus};
use quota::{fetch_usage, named_limits, NamedLimits, QuotaSnapshot};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tokio::sync::Notify;

/// Longest this thread sleeps between wake-ups. Only bounds how quickly a
/// changed poll interval or a lost app state is noticed; docking is not on
/// this thread.
const IDLE_BACKOFF: Duration = Duration::from_secs(10);
const PRUNE_EVERY: Duration = Duration::from_secs(6 * 3600);
const RETAIN_SECS: i64 = 90 * 86_400;

struct AppState {
    config: Mutex<AppConfig>,
    /// The live key's last read — what the bar draws.
    quota: Mutex<QuotaSnapshot>,
    /// Last read of every key, live or not.
    keys: Mutex<HashMap<String, KeyStatus>>,
    /// Which stored key each Claude target was last seen using (None = a key
    /// this app does not hold, or none at all).
    claude_use: Mutex<HashMap<String, Option<String>>>,
    last_switch: Mutex<Option<SwitchEvent>>,
    db: Mutex<Connection>,
    refresh: Notify,
}

/// A panic anywhere behind these locks used to poison them for the rest of the
/// session, which silently killed the poll loop and froze the bar on stale
/// numbers. Recovering the inner value is safe here: every one of them is
/// replaced wholesale rather than mutated in place.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Serialize)]
struct SwitchEvent {
    from: Option<String>,
    to: String,
    from_label: Option<String>,
    to_label: String,
    reason: String,
    at: u64,
    /// Claude targets rewritten to the new key, and the ones that failed.
    synced: Vec<String>,
    sync_errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SettingsPatch {
    base_url: String,
    poll_interval_secs: u64,
    pro_usd: f64,
    #[serde(default)]
    daily_quota_usd: Option<f64>,
    #[serde(default)]
    bar_width: Option<u32>,
    #[serde(default)]
    daily_reset_utc: Option<String>,
}

#[derive(Debug, Serialize)]
struct SettingsView {
    base_url: String,
    poll_interval_secs: u64,
    pro_usd: f64,
    daily_quota_usd: f64,
    bar_width: u32,
    daily_reset_utc: Option<String>,
    has_key: bool,
}

/// The live key, as the bar and stats window name it.
#[derive(Debug, Clone, Serialize)]
struct KeyBrief {
    id: String,
    label: String,
    rank: usize,
    total: usize,
}

#[derive(Debug, Clone, Serialize)]
struct BarView {
    #[serde(flatten)]
    snap: QuotaSnapshot,
    #[serde(flatten)]
    api: NamedLimits,
    minutes: Vec<BucketRow>,
    spend_10m: f64,
    spend_1h: f64,
    spend_1d: f64,
    daily_quota_usd: f64,
    /// Share of the daily cap used. Anchored to the reset time when one is
    /// configured (spend_since_reset / cap), otherwise rolling 24h.
    daily_pct: f64,
    /// Spend since the key's last daily reset. None = no reset time configured.
    spend_since_reset: Option<f64>,
    /// Seconds until the next daily reset. None = no reset time configured.
    reset_in_secs: Option<i64>,
    daily_reset_utc: Option<String>,
    key: Option<KeyBrief>,
    /// Keys that could take over right now, not counting the live one.
    spare_keys: usize,
    last_switch: Option<SwitchEvent>,
}

/// Keys the app can use, in priority order. With nothing stored, a key from
/// the environment stands in as a single read-only entry.
fn key_list(cfg: &AppConfig) -> Vec<KeyMeta> {
    if !cfg.keys.is_empty() {
        return cfg.keys.clone();
    }
    match env_key() {
        Some(_) => vec![KeyMeta {
            id: ENV_KEY_ID.into(),
            label: "Environment".into(),
            base_url: None,
            enabled: true,
        }],
        None => vec![],
    }
}

fn secret_for(id: &str) -> Option<String> {
    if id == ENV_KEY_ID {
        env_key()
    } else {
        load_secret(id)
    }
}

fn effective_base(cfg: &AppConfig, meta: &KeyMeta) -> String {
    meta.base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&cfg.base_url)
        .trim_end_matches('/')
        .to_string()
}

fn active_id(cfg: &AppConfig) -> Option<String> {
    let list = key_list(cfg);
    cfg.active_key
        .as_ref()
        .filter(|id| list.iter().any(|k| &k.id == *id && k.enabled))
        .cloned()
        .or_else(|| list.iter().find(|k| k.enabled).map(|k| k.id.clone()))
}

fn key_brief(cfg: &AppConfig, id: &str) -> Option<KeyBrief> {
    let list = key_list(cfg);
    let enabled: Vec<&KeyMeta> = list.iter().filter(|k| k.enabled).collect();
    let rank = enabled.iter().position(|k| k.id == id)?;
    Some(KeyBrief {
        id: id.into(),
        label: enabled[rank].label.clone(),
        rank: rank + 1,
        total: enabled.len(),
    })
}

fn apply_pro(snap: &mut QuotaSnapshot, pro_usd: f64) {
    snap.pro_usd = pro_usd;
    snap.paid_usd = pro_usd;
    snap.savings_usd = snap.total_cost_usd - pro_usd;
    snap.cache_pct = if snap.total_tokens > 0 {
        snap.cached_input_tokens as f64 / snap.total_tokens as f64 * 100.0
    } else {
        0.0
    };
}

fn bar_view(state: &AppState, snap: QuotaSnapshot) -> BarView {
    let (daily_quota_usd, daily_reset_utc, key, threshold, enabled) = {
        let cfg = lock(&state.config);
        let key = active_id(&cfg).and_then(|id| key_brief(&cfg, &id));
        let enabled: Vec<String> = key_list(&cfg)
            .into_iter()
            .filter(|k| k.enabled)
            .map(|k| k.id)
            .collect();
        (
            cfg.daily_quota_usd.max(1.0),
            cfg.daily_reset_utc.clone(),
            key,
            cfg.failover.threshold_pct,
            enabled,
        )
    };
    let spare_keys = {
        let statuses = lock(&state.keys);
        enabled
            .iter()
            .filter(|id| key.as_ref().map(|k| &k.id != *id).unwrap_or(true))
            .filter(|id| matches!(keys::health(statuses.get(*id), threshold), Health::Healthy { .. }))
            .count()
    };
    let key_id = key.as_ref().map(|k| k.id.clone()).unwrap_or_default();
    let now = quota::now_unix() as i64;
    let reset = reset_window(daily_reset_utc.as_deref(), now);
    let db = lock(&state.db);
    let minutes = db::minute_series(&db, &key_id, 30).unwrap_or_default();
    let (spend_10m, spend_1h, spend_1d) = db::recent_spend(&db, &key_id).unwrap_or((0.0, 0.0, 0.0));
    let spend_since_reset = reset.map(|(last, _)| db::spend_since(&db, &key_id, last).unwrap_or(0.0));
    drop(db);
    let daily_pct = (spend_since_reset.unwrap_or(spend_1d) / daily_quota_usd) * 100.0;
    let api = named_limits(&snap.limits);
    BarView {
        snap,
        api,
        minutes,
        spend_10m,
        spend_1h,
        spend_1d,
        daily_quota_usd,
        daily_pct,
        spend_since_reset,
        reset_in_secs: reset.map(|(_, next)| (next - now).max(0)),
        daily_reset_utc,
        key,
        spare_keys,
        last_switch: lock(&state.last_switch).clone(),
    }
}

fn emit_quota(app: &AppHandle, view: &BarView) {
    let _ = app.emit("quota-update", view);
}

/// Read one key. A dropped request keeps the previous figures (marked stale);
/// a refusal is recorded so failover can act on it.
async fn read_key(base: String, secret: Option<String>, prev: KeyStatus, pro: f64) -> KeyStatus {
    let Some(secret) = secret else {
        return KeyStatus {
            snap: QuotaSnapshot {
                error: Some("secret missing from Credential Manager".into()),
                ..Default::default()
            },
            refused: None,
        };
    };
    match fetch_usage(&base, &secret).await {
        Ok(mut snap) => {
            apply_pro(&mut snap, pro);
            KeyStatus { snap, refused: None }
        }
        Err(err) => {
            let mut snap = prev.snap;
            snap.stale = snap.fetched_at.is_some();
            snap.error = Some(err.message.clone());
            KeyStatus {
                snap,
                refused: if err.is_refusal() { err.status } else { None },
            }
        }
    }
}

async fn poll_once(app: &AppHandle, state: &AppState) -> QuotaSnapshot {
    let (list, bases, pro, failover, current) = {
        let cfg = lock(&state.config);
        let list = key_list(&cfg);
        let bases: Vec<String> = list.iter().map(|k| effective_base(&cfg, k)).collect();
        (list, bases, cfg.pro_usd, cfg.failover.clone(), active_id(&cfg))
    };

    if !list.iter().any(|k| k.enabled) {
        let snap = QuotaSnapshot {
            error: Some("no api key".into()),
            ..Default::default()
        };
        *lock(&state.quota) = snap.clone();
        emit_quota(app, &bar_view(state, snap.clone()));
        return snap;
    }

    // Every enabled key, concurrently: one GET each, and failover then acts
    // on fresh limits instead of waiting for the live key to be refused.
    let mut jobs = Vec::new();
    for (meta, base) in list.iter().zip(bases) {
        if !meta.enabled {
            continue;
        }
        let prev = lock(&state.keys).get(&meta.id).cloned().unwrap_or_default();
        let secret = secret_for(&meta.id);
        let id = meta.id.clone();
        jobs.push((
            id,
            tauri::async_runtime::spawn(read_key(base, secret, prev, pro)),
        ));
    }
    for (id, job) in jobs {
        let Ok(mut status) = job.await else { continue };
        if status.snap.error.is_none() {
            let wrote = db::insert_snapshot(&lock(&state.db), &id, &status.snap);
            if let Err(err) = wrote {
                status.snap.error = Some(format!("db: {err}"));
            }
        }
        lock(&state.keys).insert(id, status);
    }

    let chosen = {
        let statuses = lock(&state.keys);
        let order: Vec<(String, Health)> = list
            .iter()
            .filter(|k| k.enabled)
            .map(|k| (k.id.clone(), keys::health(statuses.get(&k.id), failover.threshold_pct)))
            .collect();
        if failover.enabled {
            keys::choose(&order, current.as_deref(), failover.threshold_pct, failover.fail_back)
        } else {
            (current.clone(), None)
        }
    };

    if let (Some(next), Some(reason)) = (&chosen.0, &chosen.1) {
        if Some(next) != current.as_ref() {
            switch_key(app, state, current.clone(), next.clone(), reason.clone()).await;
        }
    } else if chosen.0.is_some() {
        // First pick, or the stored choice went away: remember it quietly.
        let mut cfg = lock(&state.config);
        if cfg.active_key != chosen.0 {
            cfg.active_key = chosen.0.clone();
            let _ = save_config(&cfg);
        }
    }

    let live = chosen.0.unwrap_or_default();
    let snap = lock(&state.keys)
        .get(&live)
        .map(|s| s.snap.clone())
        .unwrap_or_default();
    *lock(&state.quota) = snap.clone();
    emit_quota(app, &bar_view(state, snap.clone()));
    snap
}

/// Make `to` the live key, then point every synced Claude target at it.
async fn switch_key(app: &AppHandle, state: &AppState, from: Option<String>, to: String, reason: String) {
    let (from_label, to_label, targets, base) = {
        let mut cfg = lock(&state.config);
        cfg.active_key = Some(to.clone());
        let _ = save_config(&cfg);
        let list = key_list(&cfg);
        let label_of = |id: &str| list.iter().find(|k| k.id == id).map(|k| k.label.clone());
        let base = list
            .iter()
            .find(|k| k.id == to)
            .map(|k| effective_base(&cfg, k))
            .unwrap_or_else(|| cfg.base_url.clone());
        let targets: Vec<ClaudeTarget> = cfg.claude_targets.iter().filter(|t| t.sync).cloned().collect();
        (
            from.as_deref().and_then(label_of),
            label_of(&to).unwrap_or_else(|| to.clone()),
            targets,
            base,
        )
    };
    let (synced, sync_errors) = sync_targets(state, targets, &to, base).await;
    let event = SwitchEvent {
        from,
        to,
        from_label,
        to_label,
        reason,
        at: quota::now_unix(),
        synced,
        sync_errors,
    };
    *lock(&state.last_switch) = Some(event.clone());
    let _ = app.emit("key-switched", &event);
}

/// Point each target at `key_id` (key + base URL only). Returns the labels
/// that were written and the errors of those that were not.
async fn sync_targets(
    state: &AppState,
    targets: Vec<ClaudeTarget>,
    key_id: &str,
    base: String,
) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    let Some(secret) = secret_for(key_id) else {
        return (ok, vec!["secret missing from Credential Manager".into()]);
    };
    let jobs: Vec<_> = targets
        .into_iter()
        .map(|t| {
            let patch = ClaudePatch {
                base_url: Some(base.clone()),
                secret: Some(secret.clone()),
                ..Default::default()
            };
            tauri::async_runtime::spawn_blocking(move || {
                let res = claude_cfg::patch(&t, &patch);
                (t, res)
            })
        })
        .collect();
    for job in jobs {
        let Ok((t, res)) = job.await else { continue };
        match res {
            Ok(_) => {
                lock(&state.claude_use).insert(t.id.clone(), Some(key_id.to_string()));
                ok.push(claude_cfg::label(&t));
            }
            Err(e) => failed.push(format!("{}: {e}", claude_cfg::label(&t))),
        }
    }
    (ok, failed)
}

/// UI-thread only: setup, a tray menu event, or a command handler. Never call
/// this from the poll loop — see rule 1 in `taskbar.rs`.
fn redock(app: &AppHandle) {
    if taskbar::is_dragging() {
        return;
    }
    let (width, offset) = app
        .try_state::<AppState>()
        .map(|s| {
            let cfg = lock(&s.config);
            (cfg.bar_width, cfg.bar_offset_x)
        })
        .unwrap_or((420, None));
    if let Some(bar) = app.get_webview_window("bar") {
        let _ = taskbar::dock_bar(&bar, width, offset);
    }
}

/// Async so it lands on the runtime rather than the UI thread: it touches
/// SQLite, and a synchronous command stalls the message pump of a topmost
/// window that the shell is waiting on.
#[tauri::command]
async fn current_quota(state: State<'_, AppState>) -> Result<BarView, String> {
    let snap = lock(&state.quota).clone();
    Ok(bar_view(&state, snap))
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsView {
    let cfg = lock(&state.config).clone();
    let has_key = !key_list(&cfg).is_empty();
    SettingsView {
        base_url: cfg.base_url,
        poll_interval_secs: cfg.poll_interval_secs,
        pro_usd: cfg.pro_usd,
        daily_quota_usd: cfg.daily_quota_usd,
        bar_width: cfg.bar_width,
        daily_reset_utc: cfg.daily_reset_utc,
        has_key,
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: SettingsPatch) -> Result<(), String> {
    {
        let mut cfg = lock(&state.config);
        cfg.base_url = settings.base_url.trim_end_matches('/').to_string();
        cfg.poll_interval_secs = settings.poll_interval_secs.max(15);
        cfg.pro_usd = if settings.pro_usd > 0.0 { settings.pro_usd } else { 20.0 };
        if let Some(q) = settings.daily_quota_usd {
            if q > 0.0 {
                cfg.daily_quota_usd = q;
            }
        }
        if let Some(w) = settings.bar_width {
            cfg.bar_width = w.clamp(280, 900);
        }
        if let Some(raw) = settings.daily_reset_utc.as_deref() {
            if !raw.trim().is_empty() && normalize_reset(Some(raw)).is_none() {
                return Err("Daily reset time must be HH:MM (UTC), e.g. 06:34".into());
            }
        }
        cfg.daily_reset_utc = normalize_reset(settings.daily_reset_utc.as_deref());
        save_config(&cfg)?;
    }
    redock(&app);
    state.refresh.notify_one();
    Ok(())
}

// ── keys ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct KeyView {
    id: String,
    label: String,
    base_url: Option<String>,
    effective_base_url: String,
    enabled: bool,
    /// Lives in the environment only; cannot be edited or removed here.
    env: bool,
    preview: Option<String>,
    active: bool,
    health: &'static str,
    health_reason: Option<String>,
    peak_pct: Option<f64>,
    status: Option<KeyStatus>,
    #[serde(flatten)]
    api: NamedLimits,
    /// Claude targets last seen using this key.
    used_in: Vec<String>,
}

#[derive(Debug, Serialize)]
struct KeysView {
    keys: Vec<KeyView>,
    failover: FailoverConfig,
    active: Option<String>,
    last_switch: Option<SwitchEvent>,
}

#[tauri::command]
async fn list_keys(state: State<'_, AppState>) -> Result<KeysView, String> {
    let cfg = lock(&state.config).clone();
    let active = active_id(&cfg);
    let statuses = lock(&state.keys).clone();
    let uses = lock(&state.claude_use).clone();
    let labels: HashMap<String, String> = cfg
        .claude_targets
        .iter()
        .map(|t| (t.id.clone(), claude_cfg::label(t)))
        .collect();
    let keys = key_list(&cfg)
        .into_iter()
        .map(|k| {
            let status = statuses.get(&k.id).cloned();
            let health = if k.enabled {
                keys::health(status.as_ref(), cfg.failover.threshold_pct)
            } else {
                Health::Unknown
            };
            let used_in = uses
                .iter()
                .filter(|(_, used)| used.as_deref() == Some(k.id.as_str()))
                .filter_map(|(tid, _)| labels.get(tid).cloned())
                .collect();
            KeyView {
                effective_base_url: effective_base(&cfg, &k),
                preview: secret_for(&k.id).map(|s| preview(&s)),
                active: active.as_deref() == Some(k.id.as_str()),
                health: if k.enabled { health.word() } else { "disabled" },
                health_reason: match &health {
                    Health::Exhausted(why) => Some(why.clone()),
                    _ => status.as_ref().and_then(|s| s.snap.error.clone()),
                },
                peak_pct: status
                    .as_ref()
                    .and_then(|s| quota::peak_all_model_pct(&s.snap.limits)),
                api: status
                    .as_ref()
                    .map(|s| named_limits(&s.snap.limits))
                    .unwrap_or_default(),
                status,
                used_in,
                env: k.id == ENV_KEY_ID,
                id: k.id,
                label: k.label,
                base_url: k.base_url,
                enabled: k.enabled,
            }
        })
        .collect();
    Ok(KeysView {
        keys,
        failover: cfg.failover,
        active,
        last_switch: lock(&state.last_switch).clone(),
    })
}

/// The full secret, for the reveal and copy buttons.
#[tauri::command]
fn reveal_key(id: String) -> Result<String, String> {
    secret_for(&id).ok_or_else(|| "no secret stored for this key".into())
}

fn clean_url(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

fn ensure_unique(cfg: &AppConfig, secret: &str, except: Option<&str>) -> Result<(), String> {
    for k in &cfg.keys {
        if Some(k.id.as_str()) == except {
            continue;
        }
        if load_secret(&k.id).as_deref() == Some(secret.trim()) {
            return Err(format!("That key is already stored as \"{}\"", k.label));
        }
    }
    Ok(())
}

#[tauri::command]
fn add_key(
    state: State<AppState>,
    label: String,
    secret: String,
    base_url: Option<String>,
) -> Result<String, String> {
    let secret = secret.trim().to_string();
    if secret.is_empty() {
        return Err("Paste a key first".into());
    }
    let mut cfg = lock(&state.config);
    ensure_unique(&cfg, &secret, None)?;
    let id = new_key_id();
    store_secret(&id, &secret)?;
    let label = match label.trim() {
        "" => format!("Key {}", cfg.keys.len() + 1),
        l => l.to_string(),
    };
    cfg.keys.push(KeyMeta {
        id: id.clone(),
        label,
        base_url: clean_url(base_url),
        enabled: true,
    });
    save_config(&cfg)?;
    drop(cfg);
    state.refresh.notify_one();
    Ok(id)
}

#[tauri::command]
fn update_key(
    state: State<AppState>,
    id: String,
    label: Option<String>,
    base_url: Option<String>,
    secret: Option<String>,
    enabled: Option<bool>,
) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    if let Some(secret) = secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        ensure_unique(&cfg, secret, Some(&id))?;
    }
    let meta = cfg
        .keys
        .iter_mut()
        .find(|k| k.id == id)
        .ok_or("unknown key")?;
    if let Some(label) = label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty()) {
        meta.label = label;
    }
    if base_url.is_some() {
        meta.base_url = clean_url(base_url);
    }
    if let Some(on) = enabled {
        meta.enabled = on;
    }
    if let Some(secret) = secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        store_secret(&id, secret)?;
        lock(&state.keys).remove(&id);
    }
    save_config(&cfg)?;
    drop(cfg);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn remove_key(state: State<AppState>, id: String) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    let before = cfg.keys.len();
    cfg.keys.retain(|k| k.id != id);
    if cfg.keys.len() == before {
        return Err("unknown key".into());
    }
    if cfg.active_key.as_deref() == Some(id.as_str()) {
        cfg.active_key = None;
    }
    save_config(&cfg)?;
    drop(cfg);
    delete_secret(&id)?;
    lock(&state.keys).remove(&id);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn move_key(state: State<AppState>, id: String, delta: i32) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    let from = cfg.keys.iter().position(|k| k.id == id).ok_or("unknown key")?;
    let to = (from as i32 + delta).clamp(0, cfg.keys.len() as i32 - 1) as usize;
    let item = cfg.keys.remove(from);
    cfg.keys.insert(to, item);
    save_config(&cfg)?;
    drop(cfg);
    state.refresh.notify_one();
    Ok(())
}

/// Pick the live key by hand. With failover on, the pick also moves to the
/// top of the order — otherwise fail-back would undo it on the next poll.
#[tauri::command]
async fn set_active_key(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let current = {
        let mut cfg = lock(&state.config);
        let list = key_list(&cfg);
        if !list.iter().any(|k| k.id == id && k.enabled) {
            return Err("that key is disabled or gone".into());
        }
        if cfg.failover.enabled {
            if let Some(pos) = cfg.keys.iter().position(|k| k.id == id) {
                let item = cfg.keys.remove(pos);
                cfg.keys.insert(0, item);
                save_config(&cfg)?;
            }
        }
        active_id(&cfg)
    };
    if current.as_deref() != Some(id.as_str()) {
        switch_key(&app, &state, current, id, "picked in Settings".into()).await;
    }
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn set_failover(state: State<AppState>, failover: FailoverConfig) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    cfg.failover = FailoverConfig {
        threshold_pct: failover.threshold_pct.clamp(50.0, 100.0),
        ..failover
    };
    save_config(&cfg)?;
    drop(cfg);
    state.refresh.notify_one();
    Ok(())
}

#[derive(Debug, Serialize)]
struct KeyProbe {
    ok: bool,
    error: Option<String>,
    #[serde(flatten)]
    api: NamedLimits,
    total_cost_usd: f64,
}

/// Read a key once without storing anything — the Test button.
#[tauri::command]
async fn test_key(
    state: State<'_, AppState>,
    secret: Option<String>,
    id: Option<String>,
    base_url: Option<String>,
) -> Result<KeyProbe, String> {
    let secret = match (secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()), id) {
        (Some(s), _) => s,
        (None, Some(id)) => secret_for(&id).ok_or("no secret stored for this key")?,
        (None, None) => return Err("Paste a key first".into()),
    };
    let base = clean_url(base_url).unwrap_or_else(|| lock(&state.config).base_url.clone());
    Ok(match fetch_usage(&base, &secret).await {
        Ok(snap) => KeyProbe {
            ok: true,
            error: None,
            api: named_limits(&snap.limits),
            total_cost_usd: snap.total_cost_usd,
        },
        Err(e) => KeyProbe {
            ok: false,
            error: Some(e.message),
            api: NamedLimits::default(),
            total_cost_usd: 0.0,
        },
    })
}

// ── Claude Code settings ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TargetView {
    #[serde(flatten)]
    info: TargetInfo,
    ok: bool,
    error: Option<String>,
    exists: bool,
    base_url: Option<String>,
    /// Stored key this file uses; None with `key_preview` set = a key this
    /// app does not hold.
    key_id: Option<String>,
    key_preview: Option<String>,
    /// The file sets ANTHROPIC_API_KEY and ANTHROPIC_AUTH_TOKEN to different values.
    key_mismatch: bool,
    models: Vec<(String, Option<String>)>,
    model: Option<String>,
}

fn match_key(cfg: &AppConfig, secret: Option<&str>) -> Option<String> {
    let secret = secret?.trim();
    key_list(cfg)
        .into_iter()
        .find(|k| secret_for(&k.id).as_deref() == Some(secret))
        .map(|k| k.id)
}

fn target_view(cfg: &AppConfig, t: &ClaudeTarget, read: Result<ClaudeEnv, String>) -> TargetView {
    let info = claude_cfg::describe(t);
    match read {
        Ok(env) => TargetView {
            info,
            ok: true,
            error: None,
            exists: env.exists,
            base_url: env.base_url.clone(),
            key_id: match_key(cfg, env.secret()),
            key_preview: env.secret().map(preview),
            key_mismatch: matches!((&env.api_key, &env.auth_token), (Some(a), Some(b)) if a != b),
            models: env.models,
            model: env.model,
        },
        Err(e) => TargetView {
            info,
            ok: false,
            error: Some(e),
            exists: false,
            base_url: None,
            key_id: None,
            key_preview: None,
            key_mismatch: false,
            models: claude_cfg::MODEL_VARS.iter().map(|m| (m.to_string(), None)).collect(),
            model: None,
        },
    }
}

fn find_target(state: &AppState, id: &str) -> Result<ClaudeTarget, String> {
    lock(&state.config)
        .claude_targets
        .iter()
        .find(|t| t.id == id)
        .cloned()
        .ok_or_else(|| "unknown target".into())
}

/// Read every target in parallel. Async and on the blocking pool: an ssh
/// read can take seconds and must not sit on the UI thread.
#[tauri::command]
async fn list_claude_targets(state: State<'_, AppState>) -> Result<Vec<TargetView>, String> {
    let targets = lock(&state.config).claude_targets.clone();
    let jobs: Vec<_> = targets
        .into_iter()
        .map(|t| {
            tauri::async_runtime::spawn_blocking(move || {
                let res = claude_cfg::read(&t);
                (t, res)
            })
        })
        .collect();
    let mut out = Vec::new();
    for job in jobs {
        let (t, res) = job.await.map_err(|e| e.to_string())?;
        let cfg = lock(&state.config).clone();
        let view = target_view(&cfg, &t, res);
        if view.ok {
            lock(&state.claude_use).insert(t.id.clone(), view.key_id.clone());
        }
        out.push(view);
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct ClaudeApply {
    target_id: String,
    /// Stored key to write. None leaves the key alone.
    #[serde(default)]
    key_id: Option<String>,
    #[serde(flatten)]
    patch: ClaudePatch,
}

#[tauri::command]
async fn apply_claude(state: State<'_, AppState>, apply: ClaudeApply) -> Result<TargetView, String> {
    let target = find_target(&state, &apply.target_id)?;
    let mut patch = apply.patch;
    if let Some(key_id) = &apply.key_id {
        patch.secret = Some(secret_for(key_id).ok_or("no secret stored for that key")?);
    }
    let t = target.clone();
    let res = tauri::async_runtime::spawn_blocking(move || claude_cfg::patch(&t, &patch))
        .await
        .map_err(|e| e.to_string())?;
    let res = res?;
    let cfg = lock(&state.config).clone();
    let view = target_view(&cfg, &target, Ok(res));
    lock(&state.claude_use).insert(target.id.clone(), view.key_id.clone());
    Ok(view)
}

#[tauri::command]
fn add_claude_target(state: State<AppState>, host: String, path: Option<String>) -> Result<String, String> {
    let host = host.trim().to_string();
    claude_cfg::validate_host(&host)?;
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        claude_cfg::validate_remote_path(p)?;
    }
    let mut cfg = lock(&state.config);
    if cfg.claude_targets.iter().any(|t| t.host.as_deref() == Some(host.as_str()) && t.path == path) {
        return Err(format!("{host} is already listed"));
    }
    let id = format!("ssh-{}", new_key_id().trim_start_matches("key-"));
    cfg.claude_targets.push(ClaudeTarget {
        id: id.clone(),
        kind: "ssh".into(),
        host: Some(host),
        path,
        sync: false,
    });
    save_config(&cfg)?;
    Ok(id)
}

#[tauri::command]
fn remove_claude_target(state: State<AppState>, id: String) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    if cfg.claude_targets.iter().any(|t| t.id == id && t.kind == "local") {
        return Err("This PC cannot be removed; turn its sync off instead".into());
    }
    cfg.claude_targets.retain(|t| t.id != id);
    save_config(&cfg)?;
    drop(cfg);
    lock(&state.claude_use).remove(&id);
    Ok(())
}

#[tauri::command]
fn set_target_sync(state: State<AppState>, id: String, sync: bool) -> Result<(), String> {
    let mut cfg = lock(&state.config);
    let t = cfg
        .claude_targets
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or("unknown target")?;
    t.sync = sync;
    save_config(&cfg)
}

/// Point every synced target at the live key now, without waiting for a switch.
#[tauri::command]
async fn sync_claude_now(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (targets, key, base) = {
        let cfg = lock(&state.config);
        let key = active_id(&cfg).ok_or("no live key")?;
        let base = key_list(&cfg)
            .iter()
            .find(|k| k.id == key)
            .map(|k| effective_base(&cfg, k))
            .unwrap_or_else(|| cfg.base_url.clone());
        let targets: Vec<ClaudeTarget> = cfg.claude_targets.iter().filter(|t| t.sync).cloned().collect();
        (targets, key, base)
    };
    if targets.is_empty() {
        return Err("No target has sync turned on".into());
    }
    let (ok, failed) = sync_targets(&state, targets, &key, base).await;
    if failed.is_empty() {
        Ok(ok)
    } else {
        Err(failed.join(" · "))
    }
}

#[tauri::command]
fn refresh_now(state: State<AppState>) {
    state.refresh.notify_one();
}

#[tauri::command]
fn nudge_bar_width(app: AppHandle, state: State<AppState>, dw: i32) -> Result<u32, String> {
    let (width, offset) = {
        let mut cfg = lock(&state.config);
        let next = (cfg.bar_width as i32 + dw).clamp(280, 800) as u32;
        cfg.bar_width = next;
        save_config(&cfg)?;
        (next, cfg.bar_offset_x)
    };
    if let Some(bar) = app.get_webview_window("bar") {
        let _ = taskbar::dock_bar(&bar, width, offset);
    }
    Ok(width)
}

#[tauri::command]
fn begin_bar_drag() {
    taskbar::set_dragging(true);
}

#[tauri::command]
fn nudge_bar(app: AppHandle, state: State<AppState>, dx: i32) -> Result<(), String> {
    taskbar::set_dragging(true);
    let (width, next) = {
        let mut cfg = lock(&state.config);
        let next = taskbar::nudge_offset(cfg.bar_offset_x, dx, cfg.bar_width);
        cfg.bar_offset_x = next;
        (cfg.bar_width, next)
    };
    if let Some(bar) = app.get_webview_window("bar") {
        let _ = taskbar::dock_bar(&bar, width, next);
    }
    Ok(())
}

#[tauri::command]
fn end_bar_drag(state: State<AppState>) -> Result<(), String> {
    taskbar::set_dragging(false);
    let cfg = lock(&state.config).clone();
    save_config(&cfg)
}

#[tauri::command]
fn reset_bar_position(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    taskbar::set_dragging(false);
    {
        let mut cfg = lock(&state.config);
        cfg.bar_offset_x = None;
        save_config(&cfg)?;
    }
    redock(&app);
    Ok(())
}

#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_stats(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("stats") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Async for the same reason as `current_quota`, and more so: this one runs a
/// dozen queries and the stats window asks for it every 15 seconds.
#[tauri::command]
async fn get_stats(state: State<'_, AppState>, key_id: Option<String>) -> Result<UsageStats, String> {
    let (pro, daily_quota, reset_utc, key) = {
        let cfg = lock(&state.config);
        let key = key_id
            .filter(|id| key_list(&cfg).iter().any(|k| &k.id == id))
            .or_else(|| active_id(&cfg))
            .unwrap_or_default();
        (cfg.pro_usd, cfg.daily_quota_usd, cfg.daily_reset_utc.clone(), key)
    };
    let reset = reset_window(reset_utc.as_deref(), quota::now_unix() as i64);
    let mut stats = db::load_stats(&lock(&state.db), &key, pro, daily_quota, reset)?;
    let live = lock(&state.keys).get(&key).map(|s| s.snap.clone()).unwrap_or_default();
    if !live.limits.is_empty() {
        stats.latest.limits = live.limits.clone();
    }
    // Surface a failing poll in the stats window too, without throwing away
    // the figures it is drawing.
    if live.error.is_some() {
        stats.latest.error = live.error.clone();
        stats.latest.stale = live.stale;
    }
    Ok(stats)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = db::open().expect("open usage.db");
    let config = load_config();
    if let Some(first) = key_list(&config).first() {
        let _ = db::adopt_legacy(&db, &first.id);
    }
    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(config),
            quota: Mutex::new(QuotaSnapshot {
                error: Some("starting…".into()),
                ..Default::default()
            }),
            keys: Mutex::new(HashMap::new()),
            claude_use: Mutex::new(HashMap::new()),
            last_switch: Mutex::new(None),
            db: Mutex::new(db),
            refresh: Notify::new(),
        })
        .invoke_handler(tauri::generate_handler![
            current_quota,
            get_settings,
            save_settings,
            refresh_now,
            begin_bar_drag,
            nudge_bar,
            nudge_bar_width,
            end_bar_drag,
            reset_bar_position,
            open_settings,
            open_stats,
            get_stats,
            list_keys,
            reveal_key,
            add_key,
            update_key,
            remove_key,
            move_key,
            set_active_key,
            set_failover,
            test_key,
            list_claude_targets,
            apply_claude,
            add_claude_target,
            remove_claude_target,
            set_target_sync,
            sync_claude_now
        ])
        .setup(|app| {
            let show_item = MenuItem::with_id(app, "show", "Show bar", true, None::<&str>)?;
            let stats_item = MenuItem::with_id(app, "stats", "Stats", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
            let reset_item =
                MenuItem::with_id(app, "reset-pos", "Reset position", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show_item,
                    &stats_item,
                    &refresh_item,
                    &reset_item,
                    &settings_item,
                    &quit_item,
                ],
            )?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Quota Bar")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => redock(app),
                    "stats" => {
                        let _ = open_stats(app.clone());
                    }
                    "refresh" => {
                        redock(app);
                        if let Some(state) = app.try_state::<AppState>() {
                            state.refresh.notify_one();
                        }
                    }
                    "reset-pos" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            let _ = reset_bar_position(app.clone(), state);
                        }
                    }
                    "settings" => {
                        let _ = open_settings(app.clone());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if let Some(bar) = app.get_webview_window("bar") {
                taskbar::watch_shell(&bar);
            }
            redock(app.handle());

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last_poll: Option<Instant> = None;
                let mut last_prune: Option<Instant> = None;
                loop {
                    let interval = Duration::from_secs(
                        handle
                            .try_state::<AppState>()
                            .map(|s| lock(&s.config).poll_interval_secs)
                            .unwrap_or(60)
                            .max(15),
                    );
                    // Sleep until the poll is due. The old loop woke every 1.5s
                    // regardless and re-docked on every wake, from this thread.
                    let nap = last_poll
                        .map(|t| interval.saturating_sub(t.elapsed()))
                        .unwrap_or(Duration::ZERO)
                        .min(IDLE_BACKOFF);

                    let forced = match handle.try_state::<AppState>() {
                        Some(state) => tokio::select! {
                            _ = state.refresh.notified() => true,
                            _ = tokio::time::sleep(nap) => false,
                        },
                        // No state means no poll, so `last_poll` never
                        // advances and `nap` stays at zero. Back off on the
                        // backstop instead of spinning.
                        None => {
                            tokio::time::sleep(IDLE_BACKOFF).await;
                            false
                        }
                    };

                    // Deliberately no docking here. This thread has no message
                    // pump, so a SetWindowPos or SHAppBarMessage from it can
                    // deadlock against the shell and hang every window on the
                    // desktop. The window timer owns all of that now.
                    let due = last_poll.map(|t| t.elapsed() >= interval).unwrap_or(true);
                    if forced || due {
                        if let Some(state) = handle.try_state::<AppState>() {
                            poll_once(&handle, state.inner()).await;
                            last_poll = Some(Instant::now());
                        }
                    }

                    if last_prune.map(|t| t.elapsed() >= PRUNE_EVERY).unwrap_or(true) {
                        if let Some(state) = handle.try_state::<AppState>() {
                            let _ = db::prune(&lock(&state.db), RETAIN_SECS);
                        }
                        last_prune = Some(Instant::now());
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Hold the windows open for an ordinary close, but never when
                // the shell is tearing the session down.
                if taskbar::shell_exiting() {
                    return;
                }
                api.prevent_close();
                if window.label() == "bar" {
                    let _ = window.show();
                } else {
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Quota Bar")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { api, code, .. } = event {
                // `code.is_none()` also covers logoff and shutdown. Refusing
                // there hangs Windows behind "this app is preventing you from
                // shutting down", so only a user-closed window keeps us alive.
                if code.is_none() && !taskbar::shell_exiting() {
                    api.prevent_exit();
                }
            }
        });
}
