//! Auto-summary generation using OpenAI API.

use std::process::Command as StdCommand;
use std::time::Duration;

use tracing::{info, warn};

/// Generate a work summary using OpenAI API.
/// Returns None if OPENAI_API_KEY is not set or on any failure.
pub async fn generate_summary() -> Option<String> {
    let api_key = std::env::var("OPENAI_API_KEY").ok()?;
    if api_key.is_empty() {
        return None;
    }

    info!("Generating auto-summary via OpenAI API...");

    let branch = get_git_branch().unwrap_or_default();
    let recent_files = get_recent_files().unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let prompt = format!(
        "Summarize what this developer is working on in 1-2 sentences based on:\n\
         Working directory: {cwd}\n\
         Git branch: {branch}\n\
         Recent files: {recent_files}\n\
         Be concise and specific."
    );

    let body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [
            {"role": "system", "content": "You are a concise assistant that summarizes developer work context."},
            {"role": "user", "content": prompt}
        ],
        "max_tokens": 100,
        "temperature": 0.3
    });

    let client = reqwest::Client::new();

    let result = tokio::time::timeout(
        Duration::from_secs(3),
        client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send(),
    )
    .await;

    match result {
        Ok(Ok(resp)) => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let summary = json["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|s| s.trim().to_string());
                if let Some(ref s) = summary {
                    info!(summary = %s, "Auto-summary generated");
                }
                summary
            } else {
                warn!("Failed to parse OpenAI response");
                None
            }
        }
        Ok(Err(e)) => {
            warn!(error = %e, "OpenAI API request failed");
            None
        }
        Err(_) => {
            warn!("OpenAI API request timed out (3s)");
            None
        }
    }
}

fn get_git_branch() -> Option<String> {
    StdCommand::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

fn get_recent_files() -> Option<String> {
    // Combine git diff and recent log
    let diff_files = StdCommand::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let log_files = StdCommand::new("git")
        .args(["log", "--name-only", "--format=", "-5"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let mut files: Vec<&str> = diff_files
        .lines()
        .chain(log_files.lines())
        .filter(|s| !s.is_empty())
        .collect();
    files.dedup();
    files.truncate(10);

    if files.is_empty() {
        None
    } else {
        Some(files.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn summary_is_skipped_when_api_key_is_absent_or_empty() {
        // Safety: test runs single-threaded for this env var manipulation
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        assert!(generate_summary().await.is_none());

        unsafe { std::env::set_var("OPENAI_API_KEY", "") };
        assert!(generate_summary().await.is_none());
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
    }
}
