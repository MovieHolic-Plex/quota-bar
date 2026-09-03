mod config;
mod db;
mod quota;
mod taskbar;

use config::{
    has_api_key, key_preview, load_api_key, load_config, normalize_reset, reset_window, save_config,
    store_api_key, AppConfig,
};
use db::{BucketRow, UsageStats};
use quota::{fetch_usage, QuotaSnapshot};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tokio::sync::Notify;

struct AppState {
    config: Mutex<AppConfig>,
    quota: Mutex<QuotaSnapshot>,
    db: Mutex<Connection>,
    refresh: Notify,
}

#[derive(Debug, Deserialize)]
struct SettingsPatch {
    base_url: String,
    poll_interval_secs: u64,
    pro_usd: f64,
    #[serde(default)]
    daily_reset_utc: Option<String>,
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct SettingsView {
    base_url: String,
    poll_interval_secs: u64,
    pro_usd: f64,
    bar_width: u32,
    daily_reset_utc: Option<String>,
    has_key: bool,
    key_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BarView {
    #[serde(flatten)]
    snap: QuotaSnapshot,
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
    let (daily_quota_usd, daily_reset_utc) = {
        let cfg = state.config.lock().unwrap();
        (cfg.daily_quota_usd.max(1.0), cfg.daily_reset_utc.clone())
    };
    let now = quota::now_unix() as i64;
    let reset = reset_window(daily_reset_utc.as_deref(), now);
    let db = state.db.lock().unwrap();
    let minutes = db::minute_series(&db, 30).unwrap_or_default();
    let (spend_10m, spend_1h, spend_1d) = db::recent_spend(&db).unwrap_or((0.0, 0.0, 0.0));
    let spend_since_reset = reset.map(|(last, _)| db::spend_since(&db, last).unwrap_or(0.0));
    let daily_pct = (spend_since_reset.unwrap_or(spend_1d) / daily_quota_usd) * 100.0;
    BarView {
        snap,
        minutes,
        spend_10m,
        spend_1h,
        spend_1d,
        daily_quota_usd,
        daily_pct,
        spend_since_reset,
        reset_in_secs: reset.map(|(_, next)| (next - now).max(0)),
        daily_reset_utc,
    }
}

fn emit_quota(app: &AppHandle, view: &BarView) {
    let _ = app.emit("quota-update", view);
}

async fn poll_once(app: &AppHandle, state: &AppState) -> QuotaSnapshot {
    let (base, pro) = {
        let cfg = state.config.lock().unwrap();
        (cfg.base_url.clone(), cfg.pro_usd)
    };
    let Some(key) = load_api_key() else {
        let snap = QuotaSnapshot {
            error: Some("no api key".into()),
            ..Default::default()
        };
        *state.quota.lock().unwrap() = snap.clone();
        emit_quota(app, &bar_view(state, snap.clone()));
        return snap;
    };
    match fetch_usage(&base, &key).await {
        Ok(mut snap) => {
            apply_pro(&mut snap, pro);
            if let Err(err) = db::insert_snapshot(&state.db.lock().unwrap(), &snap) {
                snap.error = Some(format!("db: {err}"));
            }
            *state.quota.lock().unwrap() = snap.clone();
            emit_quota(app, &bar_view(state, snap.clone()));
            snap
        }
        Err(err) => {
            let snap = QuotaSnapshot {
                error: Some(err),
                ..Default::default()
            };
            *state.quota.lock().unwrap() = snap.clone();
            emit_quota(app, &bar_view(state, snap.clone()));
            snap
        }
    }
}

fn redock(app: &AppHandle) {
    if taskbar::is_dragging() {
        return;
    }
    let (width, offset) = app
        .try_state::<AppState>()
        .map(|s| {
            let cfg = s.config.lock().unwrap();
            (cfg.bar_width, cfg.bar_offset_x)
        })
        .unwrap_or((420, None));
    if let Some(bar) = app.get_webview_window("bar") {
        let _ = taskbar::dock_bar(&bar, width, offset);
    }
}

#[tauri::command]
fn current_quota(state: State<AppState>) -> BarView {
    bar_view(&*state, state.quota.lock().unwrap().clone())
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsView {
    let cfg = state.config.lock().unwrap().clone();
    SettingsView {
        base_url: cfg.base_url,
        poll_interval_secs: cfg.poll_interval_secs,
        pro_usd: cfg.pro_usd,
        bar_width: cfg.bar_width,
        daily_reset_utc: cfg.daily_reset_utc,
        has_key: has_api_key(),
        key_preview: key_preview(),
    }
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: SettingsPatch) -> Result<(), String> {
    if let Some(key) = settings.api_key.as_deref() {
        if !key.trim().is_empty() {
            store_api_key(key)?;
        }
    }
    let mut cfg = state.config.lock().unwrap();
    cfg.base_url = settings.base_url.trim_end_matches('/').to_string();
    cfg.poll_interval_secs = settings.poll_interval_secs.max(15);
    cfg.pro_usd = if settings.pro_usd > 0.0 { settings.pro_usd } else { 20.0 };
    if let Some(raw) = settings.daily_reset_utc.as_deref() {
        if !raw.trim().is_empty() && normalize_reset(Some(raw)).is_none() {
            return Err("Daily reset time must be HH:MM (UTC), e.g. 06:34".into());
        }
    }
    cfg.daily_reset_utc = normalize_reset(settings.daily_reset_utc.as_deref());
    save_config(&cfg)?;
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn refresh_now(state: State<AppState>) {
    state.refresh.notify_one();
}

#[tauri::command]
fn begin_bar_drag() {
    taskbar::set_dragging(true);
}

#[tauri::command]
fn nudge_bar(app: AppHandle, state: State<AppState>, dx: i32) -> Result<(), String> {
    taskbar::set_dragging(true);
    let (width, next) = {
        let mut cfg = state.config.lock().unwrap();
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
    let cfg = state.config.lock().unwrap().clone();
    save_config(&cfg)
}

#[tauri::command]
fn reset_bar_position(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    taskbar::set_dragging(false);
    {
        let mut cfg = state.config.lock().unwrap();
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

#[tauri::command]
fn get_stats(state: State<AppState>) -> Result<UsageStats, String> {
    let (pro, daily_quota, reset_utc) = {
        let cfg = state.config.lock().unwrap();
        (cfg.pro_usd, cfg.daily_quota_usd, cfg.daily_reset_utc.clone())
    };
    let reset = reset_window(reset_utc.as_deref(), quota::now_unix() as i64);
    db::load_stats(&state.db.lock().unwrap(), pro, daily_quota, reset)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = db::open().expect("open usage.db");
    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(load_config()),
            quota: Mutex::new(QuotaSnapshot {
                error: Some("starting…".into()),
                ..Default::default()
            }),
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
            end_bar_drag,
            reset_bar_position,
            open_settings,
            open_stats,
            get_stats
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

            redock(app.handle());

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last = Instant::now() - Duration::from_secs(10_000);
                loop {
                    let interval = handle
                        .try_state::<AppState>()
                        .map(|s| s.config.lock().unwrap().poll_interval_secs)
                        .unwrap_or(60)
                        .max(15);
                    let due = last.elapsed() >= Duration::from_secs(interval);
                    let notified = if let Some(state) = handle.try_state::<AppState>() {
                        let timeout = tokio::time::sleep(Duration::from_millis(1500));
                        tokio::select! {
                            _ = state.refresh.notified() => true,
                            _ = timeout => false,
                        }
                    } else {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        false
                    };
                    redock(&handle);
                    if due || notified {
                        if let Some(state) = handle.try_state::<AppState>() {
                            poll_once(&handle, state.inner()).await;
                            last = Instant::now();
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
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
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
