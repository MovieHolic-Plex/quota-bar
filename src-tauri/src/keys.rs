//! Which key is live, and when to move off it.
//!
//! Every stored key is read on every poll, so the choice below is made from
//! fresh limits rather than from a 429 that has already cost a request.

use crate::quota::{peak_all_model_pct, QuotaSnapshot};
use serde::Serialize;

/// A key that failed back to must have this much headroom under the
/// threshold, or a key hovering at the line would flip back and forth — and
/// every flip rewrites the Claude settings files it is synced to.
pub const FAIL_BACK_MARGIN: f64 = 10.0;

/// Last read of one key.
#[derive(Debug, Clone, Default, Serialize)]
pub struct KeyStatus {
    #[serde(flatten)]
    pub snap: QuotaSnapshot,
    /// HTTP status when the proxy refused the key (401/403/429).
    pub refused: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    /// Read fine, all-model peak below the threshold.
    Healthy { peak: f64 },
    /// Refused, or at/over the threshold.
    Exhausted(String),
    /// Never read, or the last read did not land. Says nothing either way.
    Unknown,
}

impl Health {
    pub fn word(&self) -> &'static str {
        match self {
            Health::Healthy { .. } => "ok",
            Health::Exhausted(_) => "exhausted",
            Health::Unknown => "unknown",
        }
    }
}

pub fn health(status: Option<&KeyStatus>, threshold: f64) -> Health {
    let Some(st) = status else {
        return Health::Unknown;
    };
    if let Some(code) = st.refused {
        return Health::Exhausted(format!("HTTP {code}"));
    }
    // A dropped request leaves the last good limits in place, but they are
    // too old to route on.
    if st.snap.fetched_at.is_none() || st.snap.error.is_some() {
        return Health::Unknown;
    }
    let peak = peak_all_model_pct(&st.snap.limits).unwrap_or(0.0);
    if peak >= threshold {
        let which = st
            .snap
            .limits
            .iter()
            .filter(|l| !crate::quota::is_fable(&l.model_filter))
            .max_by(|a, b| a.used_percent.total_cmp(&b.used_percent))
            .map(|l| l.limit_window.clone())
            .unwrap_or_else(|| "limit".into());
        Health::Exhausted(format!("{which} {peak:.0}%"))
    } else {
        Health::Healthy { peak }
    }
}

/// The key that should be live, and why it changed (None = no change).
///
/// `order` is the enabled keys in priority order with their health.
pub fn choose(
    order: &[(String, Health)],
    current: Option<&str>,
    threshold: f64,
    fail_back: bool,
) -> (Option<String>, Option<String>) {
    let first_healthy = |upto: usize, margin: f64| {
        order[..upto].iter().find(|(_, h)| match h {
            Health::Healthy { peak } => *peak < threshold - margin,
            _ => false,
        })
    };
    let Some(cur_idx) = current.and_then(|c| order.iter().position(|(id, _)| id == c)) else {
        // Nothing live yet (or the live key was removed or disabled): take
        // the best key there is, falling back to plain priority order.
        let pick = first_healthy(order.len(), 0.0).or(order.first());
        return (pick.map(|(id, _)| id.clone()), None);
    };
    let (cur_id, cur_health) = &order[cur_idx];
    match cur_health {
        Health::Exhausted(why) => match first_healthy(order.len(), 0.0) {
            Some((id, _)) => (Some(id.clone()), Some(format!("{why} on the previous key"))),
            // Everything is spent. Stay put rather than cycle.
            None => (Some(cur_id.clone()), None),
        },
        _ if fail_back => match first_healthy(cur_idx, FAIL_BACK_MARGIN) {
            Some((id, _)) => (Some(id.clone()), Some("higher-priority key has headroom again".into())),
            None => (Some(cur_id.clone()), None),
        },
        _ => (Some(cur_id.clone()), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(p: f64) -> Health {
        Health::Healthy { peak: p }
    }
    fn full() -> Health {
        Health::Exhausted("daily 100%".into())
    }
    fn ids(v: &[(&str, Health)]) -> Vec<(String, Health)> {
        v.iter().map(|(i, h)| (i.to_string(), h.clone())).collect()
    }

    #[test]
    fn exhausted_key_moves_to_the_first_healthy_one() {
        let o = ids(&[("a", full()), ("b", Health::Unknown), ("c", ok(20.0))]);
        let (pick, why) = choose(&o, Some("a"), 98.0, true);
        assert_eq!(pick.as_deref(), Some("c"));
        assert!(why.unwrap().contains("daily 100%"));
    }

    #[test]
    fn stays_put_when_everything_is_spent() {
        let o = ids(&[("a", full()), ("b", full())]);
        assert_eq!(choose(&o, Some("b"), 98.0, true), (Some("b".into()), None));
    }

    #[test]
    fn unknown_current_key_is_not_abandoned() {
        let o = ids(&[("a", ok(10.0)), ("b", Health::Unknown)]);
        assert_eq!(choose(&o, Some("b"), 98.0, false), (Some("b".into()), None));
    }

    #[test]
    fn fail_back_needs_margin_under_the_threshold() {
        let near = ids(&[("a", ok(92.0)), ("b", ok(10.0))]);
        assert_eq!(choose(&near, Some("b"), 98.0, true).0.as_deref(), Some("b"));
        let clear = ids(&[("a", ok(50.0)), ("b", ok(10.0))]);
        let (pick, why) = choose(&clear, Some("b"), 98.0, true);
        assert_eq!(pick.as_deref(), Some("a"));
        assert!(why.is_some());
        assert_eq!(choose(&clear, Some("b"), 98.0, false).0.as_deref(), Some("b"));
    }

    #[test]
    fn missing_current_takes_first_healthy_then_first() {
        let o = ids(&[("a", full()), ("b", ok(5.0))]);
        assert_eq!(choose(&o, None, 98.0, true), (Some("b".into()), None));
        let o = ids(&[("a", Health::Unknown), ("b", Health::Unknown)]);
        assert_eq!(choose(&o, Some("gone"), 98.0, true), (Some("a".into()), None));
    }

    #[test]
    fn refusal_and_threshold_count_as_exhausted() {
        let refused = KeyStatus {
            refused: Some(429),
            ..Default::default()
        };
        assert_eq!(health(Some(&refused), 98.0), Health::Exhausted("HTTP 429".into()));
        assert_eq!(health(None, 98.0), Health::Unknown);
    }
}
