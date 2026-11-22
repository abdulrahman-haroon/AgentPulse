//! Connect/disconnect AgentPulse's hook in Claude Code's `settings.json`.
//! The JSON edits are pure functions (tested); file I/O backs up, then writes atomically.
//! Only entries whose command runs our `notify.ps1` are ever added or removed.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const HOOK_EVENTS: [&str; 6] = ["SessionStart", "UserPromptSubmit", "Notification", "PermissionRequest", "Stop", "SessionEnd"];

/// Seconds Claude Code lets the PermissionRequest hook run. Longer than the longest wait the
/// script allows (settings::MAX_APPROVAL_WAIT) so the script, not a kill, ends the wait.
pub const PERMISSION_HOOK_TIMEOUT: u64 = crate::settings::MAX_APPROVAL_WAIT + 30;

/// The hook script, compiled into the app so installs don't depend on where the repo lives.
pub const SCRIPT: &str = include_str!("../../hooks/notify.ps1");

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct HookStatus {
    pub installed: Vec<String>,
    pub missing: Vec<String>,
}

/// True for a hook command that runs AgentPulse's script (installed copy or repo checkout).
fn is_ours(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.contains("notify.ps1") && c.contains("agentpulse")
}

pub fn command_for(script: &Path) -> String {
    // Forward slashes: valid on Windows and no JSON/PowerShell backslash surprises.
    let p = script.to_string_lossy().replace('\\', "/");
    format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{p}\"")
}

pub fn status(settings: &Value) -> HookStatus {
    let (installed, missing) = HOOK_EVENTS.iter().map(|e| e.to_string()).partition(|e| {
        settings["hooks"][e]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|g| g["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(is_ours)))))
    });
    HookStatus { installed, missing }
}

/// Removes our entries from every event; drops groups/events/`hooks` left empty by that.
pub fn disconnect(mut settings: Value) -> Result<Value, String> {
    let Some(hooks) = settings.get_mut("hooks") else { return Ok(settings) };
    let hooks = hooks.as_object_mut().ok_or("`hooks` in settings.json isn't an object; not touching it.")?;
    for event in HOOK_EVENTS {
        let Some(groups) = hooks.get_mut(event) else { continue };
        let groups = groups.as_array_mut().ok_or_else(|| format!("`hooks.{event}` isn't a list; not touching it."))?;
        for g in groups.iter_mut() {
            if let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) {
                hs.retain(|h| !h["command"].as_str().is_some_and(is_ours));
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|hs| !hs.is_empty()));
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        settings.as_object_mut().map(|o| o.remove("hooks"));
    }
    Ok(settings)
}

/// Adds exactly one AgentPulse entry per event (replacing older ones), keeping everything else.
pub fn connect(settings: Value, command: &str) -> Result<Value, String> {
    if !settings.is_object() {
        return Err("settings.json isn't a JSON object; not touching it.".into());
    }
    let mut settings = disconnect(settings)?;
    let root = settings.as_object_mut().expect("checked above");
    let hooks = root.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or("`hooks` in settings.json isn't an object; not touching it.")?;
    for event in HOOK_EVENTS {
        let mut hook = json!({ "type": "command", "command": command });
        if event == "PermissionRequest" {
            hook["timeout"] = json!(PERMISSION_HOOK_TIMEOUT);
        }
        let groups = hooks.entry(event).or_insert_with(|| json!([]));
        groups
            .as_array_mut()
            .ok_or_else(|| format!("`hooks.{event}` isn't a list; not touching it."))?
            .push(json!({ "hooks": [hook] }));
    }
    Ok(settings)
}

pub fn read_settings(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(json!({})),
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("Couldn't parse settings.json ({e}); not touching it.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

const BACKUP_PREFIX: &str = "settings.json.agentpulse-backup-";

/// Keeps the oldest backup (the user's settings before AgentPulse ever touched them) and the
/// newest one; deletes the ones in between so clicks don't pile up files in ~/.claude.
fn prune_backups(settings: &Path) {
    let Some(dir) = settings.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut backups: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(BACKUP_PREFIX)))
        .collect();
    backups.sort();
    if backups.len() > 2 {
        for p in &backups[1..backups.len() - 1] {
            let _ = fs::remove_file(p);
        }
    }
}

/// Backs up the current file (if any), then writes via a temp file + rename.
/// Returns the backup path.
pub fn write_settings(path: &Path, value: &Value) -> Result<Option<PathBuf>, String> {
    let backup = if path.exists() {
        // Millisecond names sort by time and can't overwrite each other on quick clicks.
        let stamp = jiff::ZonedDateTime::now().strftime("%Y%m%d-%H%M%S-%3f");
        let b = path.with_file_name(format!("{BACKUP_PREFIX}{stamp}"));
        fs::copy(path, &b).map_err(|e| format!("Couldn't back up settings.json: {e}"))?;
        prune_backups(path);
        Some(b)
    } else {
        None
    };
    let tmp = path.with_file_name("settings.json.agentpulse-tmp");
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n";
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = r#"powershell -NoProfile -ExecutionPolicy Bypass -File "C:/x/dev.agentpulse.app/hooks/notify.ps1""#;

    #[test]
    fn connect_keeps_other_settings_and_hooks_in_order() {
        let before = json!({
            "model": "opus",
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "my-own-script.sh" }] }] },
            "theme": "dark"
        });
        let after = connect(before, CMD).unwrap();
        // key order preserved (serde_json preserve_order)
        assert_eq!(after.as_object().unwrap().keys().collect::<Vec<_>>(), ["model", "hooks", "theme"]);
        assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["command"], "my-own-script.sh");
        assert_eq!(after["hooks"]["Stop"][1]["hooks"][0]["command"], CMD);
        assert_eq!(status(&after).missing, Vec::<String>::new());
    }

    #[test]
    fn connect_is_idempotent_and_replaces_old_paths() {
        let old = r#"powershell -File "C:/code/AgentPulse/hooks/notify.ps1""#;
        let s = connect(json!({}), old).unwrap();
        let s = connect(s, CMD).unwrap();
        let s = connect(s, CMD).unwrap();
        for e in HOOK_EVENTS {
            let groups = s["hooks"][e].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{e}");
            assert_eq!(groups[0]["hooks"][0]["command"], CMD);
        }
        assert_eq!(s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 330);
        assert!(s["hooks"]["Stop"][0]["hooks"][0].get("timeout").is_none());
    }

    #[test]
    fn disconnect_removes_only_ours() {
        let mixed = json!({ "hooks": {
            "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }, { "type": "command", "command": CMD }] }],
            "Notification": [{ "hooks": [{ "type": "command", "command": CMD }] }]
        }});
        let s = disconnect(mixed).unwrap();
        assert_eq!(s, json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] } }));
        assert_eq!(disconnect(connect(json!({"a": 1}), CMD).unwrap()).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn refuses_unexpected_shapes() {
        assert!(connect(json!([]), CMD).is_err());
        assert!(connect(json!({ "hooks": "nope" }), CMD).is_err());
        assert!(connect(json!({ "hooks": { "Stop": {} } }), CMD).is_err());
    }

    #[test]
    fn file_roundtrip_backs_up_and_writes() {
        let dir = std::env::temp_dir().join(format!("agentpulse-hooks-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        assert_eq!(read_settings(&p).unwrap(), json!({})); // missing file = empty
        fs::write(&p, r#"{"model":"opus"}"#).unwrap();
        let backup = write_settings(&p, &connect(read_settings(&p).unwrap(), CMD).unwrap()).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), r#"{"model":"opus"}"#);
        assert_eq!(read_settings(&p).unwrap()["model"], "opus");
        // Several quick saves: the original is never lost, and only 2 backups remain.
        for _ in 0..4 {
            write_settings(&p, &read_settings(&p).unwrap()).unwrap();
        }
        let mut backups: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains(BACKUP_PREFIX)).collect();
        backups.sort();
        assert_eq!(backups.len(), 2);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), r#"{"model":"opus"}"#);

        fs::write(&p, "{ broken").unwrap();
        assert!(read_settings(&p).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
