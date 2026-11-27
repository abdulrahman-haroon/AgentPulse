mod actions;
mod codex;
mod copilot;
mod events;
mod hooks;
mod limits;
mod logs;
mod pricing;
mod push;
mod settings;
mod sound;
mod stats;

use events::{AlertKind, LiveStore, State};
use jiff::{SignedDuration, Timestamp, ZonedDateTime};
use logs::Provider;
use std::collections::{HashMap, HashSet};
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
    /// Claude Code's plan-limit numbers, saved by the status-line wrapper.
    claude_limits_path: PathBuf,
    settings: Mutex<settings::Settings>,
    settings_path: PathBuf,
    approvals: events::Approvals,
    /// Where answers to permission prompts are dropped for the waiting hook script.
    decisions_dir: PathBuf,
    /// Exists while the floating widget is on, so it comes back after a restart.
    widget_flag: PathBuf,
    /// Sessions that finished and the user hasn't checked yet: the tray's badge dot.
    unchecked: Mutex<HashSet<String>>,
}

/// The user looked at this session (opened it from AgentPulse).
fn mark_checked(state: &AppState, id: &str) {
    if let Ok(mut u) = state.unchecked.lock() {
        u.remove(id);
    }
}

/// Widget's "N new" pill: everything counts as checked.
#[tauri::command]
fn mark_all_checked(state: TauriState<'_, AppState>) {
    if let Ok(mut u) = state.unchecked.lock() {
        u.clear();
    }
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

/// Sends an example "finished" push with the given (unsaved) settings, so the user sees on their
/// phone exactly what real alerts will look like (generic or detailed). Example data, not theirs.
#[tauri::command]
async fn test_push(push: settings::PushSettings) -> Result<(), String> {
    settings::validate_push(&push)?;
    let sample = events::Alert {
        kind: AlertKind::Done,
        project: "example-project".into(),
        title: String::new(),
        body: String::new(),
        session: String::new(),
        took_secs: Some(754),
    };
    let details = push::Details {
        agent: "Claude Code".into(),
        session_title: "Add dark mode toggle".into(),
        last_prompt: "add a dark mode toggle to the settings page and remember the choice".into(),
        model: "claude-opus-5-5".into(),
        cost: Some(2.78),
    };
    let mut body = push::payload(&push, &sample, &details);
    body["title"] = format!("Test: {}", body["title"].as_str().unwrap_or_default()).into();
    body["message"] = format!("{}\n(example from AgentPulse, it's working)", body["message"].as_str().unwrap_or_default()).into();
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

/// What our own log index knows about a session, for the phone message. Hook-only sessions
/// (no log yet) are Claude Code with nothing else known.
fn push_details(app: &AppHandle, session: &str) -> push::Details {
    let state = app.state::<AppState>();
    let Ok(cache) = state.cache.lock() else { return push::Details::default() };
    let meta = cache.meta();
    let m = meta.get(session);
    let costs: Vec<f64> = cache.requests().filter(|r| r.session == session).filter_map(|r| r.cost).collect();
    push::Details {
        agent: m.map_or(Provider::Claude, |m| m.provider).label().to_string(),
        session_title: cache.titles().get(session).map(|t| t.to_string()).unwrap_or_default(),
        last_prompt: m.map(|m| m.last_prompt.clone()).unwrap_or_default(),
        model: m.and_then(|m| m.model.clone()).filter(|x| !x.starts_with('<')).unwrap_or_default(),
        cost: (!costs.is_empty()).then(|| costs.iter().sum()),
    }
}

/// Desktop notification and chime for every alert; phone push if the user turned it on.
fn on_alert(app: &AppHandle, alert: &events::Alert) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(&alert.title).body(&alert.body).show();
    // Already looking at the dashboard? Then it's seen, no badge.
    let looking = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if alert.kind == AlertKind::Done && !alert.session.is_empty() && !looking {
        if let Ok(mut u) = app.state::<AppState>().unchecked.lock() {
            u.insert(alert.session.clone());
        }
    }
    let s = app.state::<AppState>().settings.lock().map(|s| s.clone()).unwrap_or_default();
    if s.sound.wanted(alert.kind) {
        sound::play(alert.kind);
    }
    if push::wanted(&s.push, alert) {
        let body = push::payload(&s.push, alert, &push_details(app, &alert.session));
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
        mark_checked(s, &id);
        actions::open_session(&id, &m.cwd, &m.entrypoint, running)
    })
    .await
}

/// Brings up the session's project in VS Code, for any agent (widget row click, Codex/Copilot
/// cards). The folder comes from our log index, never from the UI.
#[tauri::command]
async fn focus_session(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |s| {
        let m = s.cache.lock().map_err(|e| e.to_string())?.meta().get(id.as_str()).map(|m| (*m).clone()).ok_or("Unknown session.")?;
        mark_checked(s, &id);
        // Claude Code sessions that live in VS Code open their chat; the rest open their project.
        let in_extension = m.provider == Provider::Claude && m.entrypoint.contains("vscode");
        actions::open_in_vscode(&id, &m.cwd, in_extension)
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
        if let Ok(snap) = std::fs::read_to_string(&state.claude_limits_path) {
            d.limits.extend(limits::claude(&snap, now.timestamp()));
        }
        let allowance = state.settings.lock().ok().and_then(|s| s.limits.copilot_allowance());
        if let Some(l) = allowance.and_then(|a| limits::copilot(cache.requests(), a, now.timestamp())) {
            d.limits.push(l);
        }
        d.sound_enabled = state.settings.lock().map(|s| s.sound.enabled).unwrap_or(true);
        d.unchecked = state.unchecked.lock().map(|u| u.iter().cloned().collect()).unwrap_or_default();
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

/// Installs the hook script into AgentPulse's data folder, registers it for all six events and
/// wraps the Claude Code status line (for plan limits) in settings.json (backed up first).
/// Only our own entries are touched; the user's own status line is saved and still runs.
#[tauri::command]
async fn connect_hooks(app: AppHandle) -> Result<HookInfo, String> {
    blocking(app, |s| {
        install_script(&s.script_path)?;
        let path = s.claude_dir.join("settings.json");
        let updated = hooks::connect(hooks::read_settings(&path)?, &hooks::command_for(&s.script_path))?;
        let (updated, saved) = hooks::wrap_status_line(updated, &hooks::command_for(&statusline_path(&s.script_path)));
        if let hooks::SavedStatusLine::Save(original) = saved {
            // Saved before settings.json points at the wrapper, so the user's line never goes missing.
            let text = serde_json::to_string_pretty(&original.unwrap_or(serde_json::Value::Null)).map_err(|e| e.to_string())?;
            write_atomic(&original_status_line_path(&s.script_path), &text)?;
        }
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
        let saved_path = original_status_line_path(&s.script_path);
        let original = std::fs::read_to_string(&saved_path).ok().and_then(|t| serde_json::from_str(&t).ok());
        let updated = hooks::restore_status_line(hooks::disconnect(hooks::read_settings(&path)?)?, original);
        let backup = hooks::write_settings(&path, &updated)?;
        let _ = std::fs::remove_file(&saved_path);
        hook_info(s, backup)
    })
    .await
}

fn statusline_path(script: &std::path::Path) -> PathBuf {
    script.with_file_name("statusline.ps1")
}

/// The user's own `statusLine` object (or `null`), run by the wrapper and restored on disconnect.
fn original_status_line_path(script: &std::path::Path) -> PathBuf {
    script.with_file_name("statusline-original.json")
}

fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Installs (or refreshes) both scripts next to each other.
fn install_script(path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, hooks::SCRIPT).map_err(|e| format!("Couldn't install the hook script: {e}"))?;
    std::fs::write(statusline_path(path), hooks::STATUSLINE_SCRIPT).map_err(|e| format!("Couldn't install the status-line script: {e}"))
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
/// status color, plus a green "not checked yet" dot cut into the top-right corner. Drawn here, so
/// no image files or decoders are needed.
fn tray_icon(line: [u8; 3], badge: bool) -> Image<'static> {
    const N: usize = 32;
    const BG: [u8; 3] = [0x15, 0x18, 0x2b];
    const DOT: [u8; 3] = [0x34, 0xd3, 0x99];
    const DOT_AT: (f32, f32) = (25.5, 6.5);
    // The logo's pulse, in tray pixels (same shape as icons/icon-source.svg).
    const PTS: [(f32, f32); 6] = [(5.5, 17.5), (11.0, 17.5), (13.5, 9.0), (18.5, 24.0), (21.0, 17.5), (26.5, 17.5)];
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square, corner radius 8, 1px anti-aliased edge.
            let (qx, qy) = ((cx - 16.0).abs() - 8.0, (cy - 16.0).abs() - 8.0);
            let edge = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - 8.0;
            let dist = PTS.windows(2).map(|w| seg_dist(cx, cy, w[0], w[1])).fold(f32::MAX, f32::min);
            let ink = (2.25 - dist).clamp(0.0, 1.0); // ~3.5px stroke
            // Badge: a 5px-radius dot with a 2px transparent gap around it, like a phone badge.
            let d = (cx - DOT_AT.0).hypot(cy - DOT_AT.1);
            let (dot, gap) = if badge { ((5.5 - d).clamp(0.0, 1.0), (7.5 - d).clamp(0.0, 1.0)) } else { (0.0, 0.0) };
            let tile = (0.5 - edge).clamp(0.0, 1.0) * (1.0 - gap);
            let c = |i: usize| {
                let base = BG[i] as f32 * (1.0 - ink) + line[i] as f32 * ink;
                (base * (1.0 - dot) + DOT[i] as f32 * dot) as u8
            };
            px.extend_from_slice(&[c(0), c(1), c(2), (tile.max(dot) * 255.0) as u8]);
        }
    }
    Image::new_owned(px, N as u32, N as u32)
}

const QUIET: [u8; 3] = [0x8b, 0x93, 0xa7];
const WORKING: [u8; 3] = [0x38, 0xbd, 0xf8];
const NEEDS_YOU: [u8; 3] = [0xfb, 0xbf, 0x24];

/// What the tray shows: line color, tooltip, and how many finished sessions aren't checked yet.
type TrayLook = ([u8; 3], String, usize);

/// 3×5 pixel glyphs (rows top to bottom, 3 bits each) for the taskbar badge count.
fn glyph(c: char) -> u16 {
    match c {
        '1' => 0b010_110_010_010_111,
        '2' => 0b111_001_111_100_111,
        '3' => 0b111_001_111_001_111,
        '4' => 0b101_101_111_001_001,
        '5' => 0b111_100_111_001_111,
        '6' => 0b111_100_111_101_111,
        '7' => 0b111_001_001_001_001,
        '8' => 0b111_101_111_101_111,
        '9' => 0b111_101_111_001_111,
        _ => 0b000_010_111_010_000, // '+'
    }
}

/// 32×32 taskbar overlay: a green circle with the unchecked count in white ("9+" past nine),
/// shown in the corner of AgentPulse's taskbar button like a chat app's badge.
fn badge_icon(count: usize) -> Image<'static> {
    const N: usize = 32;
    const GREEN: [u8; 3] = [0x05, 0x96, 0x69]; // darker than the tray dot, so white digits stay readable
    let text: Vec<char> = if count > 9 { vec!['9', '+'] } else { count.to_string().chars().collect() };
    let scale = if text.len() == 1 { 4 } else { 3 };
    let width = text.len() * 3 * scale + (text.len() - 1) * scale;
    let (x0, y0) = ((N - width) / 2, (N - 5 * scale) / 2);
    let lit = |x: usize, y: usize| -> bool {
        let (Some(dx), Some(dy)) = (x.checked_sub(x0), y.checked_sub(y0)) else { return false };
        let (gx, gy) = (dx / scale, dy / scale);
        let (i, col) = (gx / 4, gx % 4); // 3 columns + 1 gap per glyph
        match text.get(i) {
            Some(&c) if col < 3 && gy < 5 => (glyph(c) >> ((4 - gy) * 3 + (2 - col))) & 1 == 1,
            _ => false,
        }
    };
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let d = (x as f32 + 0.5 - 16.0).hypot(y as f32 + 0.5 - 16.0);
            let a = ((16.0 - d).clamp(0.0, 1.0) * 255.0) as u8;
            let rgb = if lit(x, y) { [255, 255, 255] } else { GREEN };
            px.extend_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
        }
    }
    Image::new_owned(px, N as u32, N as u32)
}

/// Tray look from hook states, log-only agents, running sub-agents and unchecked finished sessions.
fn tray_status(live: &HashMap<String, events::Live>, logs: &HashMap<String, stats::LogState>, agent_sessions: &[String], unchecked: usize) -> TrayLook {
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
    if unchecked > 0 {
        parts.push(format!("{unchecked} finished, not checked yet"));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if agents > 0 {
        parts.push(format!("{agents} sub-agent{} running", if agents == 1 { "" } else { "s" }));
    }
    let tip = if parts.is_empty() { "AgentPulse: all quiet".to_string() } else { format!("AgentPulse: {}", parts.join(", ")) };
    let color = if needs > 0 { NEEDS_YOU } else if working + agents > 0 { WORKING } else { QUIET };
    (color, tip, unchecked)
}

/// Every 2 s: refresh all agent logs (agents without hooks only show up there), alert when a
/// log-only session finishes its turn, and keep the tray light and badge current. The first pass
/// also parses the whole history, so the first dashboard call is fast.
fn spawn_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut before: Option<HashMap<String, stats::LogState>> = None;
        // Plus whether the dashboard was visible: Windows drops a taskbar overlay when the window
        // hides, so it's set again when the window comes back.
        let mut shown: Option<(TrayLook, bool)> = None;
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
                    for (kind, id, s) in stats::log_alerts(b, &logs) {
                        let (what, body) = match kind {
                            AlertKind::Done => ("finished", "Task complete."),
                            AlertKind::NeedsYou => ("needs you", "Waiting for your approval."),
                        };
                        let title = format!("{} {what}: {}", s.provider.label(), s.project);
                        on_alert(&app, &events::Alert { kind, project: s.project.clone(), title, body: body.into(), session: id.to_string(), took_secs: None });
                    }
                }
                let live = state.live.lock().map(|l| l.clone()).unwrap_or_default();
                let unchecked = state
                    .unchecked
                    .lock()
                    .map(|mut u| {
                        u.retain(|id| stats::still_unchecked(live.get(id).map(|l| l.state), logs.get(id).map(|s| s.state)));
                        u.len()
                    })
                    .unwrap_or(0);
                let look = tray_status(&live, &logs, &agents, unchecked);
                let main = app.get_webview_window("main");
                let visible = main.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false));
                let now_shown = (look, visible);
                if shown.as_ref() != Some(&now_shown) {
                    let count = now_shown.0 .2;
                    if let Some(tray) = app.tray_by_id("main") {
                        let _ = tray.set_icon(Some(tray_icon(now_shown.0 .0, count > 0)));
                        let _ = tray.set_tooltip(Some(&now_shown.0 .1));
                    }
                    #[cfg(windows)]
                    if let Some(w) = main {
                        let _ = w.set_overlay_icon((count > 0).then(|| badge_icon(count)));
                    }
                    shown = Some(now_shown);
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
            mark_all_checked,
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
                claude_limits_path: data.join("claude-limits.json"),
                settings: Mutex::new(settings::load(&data.join("settings.json"))),
                settings_path: data.join("settings.json"),
                approvals: approvals.clone(),
                decisions_dir: data.join("decisions"),
                widget_flag: widget_flag.clone(),
                unchecked: Mutex::new(HashSet::new()),
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
                .icon(tray_icon(QUIET, false))
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
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
                if window.label() == "widget" {
                    remember_widget(window.app_handle(), false);
                }
            }
            // Opening the dashboard = you've seen the board: the badge goes away.
            WindowEvent::Focused(true) if window.label() == "main" => {
                if let Ok(mut u) = window.app_handle().state::<AppState>().unchecked.lock() {
                    u.clear();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running AgentPulse");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_is_a_rounded_square_with_the_status_line() {
        let px = tray_icon(WORKING, false).rgba().to_vec();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0); // rounded corner is transparent
        assert_eq!(at(16, 2)[3], 255); // inside the square
        assert_eq!(&at(8, 17)[..3], &WORKING); // on the flat part of the pulse line
        assert_eq!(&at(25, 6)[..3], &[0x15, 0x18, 0x2b]); // no badge: plain tile there
    }

    #[test]
    fn unchecked_badge_is_a_green_dot_cut_into_the_corner() {
        let px = tray_icon(QUIET, true).rgba().to_vec();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(at(25, 6), &[0x34, 0xd3, 0x99, 255]); // the dot
        assert_eq!(at(19, 6)[3], 0); // transparent gap between dot and tile
        assert_eq!(&at(8, 17)[..3], &QUIET); // the pulse line is untouched
    }

    #[test]
    fn taskbar_badge_is_a_green_circle_with_the_count() {
        let px = badge_icon(1).rgba().to_vec();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0); // round
        assert_eq!(at(16, 30), &[0x05, 0x96, 0x69, 255]); // green body
        assert_eq!(&at(15, 7)[..3], &[255, 255, 255]); // top of the "1" (middle column, first row)
        assert_eq!(&at(11, 7)[..3], &[0x05, 0x96, 0x69]); // left of it is empty in a "1"
        // past nine: "9+", and it must differ from a single digit
        assert_ne!(badge_icon(12).rgba(), badge_icon(9).rgba());
    }

    #[test]
    fn tray_status_counts_everything_that_runs() {
        let now = Timestamp::now();
        let l = |state| events::Live { state, since: now, project: "p".into(), message: String::new() };
        let live = HashMap::from([("a".to_string(), l(State::NeedsYou)), ("gone".to_string(), l(State::Ended))]);
        let logs = HashMap::from([("cx".to_string(), stats::LogState { state: "working", provider: Provider::Codex, project: "p".into() })]);
        let (color, tip, badge) = tray_status(&live, &logs, &["s".into(), "gone".into()], 2);
        assert_eq!((color, tip.as_str(), badge), (NEEDS_YOU, "AgentPulse: 1 needs you, 2 finished, not checked yet, 1 working, 1 sub-agent running", 2));
        assert_eq!(tray_status(&HashMap::new(), &HashMap::new(), &[], 0), (QUIET, "AgentPulse: all quiet".to_string(), 0));
        assert_eq!(tray_status(&HashMap::new(), &HashMap::new(), &["s".into()], 0).0, WORKING);
    }
}
