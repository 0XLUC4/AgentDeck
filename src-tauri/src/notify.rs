//! Phone notifications: POSTs the text to an ntfy topic URL (https://ntfy.sh/<topic>) or any webhook.
//! Ref: https://docs.ntfy.sh/publish/

use std::time::Duration;

const MAX_TEXT: usize = 4 * 1024;

/// https anywhere; plain http only to this machine (a self-hosted relay).
fn check_url(url: &str) -> Result<(), String> {
    let local = ["http://127.0.0.1", "http://localhost"].iter().any(|p| {
        url.strip_prefix(p)
            .is_some_and(|rest| rest.starts_with([':', '/']))
    });
    if url.starts_with("https://") || local {
        Ok(())
    } else {
        Err("the notification URL must start with https://".into())
    }
}

#[tauri::command(async)]
pub fn push_notify(url: String, title: String, body: String) -> Result<(), String> {
    let url = url.trim();
    check_url(url)?;
    // ntfy header values must be ASCII; otherwise the title becomes the first line of the body.
    let header = title.is_ascii() && !title.contains(['\r', '\n']);
    let mut text = if header {
        body
    } else {
        format!("{title}\n{body}")
    };
    text.truncate(text.floor_char_boundary(MAX_TEXT));
    let mut req = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?
        .post(url)
        .body(text);
    if header {
        req = req.header("Title", title);
    }
    req.send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map(drop)
        .map_err(|e| format!("push failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_localhost() {
        assert!(check_url("https://ntfy.sh/my-topic").is_ok());
        assert!(check_url("http://localhost:8080/t").is_ok());
        assert!(check_url("http://127.0.0.1/t").is_ok());
        assert!(check_url("http://localhost.evil.com/t").is_err());
        assert!(check_url("http://ntfy.sh/t").is_err());
        assert!(check_url("file:///c:/x").is_err());
    }
}
