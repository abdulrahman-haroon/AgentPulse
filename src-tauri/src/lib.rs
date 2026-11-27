mod actions;
mod codex;
mod copilot;
mod events;
mod hooks;
mod logs;
mod pricing;
mod push;
mod settings;
mod sound;
mod stats;

use events::{AlertKind, LiveStore, State};
use jiff::{SignedDuration, Timestamp, ZonedDateTime};
use logs::Provider;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State as TauriState, WindowEvent};

struct AppState {
    claude_dir: PathBuf,
    /// Log folders of every supported agent.
    roots: Vec<(PathBuf, Provider)>,
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
    /// Exists while the floating widget is on, so it comes back after a restart.
    widget_flag: PathBuf,
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
        kind: AlertKind::NeedsYou,
        project: "AgentPulse".into(),
        title: "AgentPulse test".into(),
        body: "Phone notifications are working.".into(),
    };
    let mut body = push::payload(&push, &sample);
    body["title"] = "AgentPulse test".into();
    body["message"] = "Phone notifications are working.".into();
    push::send(&push, &body).await
}

/// The widget's bell: alert sounds on/off, every other setting kept. Narrower than save_settings,
/// so the widget never gets to change push or approval settings.
#[tauri::command]
fn set_sound(state: TauriState<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let mut current = state.settings.lock().map_err(|e| e.to_string())?;
    let mut next = current.clone();
    next.sound.enabled = enabled;
    settings::save(&state.settings_path, &next)?;
    *current = next;
    Ok(enabled)
}

/// Plays an alert chime so the user can hear it (Settings → "Play").
#[tauri::command]
fn test_sound(kind: String) {
    sound::play(if kind == "needs_you" { AlertKind::NeedsYou } else { AlertKind::Done });
}

/// Desktop notification and chime for every alert; phone push if the user turned it on.
fn on_alert(app: &AppHandle, alert: &events::Alert) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(&alert.title).body(&alert.body).show();
    let s = app.state::<AppState>().settings.lock().map(|s| s.clone()).unwrap_or_default();
    if s.sound.wanted(alert.kind) {
        sound::play(alert.kind);
    }
    if push::wanted(&s.push, alert) {
        let body = push::payload(&s.push, alert);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = push::send(&s.push, &body).await {
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
/// whether it's currently running. Only Claude Code sessions can be opened or replied to.
fn session_info(state: &AppState, id: &str) -> Result<(logs::SessionMeta, bool), String> {
    let cache = state.cache.lock().map_err(|e| e.to_string())?;
    let meta = cache.meta().get(id).map(|m| (*m).clone()).ok_or("Unknown session.")?;
    if meta.provider != Provider::Claude {
        return Err(format!("Open and Reply work with Claude Code sessions only. Switch to {} for this one.", meta.provider.label()));
    }
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

/// Brings up the session's project in VS Code, for any agent (widget row click, Codex/Copilot
/// cards). The folder comes from our log index, never from the UI.
#[tauri::command]
async fn focus_session(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |s| {
        let cwd = s.cache.lock().map_err(|e| e.to_string())?.meta().get(id.as_str()).map(|m| m.cwd.clone()).ok_or("Unknown session.")?;
        actions::open_in_vscode(&id, &cwd)
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
        let now = ZonedDateTime::now();
        let mut d = stats::build(cache.requests(), &cache.titles(), &cache.meta(), &cache.agents(), &live, &now);
        d.limits = cache.limits(now.timestamp());
        d.sound_enabled = state.settings.lock().map(|s| s.sound.enabled).unwrap_or(true);
        d.replies = state.replies.views();
        if let Ok(ap) = state.approvals.lock() {
            d.approvals = stats::approval_views(&ap, &cache.titles());
        }
        d.sources = state
            .roots
            .iter()
            .map(|(dir, p)| stats::SourceView { provider: *p, label: p.label(), path: dir.to_string_lossy().into_owned(), found: dir.is_dir() })
            .collect();
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
    let since = SystemTime::now() - Duration::from_secs((stats::HISTORY_DAYS as u64 + 1) * 86_400);
    let started = std::time::Instant::now();
    cache.refresh(&state.roots, since);
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

/// Installs the hook script into AgentPulse's data folder and registers it for all six
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
    // A dev build loads its UI from the Vite dev server, which isn't running at sign-in, so it
    // would start as a blank window. Only an installed (release) build may register itself.
    if enabled && cfg!(debug_assertions) {
        return Err("Start with Windows works in the installed app only. Dev builds need `npm run tauri dev` running.".into());
    }
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

/// Asks GitHub Releases for a newer version.
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
        let show = !w.is_visible().unwrap_or(false);
        let _ = if show { w.show() } else { w.hide() };
        remember_widget(&app, show);
    }
}

fn remember_widget(app: &AppHandle, visible: bool) {
    let flag = &app.state::<AppState>().widget_flag;
    let _ = if visible { std::fs::write(flag, "") } else { std::fs::remove_file(flag) };
}

/// The widget's "open dashboard" button.
#[tauri::command]
fn open_dashboard(app: AppHandle) {
    show_main(&app);
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Distance from (px, py) to the segment a–b.
fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - a.0 - t * dx).hypot(py - a.1 - t * dy)
}

/// 32×32 tray icon: the AgentPulse mark (pulse line on a dark rounded square) with the line in the
/// status color. Drawn here, so no image files or decoders are needed.
fn tray_icon(line: [u8; 3]) -> Image<'static> {
    const N: usize = 32;
    const BG: [u8; 3] = [0x15, 0x18, 0x2b];
    // The logo's pulse, in tray pixels (same shape as icons/icon-source.svg).
    const PTS: [(f32, f32); 6] = [(5.5, 17.5), (11.0, 17.5), (13.5, 9.0), (18.5, 24.0), (21.0, 17.5), (26.5, 17.5)];
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square, corner radius 8, 1px anti-aliased edge.
            let (qx, qy) = ((cx - 16.0).abs() - 8.0, (cy - 16.0).abs() - 8.0);
            let edge = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - 8.0;
            let fill = (0.5 - edge).clamp(0.0, 1.0);
            let dist = PTS.windows(2).map(|w| seg_dist(cx, cy, w[0], w[1])).fold(f32::MAX, f32::min);
            let ink = (2.25 - dist).clamp(0.0, 1.0); // ~3.5px stroke
            let c = |i: usize| (BG[i] as f32 * (1.0 - ink) + line[i] as f32 * ink) as u8;
            px.extend_from_slice(&[c(0), c(1), c(2), (fill * 255.0) as u8]);
        }
    }
    Image::new_owned(px, N as u32, N as u32)
}

const QUIET: [u8; 3] = [0x8b, 0x93, 0xa7];
const WORKING: [u8; 3] = [0x38, 0xbd, 0xf8];
const NEEDS_YOU: [u8; 3] = [0xfb, 0xbf, 0x24];

/// Tray color + tooltip from hook states, log-only agents and running sub-agents.
fn tray_status(live: &HashMap<String, events::Live>, logs: &HashMap<String, stats::LogState>, agent_sessions: &[String]) -> ([u8; 3], String) {
    let recent = Timestamp::now() - SignedDuration::from_hours(8);
    let hooked = |s| live.values().filter(|l| l.state == s && l.since > recent).count();
    let needs = hooked(State::NeedsYou);
    let working = hooked(State::Working) + logs.values().filter(|s| s.state == "working").count();
    // Sub-agents of a session that has ended were stopped with it.
    let agents = agent_sessions.iter().filter(|id| live.get(*id).is_none_or(|l| l.state != State::Ended)).count();
    let mut parts = Vec::new();
    if needs > 0 {
        parts.push(format!("{needs} need{} you", if needs == 1 { "s" } else { "" }));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if agents > 0 {
        parts.push(format!("{agents} sub-agent{} running", if agents == 1 { "" } else { "s" }));
    }
    let tip = if parts.is_empty() { "AgentPulse: all quiet".to_string() } else { format!("AgentPulse: {}", parts.join(", ")) };
    let color = if needs > 0 { NEEDS_YOU } else if working + agents > 0 { WORKING } else { QUIET };
    (color, tip)
}

/// Every 2 s: refresh all agent logs (agents without hooks only show up there), alert when a
/// log-only session finishes its turn, and keep the tray light current. The first pass also
/// parses the whole history, so the first dashboard call is fast.
fn spawn_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut before: Option<HashMap<String, stats::LogState>> = None;
        let mut shown: Option<([u8; 3], String)> = None;
        loop {
            let state = app.state::<AppState>();
            let snapshot = state.cache.lock().ok().map(|mut cache| {
                refresh(&state, &mut cache);
                let now = Timestamp::now();
                let agents: Vec<String> = cache.agents().into_iter().filter(|a| stats::agent_running(a, now)).map(|a| a.session.clone()).collect();
                (stats::log_states(&cache.meta(), now), agents)
            });
            if let Some((logs, agents)) = snapshot {
                if let Some(b) = &before {
                    for (kind, s) in stats::log_alerts(b, &logs) {
                        let (what, body) = match kind {
                            AlertKind::Done => ("finished", "Task complete."),
                            AlertKind::NeedsYou => ("needs you", "Waiting for your approval."),
                        };
                        let title = format!("{} {what}: {}", s.provider.label(), s.project);
                        on_alert(&app, &events::Alert { kind, project: s.project.clone(), title, body: body.into() });
                    }
                }
                let live = state.live.lock().map(|l| l.clone()).unwrap_or_default();
                let status = tray_status(&live, &logs, &agents);
                if shown.as_ref() != Some(&status) {
                    if let Some(tray) = app.tray_by_id("main") {
                        let _ = tray.set_icon(Some(tray_icon(status.0)));
                        let _ = tray.set_tooltip(Some(&status.1));
                    }
                    shown = Some(status);
                }
                before = Some(logs);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri_plugin_window_state::StateFlags;
    tauri::Builder::default()
        // Must be first: a second launch just focuses the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_FLAG]),
        ))
        // Windows reopen where the user left them; visibility is ours (tray start, widget flag).
        .plugin(tauri_plugin_window_state::Builder::new().with_state_flags(StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED).build())
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
            test_sound,
            set_sound,
            decide_permission,
            session_history,
            toggle_widget,
            open_dashboard,
            open_session,
            focus_session,
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
            // Point an existing login item at this copy of the app (it may have been registered
            // by an older install or another folder). Dev builds never register themselves.
            if !cfg!(debug_assertions) {
                use tauri_plugin_autostart::ManagerExt;
                let a = app.autolaunch();
                if a.is_enabled().unwrap_or(false) {
                    let _ = a.enable();
                }
            }
            let widget_flag = data.join("widget-visible");
            app.manage(AppState {
                claude_dir: logs::claude_dir(&home),
                roots: logs::roots(&home),
                cache: Mutex::new(logs::LogCache::default()),
                live: live.clone(),
                replies: actions::Replies::default(),
                states: Mutex::new(HashMap::new()),
                script_path,
                settings: Mutex::new(settings::load(&data.join("settings.json"))),
                settings_path: data.join("settings.json"),
                approvals: approvals.clone(),
                decisions_dir: data.join("decisions"),
                widget_flag: widget_flag.clone(),
            });
            if !std::env::args().any(|a| a == MINIMIZED_FLAG) {
                show_main(app.handle());
            }
            if widget_flag.exists() {
                if let Some(w) = app.get_webview_window("widget") {
                    let _ = w.show();
                }
            }

            let open = MenuItem::with_id(app, "open", "Open AgentPulse", true, None::<&str>)?;
            let widget = MenuItem::with_id(app, "widget", "Toggle floating widget", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &widget, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(tray_icon(QUIET))
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
            events::spawn_watcher(app.handle().clone(), events_file, live, approvals, on_alert);
            spawn_monitor(app.handle().clone());
            Ok(())
        })
        // Closing a window hides it; "Quit" in the tray menu exits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                if window.label() == "widget" {
                    remember_widget(window.app_handle(), false);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running AgentPulse");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_is_a_rounded_square_with_the_status_line() {
        let px = tray_icon(WORKING).rgba().to_vec();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0); // rounded corner is transparent
        assert_eq!(at(16, 2)[3], 255); // inside the square
        assert_eq!(&at(8, 17)[..3], &WORKING); // on the flat part of the pulse line
    }

    #[test]
    fn tray_status_counts_everything_that_runs() {
        let now = Timestamp::now();
        let l = |state| events::Live { state, since: now, project: "p".into(), message: String::new() };
        let live = HashMap::from([("a".to_string(), l(State::NeedsYou)), ("gone".to_string(), l(State::Ended))]);
        let logs = HashMap::from([("cx".to_string(), stats::LogState { state: "working", provider: Provider::Codex, project: "p".into() })]);
        let (color, tip) = tray_status(&live, &logs, &["s".into(), "gone".into()]);
        assert_eq!((color, tip.as_str()), (NEEDS_YOU, "AgentPulse: 1 needs you, 1 working, 1 sub-agent running"));
        assert_eq!(tray_status(&HashMap::new(), &HashMap::new(), &[]), (QUIET, "AgentPulse: all quiet".to_string()));
        assert_eq!(tray_status(&HashMap::new(), &HashMap::new(), &["s".into()]).0, WORKING);
    }
}
