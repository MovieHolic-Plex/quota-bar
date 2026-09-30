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

pub const fn usd_from_micro(v: i64) -> f64 {
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

/// A failed read. `status` is set when the proxy answered, so a refusal
/// (401/403/429) can be told apart from a dropped connection — only the
/// former says anything about the key.
#[derive(Debug, Clone)]
pub struct FetchError {
    pub status: Option<u16>,
    pub message: String,
}

impl FetchError {
    fn net(message: String) -> Self {
        Self { status: None, message }
    }

    /// The proxy refused this key, as opposed to the request not landing.
    pub fn is_refusal(&self) -> bool {
        matches!(self.status, Some(401 | 402 | 403 | 429))
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Highest all-model used %. Fable limits are left out: a full fable window
/// only blocks fable, and the key still serves every other model.
pub fn peak_all_model_pct(limits: &[UsageLimit]) -> Option<f64> {
    limits
        .iter()
        .filter(|l| !is_fable(&l.model_filter))
        .map(|l| {
            if l.max_value > 0 && l.remaining_value <= 0 {
                100.0
            } else {
                l.used_percent
            }
        })
        .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))))
}

pub async fn fetch_usage(base_url: &str, api_key: &str) -> Result<QuotaSnapshot, FetchError> {
    let url = format!("{}/v1/usage/self", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| FetchError::net(e.to_string()))?;

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| FetchError::net(redact(&e.to_string(), api_key)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| FetchError::net(redact(&e.to_string(), api_key)))?;
    if !status.is_success() {
        return Err(FetchError {
            status: Some(status.as_u16()),
            message: format!(
                "HTTP {} {}",
                status.as_u16(),
                redact(&text.chars().take(180).collect::<String>(), api_key)
            ),
        });
    }

    let parsed: UsageSelf = serde_json::from_str(&text)
        .map_err(|e| FetchError::net(format!("usage/self parse: {e}")))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(window: &str, fable: bool, current: i64, max: i64) -> UsageLimit {
        UsageLimit {
            limit_type: "cost_usd".into(),
            limit_window: window.into(),
            max_value: max,
            current_value: current,
            remaining_value: max - current,
            used_percent: current as f64 / max as f64 * 100.0,
            model_filter: if fable {
                Some("fable".into())
            } else {
                None
            },
            reset_at: "2026-09-10T06:34:30.940247".into(),
        }
    }

    #[test]
    fn usd_from_micro_divides_by_a_million() {
        assert!((usd_from_micro(5_600_000_000) - 5600.0).abs() < 1e-9);
        assert!((usd_from_micro(268_211_260) - 268.21126).abs() < 1e-9);
    }

    #[test]
    fn pick_limit_separates_fable_from_all_models() {
        let limits = vec![
            limit("daily", false, 268_211_260, 5_600_000_000),
            limit("daily", true, 791_421, 2_800_000_000),
            limit("3h", false, 268_211_260, 3_500_000_000),
        ];
        let daily = pick_limit(&limits, "daily", false).unwrap();
        assert_eq!(daily.max_value, 5_600_000_000);
        let fable_daily = pick_limit(&limits, "daily", true).unwrap();
        assert_eq!(fable_daily.max_value, 2_800_000_000);
        assert!(pick_limit(&limits, "weekly", false).is_none());
    }

    #[test]
    fn named_limits_maps_the_live_payload_shape() {
        let limits = vec![
            limit("3h", false, 1, 10),
            limit("daily", false, 2, 20),
            limit("weekly", false, 3, 30),
            limit("daily", true, 4, 40),
            limit("weekly", true, 5, 50),
        ];
        let named = named_limits(&limits);
        assert_eq!(named.three_h.unwrap().max_usd, usd_from_micro(10));
        assert_eq!(named.daily.unwrap().max_usd, usd_from_micro(20));
        assert_eq!(named.weekly.unwrap().max_usd, usd_from_micro(30));
        assert_eq!(named.fable_daily.unwrap().max_usd, usd_from_micro(40));
        assert_eq!(named.fable_weekly.unwrap().max_usd, usd_from_micro(50));
    }

    #[test]
    fn peak_ignores_fable_and_treats_empty_remaining_as_full() {
        let limits = vec![
            limit("daily", false, 40, 100),
            limit("weekly", false, 70, 100),
            limit("daily", true, 100, 100),
        ];
        assert_eq!(peak_all_model_pct(&limits), Some(70.0));
        let mut full = limit("3h", false, 99, 100);
        full.remaining_value = 0;
        assert_eq!(peak_all_model_pct(&[full]), Some(100.0));
        assert_eq!(peak_all_model_pct(&[]), None);
    }

    #[test]
    fn redact_strips_the_api_key_from_error_text() {
        assert_eq!(redact("boom sk-ant-secret here", "sk-ant-secret"), "boom *** here");
        assert_eq!(redact("no secret", ""), "no secret");
    }
}
