fn main() {
    // Declaring app commands makes them deny-by-default: a window can only call
    // them if a capability file grants `allow-<command>`.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "dashboard",
        "hook_status",
        "connect_hooks",
        "disconnect_hooks",
        "get_autostart",
        "set_autostart",
        "app_version",
        "check_update",
        "install_update",
        "get_settings",
        "save_settings",
        "test_push",
        "test_sound",
        "decide_permission",
        "session_history",
        "toggle_widget",
        "open_dashboard",
        "open_session",
        "send_reply",
        "cancel_reply",
        "dismiss_reply",
    ])))
    .expect("failed to run tauri-build");
}
