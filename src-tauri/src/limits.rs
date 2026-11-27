//! Plan limits that don't come from session logs:
//! - Claude Code: the 5-hour / weekly / spend-limit usage it hands to its status-line command.
//!   Our `statusline.ps1` saves that object to `<app data>/claude-limits.json` (then runs the
//!   user's own status line). Real numbers, as of the last status-line update.
//! - Copilot: AI credits logged this calendar month (UTC) against the plan the user picked.
//!   An estimate: only this computer's Copilot CLI usage is counted.

use crate::logs::{PlanLimit, Provider, Request};
use jiff::{tz::TimeZone, Timestamp, ToSpan};
use serde_json::Value;

/// `{"ts": "...", "rate_limits": {five_hour, seven_day, spend_limit}}` → limits whose window
/// hasn't reset yet. Anything malformed is skipped.
pub fn claude(snapshot: &str, now: Timestamp) -> Vec<PlanLimit> {
    let Ok(v) = serde_json::from_str::<Value>(snapshot) else { return vec![] };
    let as_of = v["ts"].as_str().and_then(|t| t.parse::<Timestamp>().ok()).map(|t| t.to_string());
    let r = &v["rate_limits"];
    let mut out = Vec::new();
    for (key, label) in [("five_hour", "5-hour"), ("seven_day", "Weekly"), ("spend_limit", "Spend")] {
        let w = &r[key];
        let Some(used) = w["used_percentage"].as_f64().filter(|u| u.is_finite()) else { continue };
        let resets = w["resets_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok());
        if resets.is_some_and(|t| t <= now) {
            continue; // a new window started; its usage is unknown until Claude Code reports again
        }
        let window = match (key, w["period"].as_str()) {
            ("spend_limit", Some(p @ ("daily" | "weekly" | "monthly"))) => format!("{}{} spend", p[..1].to_uppercase(), &p[1..]),
            _ => label.to_string(),
        };
        let detail = match (w["used_usd"].as_f64(), w["limit_usd"].as_f64()) {
            (Some(u), Some(l)) if key == "spend_limit" && u.is_finite() && l.is_finite() => Some(format!("${} of ${}", thousands(u), thousands(l))),
            _ => None,
        };
        out.push(PlanLimit {
            provider: Provider::Claude,
            window,
            used_percent: used.clamp(0.0, 100.0),
            resets_at: resets.map(|t| t.to_string()),
            detail,
            as_of: as_of.clone(),
            estimate: false,
        });
    }
    out
}

/// Copilot's monthly AI credits (reset 00:00 UTC on the 1st) used so far vs. the plan allowance.
pub fn copilot<'a>(requests: impl Iterator<Item = &'a Request>, allowance: u64, now: Timestamp) -> Option<PlanLimit> {
    if allowance == 0 {
        return None;
    }
    let today = now.to_zoned_datetime(TimeZone::UTC).date();
    let start = today.first_of_month().to_zoned_datetime(TimeZone::UTC).ok()?;
    let next = start.checked_add(1.month()).ok()?;
    let (start, next) = (start.timestamp(), next.timestamp());
    let used: f64 = requests.filter(|r| r.provider == Provider::Copilot && r.ts >= start && r.ts < next).map(|r| r.ai_credits).sum();
    Some(PlanLimit {
        provider: Provider::Copilot,
        window: "Monthly credits".into(),
        used_percent: (used / allowance as f64 * 100.0).clamp(0.0, 100.0),
        resets_at: Some(next.to_string()),
        detail: Some(format!("{} of {} credits", thousands(used), thousands(allowance as f64))),
        as_of: None,
        estimate: true,
    })
}

/// 7000.4 → "7,000".
fn thousands(v: f64) -> String {
    let n = v.max(0.0).round() as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn claude_reads_windows_and_drops_reset_ones() {
        // 2026-06-15T12:00Z = 1781524800
        let snap = r#"{"ts":"2026-06-15T11:00:00Z","rate_limits":{
            "five_hour":{"used_percentage":23.5,"resets_at":1781524700},
            "seven_day":{"used_percentage":41.2,"resets_at":1781956800},
            "spend_limit":{"used_percentage":120,"resets_at":1782000000,"used_usd":6012.4,"limit_usd":5000,"period":"monthly"}}}"#;
        let l = claude(snap, ts("2026-06-15T12:00:00Z"));
        let got: Vec<_> = l.iter().map(|l| (l.window.as_str(), l.used_percent, l.detail.as_deref())).collect();
        // five_hour reset 100 s ago → gone; spend over 100 % is shown full, details say by how much
        assert_eq!(got, [("Weekly", 41.2, None), ("Monthly spend", 100.0, Some("$6,012 of $5,000"))]);
        assert!(l.iter().all(|l| l.provider == Provider::Claude && !l.estimate && l.as_of.as_deref() == Some("2026-06-15T11:00:00Z")));
    }

    #[test]
    fn claude_tolerates_missing_and_bad_input() {
        let now = ts("2026-06-15T12:00:00Z");
        assert!(claude("not json", now).is_empty());
        assert!(claude(r#"{"rate_limits":null}"#, now).is_empty());
        let l = claude(r#"{"rate_limits":{"five_hour":{"used_percentage":"x"},"seven_day":{"used_percentage":5}}}"#, now);
        assert_eq!((l.len(), l[0].window.as_str(), l[0].resets_at.is_none()), (1, "Weekly", true));
    }

    #[test]
    fn copilot_counts_this_utc_month_only() {
        let r = |provider, at: &str, ai_credits| Request {
            provider,
            key: None,
            ts: ts(at),
            session: "s".into(),
            project: "p".into(),
            model: "m".into(),
            tokens: 0,
            output: 0,
            cost: None,
            cache_saved: 0.0,
            ai_credits,
        };
        let reqs = [
            r(Provider::Copilot, "2026-05-31T23:59:59Z", 9_000.0), // last month
            r(Provider::Copilot, "2026-06-01T00:00:00Z", 1_000.0),
            r(Provider::Copilot, "2026-06-15T11:00:00Z", 2_500.0),
            r(Provider::Claude, "2026-06-15T11:00:00Z", 500.0),
        ];
        let l = copilot(reqs.iter(), 7_000, ts("2026-06-15T12:00:00Z")).unwrap();
        assert_eq!(l.used_percent, 50.0);
        assert_eq!(l.detail.as_deref(), Some("3,500 of 7,000 credits"));
        assert_eq!(l.resets_at.as_deref(), Some("2026-07-01T00:00:00Z"));
        assert!(l.estimate);
        assert!(copilot(reqs.iter(), 0, ts("2026-06-15T12:00:00Z")).is_none());
    }
}
