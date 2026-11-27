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

/// What AgentPulse knows about the alert's session, from its own log index (filled in by lib.rs).
#[derive(Debug, Default, Clone)]
pub struct Details {
    /// "Claude Code", "Copilot", "Codex". Always sent: it says which app to look at, not what you work on.
    pub agent: String,
    pub session_title: String,
    pub last_prompt: String,
    pub model: String,
    /// Session cost so far, when the agent is billed per token.
    pub cost: Option<f64>,
}

/// "45s", "12m", "1h 5m"
fn took(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// The JSON body for ntfy's publish API. Pure, so the privacy rules are testable: without
/// `include_details` only the agent's name and the kind of event leave the machine.
pub fn payload(p: &PushSettings, a: &Alert, d: &Details) -> Value {
    let agent = if d.agent.is_empty() { "Claude Code" } else { &d.agent };
    let done = a.kind == AlertKind::Done;
    let (title, message) = if !p.include_details {
        let title = format!("{agent} {}", if done { "finished" } else { "needs you" });
        let message = if done { "A task is done. Open AgentPulse for details." } else { "A session is waiting for your answer." };
        (title, message.to_string())
    } else {
        let title = format!("{agent} {} · {}", if done { "finished" } else { "needs you" }, a.project);
        let mut lines = Vec::new();
        if !d.session_title.is_empty() {
            lines.push(d.session_title.clone());
        }
        if done {
            if !d.last_prompt.is_empty() {
                lines.push(format!("You asked: {}", crate::logs::one_line(&d.last_prompt, 160)));
            }
        } else if !a.body.is_empty() {
            lines.push(a.body.clone()); // the agent's question / the tool it wants to use
        }
        let facts: Vec<String> = [
            a.took_secs.filter(|_| done).map(|s| format!("Worked {}", took(s))),
            d.cost.filter(|c| *c >= 0.005).map(|c| format!("${c:.2} so far")),
            Some(d.model.trim_start_matches("claude-").to_string()).filter(|m| !m.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !facts.is_empty() {
            lines.push(facts.join(" · "));
        }
        (title, if lines.is_empty() { "—".to_string() } else { lines.join("\n") })
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
        Alert { kind, project: "secret-project".into(), title: "Claude needs you: secret-project".into(), body: "Allow rm -rf build?".into(), session: String::new(), took_secs: None }
    }

    fn settings(include_details: bool) -> PushSettings {
        PushSettings { enabled: true, topic: "agentpulse-k3J9xQ2mPz7a".into(), include_details, ..Default::default() }
    }

    fn details() -> Details {
        Details {
            agent: "Copilot".into(),
            session_title: "Secret title".into(),
            last_prompt: "secret prompt".into(),
            model: "claude-opus-5-5".into(),
            cost: Some(2.784),
        }
    }

    #[test]
    fn generic_by_default_never_leaks_project_or_question() {
        let body = payload(&settings(false), &alert(AlertKind::NeedsYou), &details()).to_string();
        for secret in ["secret-project", "rm -rf", "Secret title", "secret prompt", "opus", "2.78"] {
            assert!(!body.contains(secret), "{secret} leaked: {body}");
        }
        assert!(body.contains("\"priority\":4"));
        assert!(body.contains("Copilot needs you")); // the right app, not always "Claude"
    }

    #[test]
    fn details_only_when_opted_in() {
        let body = payload(&settings(true), &alert(AlertKind::NeedsYou), &details());
        assert_eq!(body["title"], "Copilot needs you · secret-project");
        assert_eq!(body["message"], "Secret title\nAllow rm -rf build?\n$2.78 so far · opus-5-5");
        let mut done = alert(AlertKind::Done);
        done.took_secs = Some(754);
        let body = payload(&settings(true), &done, &details());
        assert_eq!(body["message"], "Secret title\nYou asked: secret prompt\nWorked 12m · $2.78 so far · opus-5-5");
        // nothing known: still a valid message
        assert_eq!(payload(&settings(true), &done, &Details::default())["message"], "Worked 12m");
        assert_eq!(took(45), "45s");
        assert_eq!(took(3_900), "1h 5m");
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
