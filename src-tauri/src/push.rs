//! Phone notifications through ntfy (https://ntfy.sh or a self-hosted server).
//! Opt-in; by default messages are generic so nothing about your projects leaves the machine.

use crate::events::{Alert, AlertKind};
use crate::settings::PushSettings;
use serde_json::{json, Value};

/// Whether this alert should go to the phone, given the user's choices.
pub fn wanted(p: &PushSettings, a: &Alert) -> bool {
    p.enabled
        && match a.kind {
            AlertKind::Done => p.on_done,
            AlertKind::NeedsYou => p.on_needs_you,
        }
}

/// The JSON body for ntfy's publish API. Pure, so the privacy rules are testable.
pub fn payload(p: &PushSettings, a: &Alert) -> Value {
    let (title, message) = match (a.kind, p.include_details) {
        (AlertKind::Done, false) => ("Claude finished".to_string(), "A Claude Code task is done.".to_string()),
        (AlertKind::NeedsYou, false) => ("Claude needs you".to_string(), "A Claude Code session is waiting for your approval.".to_string()),
        (_, true) => (a.title.clone(), if a.body.is_empty() { "—".to_string() } else { a.body.clone() }),
    };
    json!({
        "topic": p.topic,
        "title": title,
        "message": message,
        "tags": [if a.kind == AlertKind::Done { "white_check_mark" } else { "raising_hand" }],
        "priority": if a.kind == AlertKind::NeedsYou { 4 } else { 3 },
    })
}

/// POSTs to the server root (ntfy's JSON publish endpoint).
pub async fn send(p: &PushSettings, body: &Value) -> Result<(), String> {
    // reqwest is built without a default TLS provider (the updater only installs one when it
    // runs), so make sure one exists before the first HTTPS call.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.post(&p.server).json(body).send().await.map_err(|e| format!("Couldn't reach {}: {e}", p.server))?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("{} answered {}", p.server, res.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(kind: AlertKind) -> Alert {
        Alert { kind, project: "secret-project".into(), title: "Claude needs you: secret-project".into(), body: "Allow rm -rf build?".into(), session: String::new() }
    }

    fn settings(include_details: bool) -> PushSettings {
        PushSettings { enabled: true, topic: "agentpulse-k3J9xQ2mPz7a".into(), include_details, ..Default::default() }
    }

    #[test]
    fn generic_by_default_never_leaks_project_or_question() {
        let body = payload(&settings(false), &alert(AlertKind::NeedsYou)).to_string();
        assert!(!body.contains("secret-project") && !body.contains("rm -rf"), "{body}");
        assert!(body.contains("\"priority\":4"));
    }

    #[test]
    fn details_only_when_opted_in() {
        let body = payload(&settings(true), &alert(AlertKind::NeedsYou)).to_string();
        assert!(body.contains("secret-project") && body.contains("rm -rf"));
    }

    /// Builds the real HTTPS client and fails fast against an unreachable address,
    /// proving TLS setup works without the updater having run. No data leaves the machine.
    #[test]
    fn https_client_builds_without_updater() {
        let rt = tauri::async_runtime::block_on(async {
            let p = PushSettings { enabled: true, server: "https://127.0.0.1:9".into(), topic: "agentpulse-k3J9xQ2mPz7a".into(), ..Default::default() };
            send(&p, &json!({})).await
        });
        let err = rt.unwrap_err();
        assert!(err.starts_with("Couldn't reach"), "{err}"); // a connect error, not a TLS/provider panic
    }

    #[test]
    fn respects_event_choices() {
        let mut s = settings(false);
        assert!(wanted(&s, &alert(AlertKind::Done)));
        s.on_done = false;
        assert!(!wanted(&s, &alert(AlertKind::Done)));
        assert!(wanted(&s, &alert(AlertKind::NeedsYou)));
        s.enabled = false;
        assert!(!wanted(&s, &alert(AlertKind::NeedsYou)));
    }
}
