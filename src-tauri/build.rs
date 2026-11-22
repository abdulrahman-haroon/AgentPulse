fn main() {
    // Declaring app commands makes them deny-by-default: a window can only call
    // them if a capability file grants `allow-<command>`.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["today_usage"])),
    )
    .expect("failed to run tauri-build");
}
