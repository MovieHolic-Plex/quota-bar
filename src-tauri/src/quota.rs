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
    /// True when `error` is set but the figures are a retained copy of the
    /// last successful poll. A dropped request should not blank the readout.
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub limits: Vec<UsageLimit>,
}

/// `/v1/usage/self` stores cost as microdollars (USD × 1_000_000).
#[derive(Debug, Clone, Serialize, Default)]
pub struct LimitView {
    pub window: String,
    pub model_filter: Option<String>,
    pub used_percent: f64,
    pub current_usd: f64,
    pub max_usd: f64,
    pub remaining_usd: f64,
    pub reset_at: String,
}

pub fn usd_from_micro(v: i64) -> f64 {
    v as f64 / 1_000_000.0
}

pub fn is_fable(mf: &Option<String>) -> bool {
    mf.as_deref()
        .map(|s| s.to_ascii_lowercase().contains("fable"))
        .unwrap_or(false)
}

pub fn pick_limit<'a>(
    limits: &'a [UsageLimit],
    window: &str,
    fable: bool,
) -> Option<&'a UsageLimit> {
    limits.iter().find(|l| {
        l.limit_window.eq_ignore_ascii_case(window) && is_fable(&l.model_filter) == fable
    })
}

pub fn to_view(l: &UsageLimit) -> LimitView {
    LimitView {
        window: l.limit_window.clone(),
        model_filter: l.model_filter.clone(),
        used_percent: l.used_percent,
        current_usd: usd_from_micro(l.current_value),
        max_usd: usd_from_micro(l.max_value),
        remaining_usd: usd_from_micro(l.remaining_value),
        reset_at: l.reset_at.clone(),
    }
}

pub fn named_limits(limits: &[UsageLimit]) -> NamedLimits {
    NamedLimits {
        three_h: pick_limit(limits, "3h", false).map(to_view),
        daily: pick_limit(limits, "daily", false).map(to_view),
        weekly: pick_limit(limits, "weekly", false).map(to_view),
        fable_daily: pick_limit(limits, "daily", true).map(to_view),
        fable_weekly: pick_limit(limits, "weekly", true).map(to_view),
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NamedLimits {
    pub three_h: Option<LimitView>,
    pub daily: Option<LimitView>,
    pub weekly: Option<LimitView>,
    pub fable_daily: Option<LimitView>,
    pub fable_weekly: Option<LimitView>,
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
        stale: false,
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
