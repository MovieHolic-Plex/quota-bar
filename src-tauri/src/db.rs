use crate::quota::{now_unix, QuotaSnapshot};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn open() -> Result<Connection, String> {
    let dir = crate::config::config_dir().ok_or("could not resolve data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join("usage.db");
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        PRAGMA busy_timeout=3000;
        CREATE TABLE IF NOT EXISTS snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            request_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            cached_input_tokens INTEGER NOT NULL,
            total_cost_usd REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_ts ON snapshots(ts);
        ",
    )
    .map_err(|e| e.to_string())?;
    // Totals are per key, so a delta across two keys is meaningless. Every
    // row says which key it came from and every query stays inside one.
    let has_key_col: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('snapshots') WHERE name = 'key_id'")
        .and_then(|mut st| st.exists([]))
        .map_err(|e| e.to_string())?;
    if !has_key_col {
        conn.execute_batch("ALTER TABLE snapshots ADD COLUMN key_id TEXT;")
            .map_err(|e| e.to_string())?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_snapshots_key_ts ON snapshots(key_id, ts);")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Rows written before keys had ids belong to whichever key was first.
pub fn adopt_legacy(conn: &Connection, key_id: &str) -> Result<usize, String> {
    conn.execute(
        "UPDATE snapshots SET key_id = ?1 WHERE key_id IS NULL",
        params![key_id],
    )
    .map_err(|e| e.to_string())
}

pub fn insert_snapshot(conn: &Connection, key: &str, snap: &QuotaSnapshot) -> Result<(), String> {
    conn.execute(
        "INSERT INTO snapshots (ts, request_count, total_tokens, cached_input_tokens, total_cost_usd, key_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snap.fetched_at.unwrap_or_else(now_unix) as i64,
            snap.request_count,
            snap.total_tokens,
            snap.cached_input_tokens,
            snap.total_cost_usd,
            key
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop samples older than `retain_secs`.
///
/// One row lands per poll, so without this the table grows without bound and
/// the wide bands end up scanning years of history every time the stats window
/// refreshes. The cost of pruning is that the "all" band means "as far back as
/// we still keep", which is what `first_ts` already reports.
pub fn prune(conn: &Connection, retain_secs: i64) -> Result<usize, String> {
    let cutoff = now_unix() as i64 - retain_secs.max(86_400);
    conn.execute("DELETE FROM snapshots WHERE ts < ?1", params![cutoff])
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct BandStats {
    pub label: String,
    pub seconds: u64,
    pub requests: i64,
    pub tokens: i64,
    pub cached: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BucketRow {
    pub start_ts: i64,
    pub tokens: i64,
    pub cached: i64,
    pub cost_usd: f64,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct UsageStats {
    pub key_id: String,
    pub latest: QuotaSnapshot,
    pub bands: Vec<BandStats>,
    pub hourly: Vec<BucketRow>,
    pub daily: Vec<BucketRow>,
    pub minutes: Vec<BucketRow>,
    pub snapshot_count: i64,
    pub first_ts: Option<i64>,
    pub daily_quota_usd: f64,
    /// Spend since the key's last daily reset (None when no reset time is configured).
    pub since_reset: Option<BandStats>,
    pub last_reset_ts: Option<i64>,
    pub next_reset_ts: Option<i64>,
}

struct Row {
    ts: i64,
    request_count: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    total_cost_usd: f64,
}

fn latest_row(conn: &Connection, key: &str) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots WHERE key_id = ?1 ORDER BY ts DESC, id DESC LIMIT 1",
        params![key],
        |r| {
            Ok(Row {
                ts: r.get(0)?,
                request_count: r.get(1)?,
                total_tokens: r.get(2)?,
                cached_input_tokens: r.get(3)?,
                total_cost_usd: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn row_at_or_before(conn: &Connection, key: &str, ts: i64) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots WHERE key_id = ?1 AND ts <= ?2 ORDER BY ts DESC, id DESC LIMIT 1",
        params![key, ts],
        |r| {
            Ok(Row {
                ts: r.get(0)?,
                request_count: r.get(1)?,
                total_tokens: r.get(2)?,
                cached_input_tokens: r.get(3)?,
                total_cost_usd: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn earliest_row(conn: &Connection, key: &str) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots WHERE key_id = ?1 ORDER BY ts ASC, id ASC LIMIT 1",
        params![key],
        |r| {
            Ok(Row {
                ts: r.get(0)?,
                request_count: r.get(1)?,
                total_tokens: r.get(2)?,
                cached_input_tokens: r.get(3)?,
                total_cost_usd: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn clamp_delta(new: i64, old: i64) -> i64 {
    (new - old).max(0)
}

fn band(conn: &Connection, key: &str, label: &str, seconds: u64, latest: &Row) -> Result<BandStats, String> {
    let cutoff = if seconds == 0 {
        i64::MIN / 4
    } else {
        latest.ts.saturating_sub(seconds as i64)
    };
    band_from_cutoff(conn, key, label, seconds, cutoff, latest)
}

/// Delta since an absolute point in time (e.g. the key's last daily reset).
fn band_since(
    conn: &Connection,
    key: &str,
    label: &str,
    since_ts: i64,
    latest: &Row,
) -> Result<BandStats, String> {
    let seconds = latest.ts.saturating_sub(since_ts).max(0) as u64;
    band_from_cutoff(conn, key, label, seconds, since_ts, latest)
}

fn band_from_cutoff(
    conn: &Connection,
    key: &str,
    label: &str,
    seconds: u64,
    cutoff: i64,
    latest: &Row,
) -> Result<BandStats, String> {
    let baseline = row_at_or_before(conn, key, cutoff)?.or(earliest_row(conn, key)?);
    let Some(old) = baseline else {
        return Ok(BandStats {
            label: label.into(),
            seconds,
            requests: 0,
            tokens: 0,
            cached: 0,
            cost_usd: 0.0,
        });
    };
    // A band is two indexed point lookups and nothing else. The row count that
    // used to live here scanned the index from `old.ts` to the end of the table
    // once per band, eleven times per stats load, and nothing ever read it.
    Ok(BandStats {
        label: label.into(),
        seconds,
        requests: clamp_delta(latest.request_count, old.request_count),
        tokens: clamp_delta(latest.total_tokens, old.total_tokens),
        cached: clamp_delta(latest.cached_input_tokens, old.cached_input_tokens),
        cost_usd: (latest.total_cost_usd - old.total_cost_usd).max(0.0),
    })
}

fn buckets(
    conn: &Connection,
    key: &str,
    bucket_secs: i64,
    lookback_secs: i64,
) -> Result<Vec<BucketRow>, String> {
    let sql = format!(
        "
        WITH hourly AS (
            SELECT (ts / {bucket}) * {bucket} AS start_ts,
                   MAX(total_tokens) AS tokens,
                   MAX(cached_input_tokens) AS cached,
                   MAX(total_cost_usd) AS cost,
                   MAX(request_count) AS reqs
            FROM snapshots
            WHERE key_id = ?1 AND ts >= (strftime('%s','now') - {lookback})
            GROUP BY 1
        ),
        delta AS (
            SELECT start_ts,
                   tokens - LAG(tokens) OVER (ORDER BY start_ts) AS d_tokens,
                   cached - LAG(cached) OVER (ORDER BY start_ts) AS d_cached,
                   cost - LAG(cost) OVER (ORDER BY start_ts) AS d_cost,
                   reqs - LAG(reqs) OVER (ORDER BY start_ts) AS d_reqs
            FROM hourly
        )
        SELECT start_ts,
               CASE WHEN d_tokens IS NULL OR d_tokens < 0 THEN 0 ELSE d_tokens END,
               CASE WHEN d_cached IS NULL OR d_cached < 0 THEN 0 ELSE d_cached END,
               CASE WHEN d_cost IS NULL OR d_cost < 0 THEN 0 ELSE d_cost END,
               CASE WHEN d_reqs IS NULL OR d_reqs < 0 THEN 0 ELSE d_reqs END
        FROM delta
        ORDER BY start_ts
        ",
        bucket = bucket_secs,
        lookback = lookback_secs
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![key], |r| {
            Ok(BucketRow {
                start_ts: r.get(0)?,
                tokens: r.get(1)?,
                cached: r.get(2)?,
                cost_usd: r.get(3)?,
                requests: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn load_stats(
    conn: &Connection,
    key: &str,
    paid_usd: f64,
    daily_quota_usd: f64,
    reset: Option<(i64, i64)>,
) -> Result<UsageStats, String> {
    let latest_row = latest_row(conn, key)?;
    let snapshot_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM snapshots WHERE key_id = ?1",
            params![key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let first_ts = earliest_row(conn, key)?.map(|r| r.ts);

    let Some(latest) = latest_row else {
        return Ok(UsageStats {
            key_id: key.into(),
            latest: QuotaSnapshot {
                error: Some("no samples yet".into()),
                ..Default::default()
            },
            bands: vec![],
            hourly: vec![],
            daily: vec![],
            minutes: vec![],
            snapshot_count,
            first_ts,
            daily_quota_usd,
            since_reset: None,
            last_reset_ts: reset.map(|r| r.0),
            next_reset_ts: reset.map(|r| r.1),
        });
    };

    let cache_pct = if latest.total_tokens > 0 {
        latest.cached_input_tokens as f64 / latest.total_tokens as f64 * 100.0
    } else {
        0.0
    };
    let snap = QuotaSnapshot {
        request_count: latest.request_count,
        total_tokens: latest.total_tokens,
        cached_input_tokens: latest.cached_input_tokens,
        total_cost_usd: latest.total_cost_usd,
        paid_usd,
        pro_usd: paid_usd,
        savings_usd: latest.total_cost_usd - paid_usd,
        cache_pct,
        error: None,
        fetched_at: Some(latest.ts as u64),
        stale: false,
        limits: vec![],
    };

    let bands = vec![
        band(conn, key, "10m", 600, &latest)?,
        band(conn, key, "1h", 3600, &latest)?,
        band(conn, key, "5h", 5 * 3600, &latest)?,
        band(conn, key, "1d", 24 * 3600, &latest)?,
        band(conn, key, "3d", 3 * 24 * 3600, &latest)?,
        band(conn, key, "7d", 7 * 24 * 3600, &latest)?,
        band(conn, key, "30d", 30 * 24 * 3600, &latest)?,
        band(conn, key, "all", 0, &latest)?,
    ];

    let since_reset = match reset {
        Some((last, _)) => Some(band_since(conn, key, "reset", last, &latest)?),
        None => None,
    };

    Ok(UsageStats {
        key_id: key.into(),
        latest: snap,
        bands,
        hourly: buckets(conn, key, 3600, 48 * 3600)?,
        daily: buckets(conn, key, 86400, 30 * 86400)?,
        minutes: minute_series(conn, key, 30)?,
        snapshot_count,
        first_ts,
        daily_quota_usd,
        since_reset,
        last_reset_ts: reset.map(|r| r.0),
        next_reset_ts: reset.map(|r| r.1),
    })
}

/// API-equivalent USD spent since `since_ts` (0.0 when there are no samples).
pub fn spend_since(conn: &Connection, key: &str, since_ts: i64) -> Result<f64, String> {
    let Some(latest) = latest_row(conn, key)? else {
        return Ok(0.0);
    };
    Ok(band_since(conn, key, "reset", since_ts, &latest)?.cost_usd)
}

pub fn recent_spend(conn: &Connection, key: &str) -> Result<(f64, f64, f64), String> {
    let Some(latest) = latest_row(conn, key)? else {
        return Ok((0.0, 0.0, 0.0));
    };
    let ten = band(conn, key, "10m", 600, &latest)?;
    let hour = band(conn, key, "1h", 3600, &latest)?;
    let day = band(conn, key, "1d", 24 * 3600, &latest)?;
    Ok((ten.cost_usd, hour.cost_usd, day.cost_usd))
}

pub fn minute_series(conn: &Connection, key: &str, minutes: i64) -> Result<Vec<BucketRow>, String> {
    let now = now_unix() as i64;
    let lookback = minutes * 60;
    let raw = buckets(conn, key, 60, lookback)?;
    let mut by_ts: HashMap<i64, BucketRow> = HashMap::new();
    for row in raw {
        by_ts.insert(row.start_ts, row);
    }
    let end = (now / 60) * 60;
    let start = end - (minutes - 1) * 60;
    let mut out = Vec::with_capacity(minutes as usize);
    let mut t = start;
    while t <= end {
        out.push(by_ts.remove(&t).unwrap_or(BucketRow {
            start_ts: t,
            tokens: 0,
            cached: 0,
            cost_usd: 0.0,
            requests: 0,
        }));
        t += 60;
    }
    Ok(out)
}
