mod config;
mod quota;
mod taskbar;

use config::{has_api_key, key_preview, load_api_key, load_config, save_config, store_api_key, AppConfig};
use quota::{fetch_quota, QuotaSnapshot};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;

struct AppState {
    config: Mutex<AppConfig>,
    quota: Mutex<QuotaSnapshot>,
    refresh: Notify,
}

#[derive(Debug, Deserialize)]
struct SettingsPatch {
    base_url: String,
    poll_interval_secs: u64,
    probe_model: String,
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct SettingsView {
    base_url: String,
    poll_interval_secs: u64,
    probe_model: String,
    bar_width: u32,
    has_key: bool,
    key_preview: Option<String>,
}

fn emit_quota(app: &AppHandle, snap: &QuotaSnapshot) {
    let _ = app.emit("quota-update", snap);
}

async fn poll_once(app: &AppHandle, state: &AppState) -> QuotaSnapshot {
    let (base, model) = {
        let cfg = state.config.lock().unwrap();
        (cfg.base_url.clone(), cfg.probe_model.clone())
    };
    let Some(key) = load_api_key() else {
        let snap = QuotaSnapshot {
            error: Some("no api key".into()),
            ..Default::default()
        };
        *state.quota.lock().unwrap() = snap.clone();
        emit_quota(app, &snap);
        return snap;
    };
    match fetch_quota(&base, &key, &model).await {
        Ok(snap) => {
            *state.quota.lock().unwrap() = snap.clone();
            emit_quota(app, &snap);
            snap
        }
        Err(err) => {
            let snap = QuotaSnapshot {
                error: Some(err),
                ..Default::default()
            };
            *state.quota.lock().unwrap() = snap.clone();
            emit_quota(app, &snap);
            snap
        }
    }
}

fn redock(app: &AppHandle) {
    let width = app
        .try_state::<AppState>()
        .map(|s| s.config.lock().unwrap().bar_width)
        .unwrap_or(420);
    if let Some(bar) = app.get_webview_window("bar") {
        let _ = taskbar::dock_bar(&bar, width);
    }
}

#[tauri::command]
fn current_quota(state: State<AppState>) -> QuotaSnapshot {
    state.quota.lock().unwrap().clone()
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsView {
    let cfg = state.config.lock().unwrap().clone();
    SettingsView {
        base_url: cfg.base_url,
        poll_interval_secs: cfg.poll_interval_secs,
        probe_model: cfg.probe_model,
        bar_width: cfg.bar_width,
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
    cfg.poll_interval_secs = settings.poll_interval_secs.max(30);
    if !settings.probe_model.trim().is_empty() {
        cfg.probe_model = settings.probe_model;
    }
    save_config(&cfg)?;
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn refresh_now(state: State<AppState>) {
    state.refresh.notify_one();
}

#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(load_config()),
            quota: Mutex::new(QuotaSnapshot {
                error: Some("starting…".into()),
                ..Default::default()
            }),
            refresh: Notify::new(),
        })
        .invoke_handler(tauri::generate_handler![
            current_quota,
            get_settings,
            save_settings,
            refresh_now,
            open_settings
        ])
        .setup(|app| {
            let refresh_item = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&refresh_item, &settings_item, &quit_item])?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Quota Bar")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "refresh" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            state.refresh.notify_one();
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
                    redock(&handle);
                    let interval = handle
                        .try_state::<AppState>()
                        .map(|s| s.config.lock().unwrap().poll_interval_secs)
                        .unwrap_or(180)
                        .max(30);
                    let due = last.elapsed() >= Duration::from_secs(interval);
                    let notified = if let Some(state) = handle.try_state::<AppState>() {
                        let timeout = tokio::time::sleep(Duration::from_millis(250));
                        tokio::select! {
                            _ = state.refresh.notified() => true,
                            _ = timeout => false,
                        }
                    } else {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                        false
                    };
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
        .run(tauri::generate_context!())
        .expect("error while running Quota Bar");
}
