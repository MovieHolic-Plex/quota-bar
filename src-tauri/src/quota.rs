use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaSnapshot {
    pub remaining_5h: Option<f64>,
    pub remaining_7d: Option<f64>,
    pub used_5h: Option<f64>,
    pub used_7d: Option<f64>,
    pub reset_5h: Option<u64>,
    pub reset_7d: Option<u64>,
    pub status_5h: Option<String>,
    pub status_7d: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub fetched_at: Option<u64>,
}

fn header_f64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
}

fn header_u64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

fn header_str(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn remaining(used: Option<f64>) -> Option<f64> {
    used.map(|u| (1.0 - u).clamp(0.0, 1.0))
}

pub async fn fetch_quota(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<QuotaSnapshot, String> {
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "."}]
    });

    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| redact(&e.to_string(), api_key))?;

    let status = response.status();
    let headers = response.headers().clone();
    let used_5h = header_f64(&headers, "anthropic-ratelimit-unified-5h-utilization");
    let used_7d = header_f64(&headers, "anthropic-ratelimit-unified-7d-utilization");

    let mut snap = QuotaSnapshot {
        used_5h,
        used_7d,
        remaining_5h: remaining(used_5h),
        remaining_7d: remaining(used_7d),
        reset_5h: header_u64(&headers, "anthropic-ratelimit-unified-5h-reset"),
        reset_7d: header_u64(&headers, "anthropic-ratelimit-unified-7d-reset"),
        status_5h: header_str(&headers, "anthropic-ratelimit-unified-5h-status"),
        status_7d: header_str(&headers, "anthropic-ratelimit-unified-7d-status"),
        status: header_str(&headers, "anthropic-ratelimit-unified-status"),
        error: None,
        fetched_at: Some(now_unix()),
    };

    if !status.is_success() && snap.remaining_5h.is_none() && snap.remaining_7d.is_none() {
        let text = response.text().await.unwrap_or_default();
        snap.error = Some(format!(
            "HTTP {} {}",
            status.as_u16(),
            redact(&text.chars().take(180).collect::<String>(), api_key)
        ));
    }

    Ok(snap)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn redact(input: &str, secret: &str) -> String {
    if secret.is_empty() {
        input.to_string()
    } else {
        input.replace(secret, "***")
    }
}
