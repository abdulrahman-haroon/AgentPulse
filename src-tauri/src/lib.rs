mod actions;
mod events;
mod hooks;
mod logs;
mod pricing;
mod push;
mod settings;
mod stats;

use jiff::{SignedDuration, Timestamp, ZonedDateTime};
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
    /// Installed copy of the hook script that settings.json points at.
    script_path: PathBuf,
    settings: Mutex<settings::Settings>,
    settings_path: PathBuf,
    approvals: events::Approvals,
    /// Where answers to permission prompts are dropped for the waiting hook script.
    decisions_dir: PathBuf,
}

/// Answers a permission prompt that the hook is holding open. Only requests the hook actually
/// sent (and that haven't expired) can be answered; the id is a UUID, so it's safe as a filename.
#[tauri::command]
fn decide_permission(state: TauriState<'_, AppState>, request_id: String, allow: bool, reason: String) -> Result<(), String> {
    let mut approvals = state.approvals.lock().map_err(|e| e.to_string())?;
    events::decide(&mut approvals, &state.decisions_dir, &request_id, allow, &reason, Timestamp::now())
}

#[tauri::command]
fn get_settings(state: TauriState<'_, AppState>) -> Result<settings::Settings, String> {
    state.settings.lock().map(|s| s.clone()).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(state: TauriState<'_, AppState>, settings: settings::Settings) -> Result<settings::Settings, String> {
    settings::save(&state.settings_path, &settings)?;
    *state.settings.lock().map_err(|e| e.to_string())? = settings.clone();
    Ok(settings)
}

/// Sends a sample "needs you" push with the given (unsaved) settings so the user can check their phone.
#[tauri::command]
async fn test_push(push: settings::PushSettings) -> Result<(), String> {
    settings::validate_push(&push)?;
    let sample = events::Alert {
        kind: events::AlertKind::NeedsYou,
        project: "AgentPulse".into(),
        title: "AgentPulse test".into(),
        body: "Phone notifications are working.".into(),
    };
    let mut body = push::payload(&push, &sample);
    body["title"] = "AgentPulse test".into();
    body["message"] = "Phone notifications are working.".into();
    push::send(&push, &body).await
}

/// Desktop notification for every alert; phone push if the user turned it on.
fn on_alert(app: &AppHandle, alert: &events::Alert) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(&alert.title).body(&alert.body).show();
    let push = app.state::<AppState>().settings.lock().map(|s| s.push.clone()).unwrap_or_default();
    if push::wanted(&push, alert) {
        let body = push::payload(&push, alert);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = push::send(&push, &body).await {
                eprintln!("[agentpulse] phone push failed: {e}");
            }
        });
    }
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

/// Everything the UI shows, computed from logs (cached, incremental) and live hook state.
#[tauri::command]
async fn dashboard(app: AppHandle) -> Result<stats::Dashboard, String> {
    blocking(app, |state| {
        let mut cache = state.cache.lock().map_err(|e| e.to_string())?;
        refresh(state, &mut cache);
        let live = state.live.lock().map_err(|e| e.to_string())?.clone();
        let mut d = stats::build(cache.requests(), &cache.titles(), &cache.meta(), &live, &ZonedDateTime::now());
        d.replies = state.replies.views();
        if let Ok(ap) = state.approvals.lock() {
            d.approvals = stats::approval_views(&ap, &cache.titles());
        }
        if let Ok(mut s) = state.states.lock() {
            *s = d.sessions.iter().map(|v| (v.id.clone(), v.state)).collect();
        }
        Ok(d)
    })
    .await
}

/// Every session from the last 30 days, for the session browser (fetched on demand, not polled).
#[tauri::command]
async fn session_history(app: AppHandle) -> Result<Vec<stats::SessionRow>, String> {
    blocking(app, |state| {
        let mut cache = state.cache.lock().map_err(|e| e.to_string())?;
        refresh(state, &mut cache);
        Ok(stats::history(cache.requests(), &cache.titles(), &cache.meta(), &ZonedDateTime::now()))
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

#[derive(serde::Serialize)]
struct HookInfo {
    #[serde(flatten)]
    status: hooks::HookStatus,
    settings_path: String,
    /// Set after connect/disconnect when an existing settings.json was backed up.
    backup_path: Option<String>,
}

fn hook_info(state: &AppState, backup: Option<PathBuf>) -> Result<HookInfo, String> {
    let path = state.claude_dir.join("settings.json");
    Ok(HookInfo {
        status: hooks::status(&hooks::read_settings(&path)?),
        settings_path: path.to_string_lossy().into_owned(),
        backup_path: backup.map(|b| b.to_string_lossy().into_owned()),
    })
}

/// Which AgentPulse hooks are registered in Claude Code's settings.json (read-only).
#[tauri::command]
async fn hook_status(app: AppHandle) -> Result<HookInfo, String> {
    blocking(app, |s| hook_info(s, None)).await
}

/// Installs the hook script into AgentPulse's data folder and registers it for all five
/// events in settings.json (backed up first). Only our own entries are touched.
#[tauri::command]
async fn connect_hooks(app: AppHandle) -> Result<HookInfo, String> {
    blocking(app, |s| {
        install_script(&s.script_path)?;
        let path = s.claude_dir.join("settings.json");
        let updated = hooks::connect(hooks::read_settings(&path)?, &hooks::command_for(&s.script_path))?;
        std::fs::create_dir_all(&s.claude_dir).map_err(|e| e.to_string())?;
        let backup = hooks::write_settings(&path, &updated)?;
        hook_info(s, backup)
    })
    .await
}

#[tauri::command]
async fn disconnect_hooks(app: AppHandle) -> Result<HookInfo, String> {
    blocking(app, |s| {
        let path = s.claude_dir.join("settings.json");
        let updated = hooks::disconnect(hooks::read_settings(&path)?)?;
        let backup = hooks::write_settings(&path, &updated)?;
        hook_info(s, backup)
    })
    .await
}

fn install_script(path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, hooks::SCRIPT).map_err(|e| format!("Couldn't install the hook script: {e}"))
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let a = app.autolaunch();
    if enabled { a.enable() } else { a.disable() }.map_err(|e| e.to_string())?;
    a.is_enabled().map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    current: String,
    /// Newer version on GitHub Releases, if any.
    available: Option<String>,
    notes: Option<String>,
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Asks GitHub Releases for a newer version. The only network call AgentPulse makes.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| format!("Couldn't check for updates: {e}"))?;
    Ok(UpdateInfo {
        current: app.package_info().version.to_string(),
        available: update.as_ref().map(|u| u.version.clone()),
        notes: update.and_then(|u| u.body),
    })
}

/// Downloads the update, verifies its signature against our public key, installs, restarts.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else { return Err("Already up to date.".into()) };
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| format!("Update failed: {e}"))?;
    app.restart();
}

/// Passed by the login item so AgentPulse starts in the tray without opening a window.
const MINIMIZED_FLAG: &str = "--minimized";

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
    let recent = Timestamp::now() - SignedDuration::from_hours(8);
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
        // Must be first: a second launch just focuses the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_FLAG]),
        ))
        .invoke_handler(tauri::generate_handler![
            dashboard,
            hook_status,
            connect_hooks,
            disconnect_hooks,
            get_autostart,
            set_autostart,
            app_version,
            check_update,
            install_update,
            get_settings,
            save_settings,
            test_push,
            decide_permission,
            session_history,
            toggle_widget,
            open_session,
            send_reply,
            cancel_reply,
            dismiss_reply
        ])
        .setup(|app| {
            let home = app.path().home_dir()?;
            let data = app.path().app_local_data_dir()?;
            let live: LiveStore = Arc::new(Mutex::new(HashMap::new()));
            let approvals: events::Approvals = Arc::new(Mutex::new(HashMap::new()));
            // Answers left from a previous run have no hook waiting for them.
            events::clear_stale_decisions(&data.join("decisions"));
            let script_path = data.join("hooks").join("notify.ps1");
            // Keep an installed hook script in sync with this app version.
            if script_path.exists() {
                let _ = install_script(&script_path);
            }
            app.manage(AppState {
                claude_dir: logs::claude_dir(&home),
                cache: Mutex::new(logs::LogCache::default()),
                live: live.clone(),
                replies: actions::Replies::default(),
                states: Mutex::new(HashMap::new()),
                script_path,
                settings: Mutex::new(settings::load(&data.join("settings.json"))),
                settings_path: data.join("settings.json"),
                approvals: approvals.clone(),
                decisions_dir: data.join("decisions"),
            });
            if !std::env::args().any(|a| a == MINIMIZED_FLAG) {
                show_main(app.handle());
            }

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

            let events_file = data.join("events.jsonl");
            events::spawn_watcher(app.handle().clone(), events_file, live, approvals, on_alert, update_tray);

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
