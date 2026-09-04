use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageLimit {
    pub limit_type: String,
    pub limit_window: String,
    pub max_value: i64,
    pub current_value: i64,
    pub remaining_value: i64,
    pub used_percent: f64,
    pub model_filter: Option<String>,
    pub reset_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaSnapshot {
    pub request_count: i64,
    pub total_tokens: i64,
    pub cached_input_tokens: i64,
    pub total_cost_usd: f64,
    pub paid_usd: f64,
    pub pro_usd: f64,
    pub savings_usd: f64,
    pub cache_pct: f64,
    pub error: Option<String>,
    pub fetched_at: Option<u64>,
    #[serde(default)]
    pub limits: Vec<UsageLimit>,
}

#[derive(Debug, Deserialize)]
struct UsageSelf {
    request_count: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    total_cost_usd: f64,
    #[serde(default)]
    limits: Vec<UsageLimit>,
}

pub async fn fetch_usage(base_url: &str, api_key: &str) -> Result<QuotaSnapshot, String> {
    let url = format!("{}/v1/usage/self", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| redact(&e.to_string(), api_key))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| redact(&e.to_string(), api_key))?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {} {}",
            status.as_u16(),
            redact(&text.chars().take(180).collect::<String>(), api_key)
        ));
    }

    let parsed: UsageSelf =
        serde_json::from_str(&text).map_err(|e| format!("usage/self parse: {e}"))?;

    Ok(QuotaSnapshot {
        request_count: parsed.request_count,
        total_tokens: parsed.total_tokens,
        cached_input_tokens: parsed.cached_input_tokens,
        total_cost_usd: parsed.total_cost_usd,
        paid_usd: 0.0,
        pro_usd: 20.0,
        savings_usd: parsed.total_cost_usd - 20.0,
        cache_pct: if parsed.total_tokens > 0 {
            parsed.cached_input_tokens as f64 / parsed.total_tokens as f64 * 100.0
        } else {
            0.0
        },
        error: None,
        fetched_at: Some(now_unix()),
        limits: parsed.limits,
    })
}

pub fn now_unix() -> u64 {
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
