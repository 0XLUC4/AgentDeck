//! Subscription usage limits (5-hour and weekly windows) for Claude Pro/Max and ChatGPT/Codex.
//! Claude: the endpoint behind Claude Code's `/usage`, called with Claude Code's own OAuth token.
//! Codex: the newest `rate_limits` snapshot the Codex CLI writes into its session transcripts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

/// One limit window: percent used (0-100) and when it resets (unix seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitWindow {
    pub percent: f64,
    pub resets_at: Option<i64>,
}

/// The two windows both providers report. A window is `None` when the provider leaves it out.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SubscriptionLimits {
    pub five_hour: Option<LimitWindow>,
    pub weekly: Option<LimitWindow>,
}

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_LOOKBACK_MS: i64 = 7 * 24 * 3600 * 1000;

/// Parses the `/api/oauth/usage` body: `five_hour` / `seven_day` with `utilization` (percent)
/// and `resets_at` (RFC 3339).
pub fn parse_claude_usage(body: &str) -> Option<SubscriptionLimits> {
    let json: Value = serde_json::from_str(body).ok()?;
    let window = |key: &str| {
        let w = json.get(key)?;
        Some(LimitWindow {
            percent: w.get("utilization")?.as_f64()?,
            resets_at: w
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp()),
        })
    };
    let limits = SubscriptionLimits { five_hour: window("five_hour"), weekly: window("seven_day") };
    (limits != SubscriptionLimits::default()).then_some(limits)
}

/// Parses one Codex transcript line holding `rate_limits` (`primary` / `secondary` with
/// `used_percent`, `window_minutes`, `resets_at` in unix seconds). A window that has reset since
/// the snapshot (`resets_at` before `now_s`) reads 0%.
pub fn parse_codex_line(line: &str, now_s: i64) -> Option<SubscriptionLimits> {
    let json: Value = serde_json::from_str(line).ok()?;
    let limits = json.get("payload").and_then(|p| p.get("rate_limits")).or_else(|| json.get("rate_limits"))?;
    let mut out = SubscriptionLimits::default();
    for key in ["primary", "secondary"] {
        let Some(w) = limits.get(key).filter(|w| w.is_object()) else { continue };
        let Some(percent) = w.get("used_percent").and_then(Value::as_f64) else { continue };
        let resets_at = w.get("resets_at").and_then(Value::as_i64);
        let percent = if resets_at.is_some_and(|r| r < now_s) { 0.0 } else { percent };
        let window = Some(LimitWindow { percent, resets_at });
        // Windows up to a day are the short one (300 min today); longer ones the weekly one.
        let short = w.get("window_minutes").and_then(Value::as_i64).map_or(key == "primary", |m| m <= 24 * 60);
        if short { out.five_hour = window } else { out.weekly = window }
    }
    (out != SubscriptionLimits::default()).then_some(out)
}

/// Claude subscription usage. `Ok(None)`: not signed in to Claude Code with a subscription
/// (or the stored token expired: Claude Code refreshes it on its next run).
pub fn claude_limits() -> Result<Option<SubscriptionLimits>, String> {
    let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| Some(super::home()?.join(".claude")))
    else {
        return Ok(None);
    };
    let Ok(text) = std::fs::read_to_string(dir.join(".credentials.json")) else { return Ok(None) };
    let creds: Value = serde_json::from_str(&text).map_err(|e| format!("Claude credentials: {e}"))?;
    let oauth = &creds["claudeAiOauth"];
    let Some(token) = oauth["accessToken"].as_str() else { return Ok(None) };
    let expired = oauth["expiresAt"].as_i64().is_some_and(|ms| ms <= chrono::Utc::now().timestamp_millis());
    if expired {
        return Ok(None);
    }
    let body = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "agentdeck")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::text)
        // The error text never holds the token (it is a header).
        .map_err(|e| format!("Claude usage: {e}"))?;
    Ok(parse_claude_usage(&body))
}

/// Codex subscription usage from the most recently written session that has a snapshot.
pub fn codex_limits() -> Option<SubscriptionLimits> {
    let root = super::codex_sessions()?;
    let now = chrono::Utc::now();
    let mut files = Vec::new();
    super::walk(&root, now.timestamp_millis() - CODEX_LOOKBACK_MS, &mut files);
    files.retain(|(path, _, _)| path.extension().is_some_and(|e| e == "jsonl"));
    files.sort_by_key(|(_, _, mtime)| std::cmp::Reverse(*mtime));
    files.iter().take(5).find_map(|(path, _, _)| newest_snapshot(path, now.timestamp()))
}

fn newest_snapshot(path: &Path, now_s: i64) -> Option<SubscriptionLimits> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().rev().filter(|line| line.contains("\"rate_limits\":{")).find_map(|line| parse_codex_line(line, now_s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_usage() {
        let body = r#"{"five_hour":{"utilization":42.0,"resets_at":"2026-11-20T14:19:59.584353+00:00"},
            "seven_day":{"utilization":7.5,"resets_at":null},"seven_day_opus":null}"#;
        let limits = parse_claude_usage(body).unwrap();
        assert_eq!(limits.five_hour, Some(LimitWindow { percent: 42.0, resets_at: Some(1_795_184_399) }));
        assert_eq!(limits.weekly, Some(LimitWindow { percent: 7.5, resets_at: None }));
        assert_eq!(parse_claude_usage(r#"{"five_hour":null,"seven_day":null}"#), None);
        assert_eq!(parse_claude_usage("not json"), None);
    }

    #[test]
    fn parses_codex_snapshot() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{
            "primary":{"used_percent":22.0,"window_minutes":300,"resets_at":2000},
            "secondary":{"used_percent":18.0,"window_minutes":10080,"resets_at":900}}}}"#;
        let limits = parse_codex_line(line, 1000).unwrap();
        assert_eq!(limits.five_hour, Some(LimitWindow { percent: 22.0, resets_at: Some(2000) }));
        // The weekly window reset after the snapshot.
        assert_eq!(limits.weekly, Some(LimitWindow { percent: 0.0, resets_at: Some(900) }));
        assert_eq!(parse_codex_line(r#"{"payload":{"rate_limits":null}}"#, 0), None);
    }
}
