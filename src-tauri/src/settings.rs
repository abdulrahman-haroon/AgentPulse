//! AgentPulse's own settings (`<app data>/settings.json`). Validated on every save;
//! a missing or unreadable file falls back to defaults (push stays off).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PushSettings {
    /// Off by default: this is the only feature that sends data off the machine.
    pub enabled: bool,
    /// ntfy server; https only.
    pub server: String,
    /// On ntfy.sh, anyone who knows the topic can read it, so it must be long and random.
    pub topic: String,
    pub on_done: bool,
    pub on_needs_you: bool,
    /// Include project name and Claude's question. Off = generic text only.
    pub include_details: bool,
}

impl Default for PushSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            server: "https://ntfy.sh".into(),
            topic: String::new(),
            on_done: true,
            on_needs_you: true,
            include_details: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ApprovalSettings {
    /// Off by default. When on, Claude Code's permission prompts appear here first.
    pub enabled: bool,
    /// How long the hook waits for an answer here before Claude Code shows its own prompt.
    pub wait_seconds: u64,
}

impl Default for ApprovalSettings {
    fn default() -> Self {
        Self { enabled: false, wait_seconds: 60 }
    }
}

/// Kept below the hook timeout registered in settings.json (see hooks::PERMISSION_HOOK_TIMEOUT).
pub const MAX_APPROVAL_WAIT: u64 = 300;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SoundSettings {
    /// On by default: an alert you can't hear is easy to miss (app toasts are silent).
    pub enabled: bool,
    pub on_done: bool,
    pub on_needs_you: bool,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self { enabled: true, on_done: true, on_needs_you: true }
    }
}

impl SoundSettings {
    pub fn wanted(&self, kind: crate::events::AlertKind) -> bool {
        self.enabled && if kind == crate::events::AlertKind::Done { self.on_done } else { self.on_needs_you }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub push: PushSettings,
    pub approvals: ApprovalSettings,
    pub sound: SoundSettings,
}

pub fn validate_push(p: &PushSettings) -> Result<(), String> {
    let t = &p.topic;
    if t.len() < 12 || t.len() > 64 || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Topic must be 12–64 letters, digits, '-' or '_'. Use the generated one: a short topic can be guessed and read by anyone.".into());
    }
    let url = reqwest::Url::parse(&p.server).map_err(|_| "Server must be a valid URL.".to_string())?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Server must be a plain https:// address, e.g. https://ntfy.sh".into());
    }
    Ok(())
}

pub fn validate(s: &Settings) -> Result<(), String> {
    if s.push.enabled {
        validate_push(&s.push)?;
    }
    if !(15..=MAX_APPROVAL_WAIT).contains(&s.approvals.wait_seconds) {
        return Err(format!("Approval wait must be between 15 and {MAX_APPROVAL_WAIT} seconds."));
    }
    Ok(())
}

pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
        .filter(|s| validate(s).is_ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, s: &Settings) -> Result<(), String> {
    validate(s)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(server: &str, topic: &str) -> PushSettings {
        PushSettings { enabled: true, server: server.into(), topic: topic.into(), ..Default::default() }
    }

    #[test]
    fn validates_topic_and_server() {
        assert!(validate_push(&push("https://ntfy.sh", "agentpulse-k3J9xQ2mPz7a")).is_ok());
        assert!(validate_push(&push("https://ntfy.example.com/", "agentpulse-k3J9xQ2mPz7a")).is_ok());
        assert!(validate_push(&push("https://ntfy.sh", "test")).is_err()); // guessable
        assert!(validate_push(&push("https://ntfy.sh", "agentpulse k3J9xQ2mPz7a")).is_err());
        assert!(validate_push(&push("http://ntfy.sh", "agentpulse-k3J9xQ2mPz7a")).is_err());
        assert!(validate_push(&push("https://user:pw@ntfy.sh", "agentpulse-k3J9xQ2mPz7a")).is_err());
        assert!(validate_push(&push("https://ntfy.sh?x=1", "agentpulse-k3J9xQ2mPz7a")).is_err());
        assert!(validate_push(&push("file:///etc/passwd", "agentpulse-k3J9xQ2mPz7a")).is_err());
    }

    #[test]
    fn sound_defaults_on_for_older_settings_files() {
        use crate::events::AlertKind;
        let s: Settings = serde_json::from_str(r#"{"push":{"enabled":false}}"#).unwrap();
        assert!(s.sound.wanted(AlertKind::Done) && s.sound.wanted(AlertKind::NeedsYou));
        let quiet = SoundSettings { on_done: false, ..Default::default() };
        assert!(!quiet.wanted(AlertKind::Done) && quiet.wanted(AlertKind::NeedsYou));
        assert!(!SoundSettings { enabled: false, ..Default::default() }.wanted(AlertKind::NeedsYou));
    }

    #[test]
    fn disabled_push_needs_no_topic_and_bad_files_fall_back() {
        assert!(validate(&Settings::default()).is_ok());
        let dir = std::env::temp_dir().join(format!("agentpulse-settings-{}", std::process::id()));
        let p = dir.join("settings.json");
        assert_eq!(load(&p), Settings::default()); // missing
        let mut s = Settings::default();
        s.push = push("https://ntfy.sh", "agentpulse-k3J9xQ2mPz7a");
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
        fs::write(&p, r#"{"push":{"enabled":true,"topic":"x"}}"#).unwrap(); // invalid → defaults
        assert_eq!(load(&p), Settings::default());
        assert!(save(&p, &Settings { push: push("http://x", "short"), ..Default::default() }).is_err());
        let mut too_long = Settings::default();
        too_long.approvals.wait_seconds = 3600;
        assert!(save(&p, &too_long).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
