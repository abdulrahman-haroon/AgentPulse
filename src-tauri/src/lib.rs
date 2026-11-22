mod events;
mod pricing;
mod usage;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

/// Today's token usage and estimated cost, summed from local Claude Code logs.
#[tauri::command]
async fn today_usage(app: AppHandle) -> Result<usage::DaySummary, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let dir = usage::claude_dir(&home);
    tauri::async_runtime::spawn_blocking(move || usage::today(&dir))
        .await
        .map_err(|e| e.to_string())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![today_usage])
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open AgentPulse", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("bundle has an icon"))
                .tooltip("AgentPulse")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
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
            events::spawn_watcher(app.handle().clone(), events_file);
            Ok(())
        })
        // Closing the window hides it to the tray; "Quit" in the tray menu exits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running AgentPulse");
}
