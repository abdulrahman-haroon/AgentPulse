mod actions;
mod events;
mod logs;
mod pricing;
mod stats;

use chrono::{Duration, Local};
use events::{Live, LiveStore, State};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State as TauriState, WindowEvent};

struct AppState {
    claude_dir: PathBuf,
    cache: Mutex<logs::LogCache>,
    live: LiveStore,
    replies: actions::Replies,
    /// Session states from the latest dashboard, so actions can refuse to touch running sessions.
    states: Mutex<HashMap<String, &'static str>>,
}

/// Runs blocking work (file reads, locks the cache) off both the UI and async threads.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}

/// A session's folder/entrypoint/model (from our own log cache, never from the UI) and
/// whether it's currently running.
fn session_info(state: &AppState, id: &str) -> Result<(logs::SessionMeta, bool), String> {
    let cache = state.cache.lock().map_err(|e| e.to_string())?;
    let meta = cache.meta().get(id).map(|m| (*m).clone()).ok_or("Unknown session.")?;
    let running = state
        .states
        .lock()
        .map_err(|e| e.to_string())?
        .get(id)
        .is_some_and(|s| matches!(*s, "working" | "needs_you"));
    Ok((meta, running))
}

/// Focus the session in VS Code, or resume it in a new terminal.
#[tauri::command]
async fn open_session(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |s| {
        let (m, running) = session_info(s, &id)?;
        actions::open_session(&id, &m.cwd, &m.entrypoint, running)
    })
    .await
}

/// Send a follow-up to a finished/idle session via Claude Code's headless mode.
#[tauri::command]
async fn send_reply(app: AppHandle, id: String, message: String, mode: String) -> Result<(), String> {
    blocking(app, move |s| {
        let (m, running) = session_info(s, &id)?;
        if running {
            return Err("This session is busy. Wait until it's done.".into());
        }
        actions::start_reply(&s.replies, &id, &m.cwd, m.model.as_deref(), &message, &mode)
    })
    .await
}

#[tauri::command]
fn cancel_reply(state: TauriState<'_, AppState>, id: String) {
    state.replies.cancel(&id);
}

#[tauri::command]
fn dismiss_reply(state: TauriState<'_, AppState>, id: String) {
    state.replies.dismiss(&id);
}

#[derive(serde::Serialize)]
struct HookStatus {
    /// Hook events in ~/.claude/settings.json that run the AgentPulse script.
    installed: Vec<String>,
    missing: Vec<String>,
}

const HOOK_EVENTS: [&str; 5] = ["SessionStart", "UserPromptSubmit", "Notification", "Stop", "SessionEnd"];

/// Everything the UI shows, computed from logs (cached, incremental) and live hook state.
#[tauri::command]
async fn dashboard(app: AppHandle) -> Result<stats::Dashboard, String> {
    blocking(app, |state| {
        let mut cache = state.cache.lock().map_err(|e| e.to_string())?;
        refresh(state, &mut cache);
        let live = state.live.lock().map_err(|e| e.to_string())?.clone();
        let mut d = stats::build(cache.requests(), &cache.titles(), &cache.meta(), &live, Local::now());
        d.replies = state.replies.views();
        if let Ok(mut s) = state.states.lock() {
            *s = d.sessions.iter().map(|v| (v.id.clone(), v.state)).collect();
        }
        Ok(d)
    })
    .await
}

fn refresh(state: &AppState, cache: &mut logs::LogCache) {
    // One spare day so files touched just before the window's first midnight are included.
    let since = SystemTime::now() - std::time::Duration::from_secs((stats::HISTORY_DAYS as u64 + 1) * 86_400);
    let started = std::time::Instant::now();
    cache.refresh(&state.claude_dir.join("projects"), since);
    if cfg!(debug_assertions) && started.elapsed().as_millis() > 200 {
        eprintln!("[agentpulse] log refresh took {:?}", started.elapsed());
    }
}

/// Which AgentPulse hooks are registered in Claude Code settings (read-only check).
#[tauri::command]
fn hook_status(state: TauriState<'_, AppState>) -> HookStatus {
    let settings = std::fs::read_to_string(state.claude_dir.join("settings.json")).unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&settings).unwrap_or_default();
    let (installed, missing) = HOOK_EVENTS.iter().map(|e| e.to_string()).partition(|e| {
        json["hooks"][e].to_string().contains("notify.ps1")
    });
    HookStatus { installed, missing }
}

#[tauri::command]
fn toggle_widget(app: AppHandle) {
    if let Some(w) = app.get_webview_window("widget") {
        let _ = if w.is_visible().unwrap_or(false) { w.hide() } else { w.show() };
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// A 32×32 filled circle; the tray icon is a status light.
fn dot(rgb: [u8; 3]) -> Image<'static> {
    const N: i32 = 32;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            // 1px anti-aliased edge
            let a = (13.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            px.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0) as u8]);
        }
    }
    Image::new_owned(px, N as u32, N as u32)
}

fn update_tray(app: &AppHandle, live: &HashMap<String, Live>) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let recent = Local::now() - Duration::hours(8);
    let count = |s| live.values().filter(|l| l.state == s && l.since > recent).count();
    let (needs, working) = (count(State::NeedsYou), count(State::Working));
    let (color, tip) = match (needs, working) {
        (0, 0) => ([0x89, 0x87, 0x81], "AgentPulse: all quiet".to_string()),
        (0, w) => ([0x0c, 0xa3, 0x0c], format!("AgentPulse: {w} working")),
        (n, w) => ([0xfa, 0xb2, 0x19], format!("AgentPulse: {n} need you, {w} working")),
    };
    set_tray(&tray, color, &tip);
}

fn set_tray(tray: &TrayIcon, color: [u8; 3], tip: &str) {
    let _ = tray.set_icon(Some(dot(color)));
    let _ = tray.set_tooltip(Some(tip));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            dashboard,
            hook_status,
            toggle_widget,
            open_session,
            send_reply,
            cancel_reply,
            dismiss_reply
        ])
        .setup(|app| {
            let home = app.path().home_dir()?;
            let live: LiveStore = Arc::new(Mutex::new(HashMap::new()));
            app.manage(AppState {
                claude_dir: logs::claude_dir(&home),
                cache: Mutex::new(logs::LogCache::default()),
                live: live.clone(),
                replies: actions::Replies::default(),
                states: Mutex::new(HashMap::new()),
            });

            let open = MenuItem::with_id(app, "open", "Open AgentPulse", true, None::<&str>)?;
            let widget = MenuItem::with_id(app, "widget", "Toggle floating widget", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &widget, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(dot([0x89, 0x87, 0x81]))
                .tooltip("AgentPulse")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "widget" => toggle_widget(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            let events_file = app.path().app_local_data_dir()?.join("events.jsonl");
            events::spawn_watcher(app.handle().clone(), events_file, live, update_tray);

            // Parse log history in the background so the first dashboard call is fast.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let state = handle.state::<AppState>();
                if let Ok(mut c) = state.cache.lock() {
                    refresh(&state, &mut c);
                }
            });
            Ok(())
        })
        // Closing a window hides it; "Quit" in the tray menu exits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running AgentPulse");
}
