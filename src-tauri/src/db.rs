use crate::quota::{now_unix, QuotaSnapshot};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;

pub fn open() -> Result<Connection, String> {
    let dir = crate::config::config_dir().ok_or("could not resolve data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join("usage.db");
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
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
    Ok(conn)
}

pub fn insert_snapshot(conn: &Connection, snap: &QuotaSnapshot) -> Result<(), String> {
    conn.execute(
        "INSERT INTO snapshots (ts, request_count, total_tokens, cached_input_tokens, total_cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            snap.fetched_at.unwrap_or_else(now_unix) as i64,
            snap.request_count,
            snap.total_tokens,
            snap.cached_input_tokens,
            snap.total_cost_usd
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct BandStats {
    pub label: String,
    pub seconds: u64,
    pub requests: i64,
    pub tokens: i64,
    pub cached: i64,
    pub cost_usd: f64,
    pub samples: i64,
}

#[derive(Debug, Serialize)]
pub struct BucketRow {
    pub start_ts: i64,
    pub tokens: i64,
    pub cached: i64,
    pub cost_usd: f64,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct UsageStats {
    pub latest: QuotaSnapshot,
    pub bands: Vec<BandStats>,
    pub hourly: Vec<BucketRow>,
    pub daily: Vec<BucketRow>,
    pub snapshot_count: i64,
    pub first_ts: Option<i64>,
}

struct Row {
    ts: i64,
    request_count: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    total_cost_usd: f64,
}

fn latest_row(conn: &Connection) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots ORDER BY ts DESC, id DESC LIMIT 1",
        [],
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

fn row_at_or_before(conn: &Connection, ts: i64) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots WHERE ts <= ?1 ORDER BY ts DESC, id DESC LIMIT 1",
        params![ts],
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

fn earliest_row(conn: &Connection) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT ts, request_count, total_tokens, cached_input_tokens, total_cost_usd
         FROM snapshots ORDER BY ts ASC, id ASC LIMIT 1",
        [],
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

fn band(conn: &Connection, label: &str, seconds: u64, latest: &Row) -> Result<BandStats, String> {
    let cutoff = if seconds == 0 {
        i64::MIN / 4
    } else {
        latest.ts.saturating_sub(seconds as i64)
    };
    let baseline = row_at_or_before(conn, cutoff)?.or(earliest_row(conn)?);
    let Some(old) = baseline else {
        return Ok(BandStats {
            label: label.into(),
            seconds,
            requests: 0,
            tokens: 0,
            cached: 0,
            cost_usd: 0.0,
            samples: 0,
        });
    };
    let samples: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM snapshots WHERE ts >= ?1",
            params![old.ts],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(BandStats {
        label: label.into(),
        seconds,
        requests: clamp_delta(latest.request_count, old.request_count),
        tokens: clamp_delta(latest.total_tokens, old.total_tokens),
        cached: clamp_delta(latest.cached_input_tokens, old.cached_input_tokens),
        cost_usd: (latest.total_cost_usd - old.total_cost_usd).max(0.0),
        samples,
    })
}

fn buckets(conn: &Connection, bucket_secs: i64, lookback_secs: i64) -> Result<Vec<BucketRow>, String> {
    let sql = format!(
        "
        WITH hourly AS (
            SELECT (ts / {bucket}) * {bucket} AS start_ts,
                   MAX(total_tokens) AS tokens,
                   MAX(cached_input_tokens) AS cached,
                   MAX(total_cost_usd) AS cost,
                   MAX(request_count) AS reqs
            FROM snapshots
            WHERE ts >= (strftime('%s','now') - {lookback})
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
        .query_map([], |r| {
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

pub fn load_stats(conn: &Connection, paid_usd: f64) -> Result<UsageStats, String> {
    let latest_row = latest_row(conn)?;
    let snapshot_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap_or(0);
    let first_ts = earliest_row(conn)?.map(|r| r.ts);

    let Some(latest) = latest_row else {
        return Ok(UsageStats {
            latest: QuotaSnapshot {
                error: Some("no samples yet".into()),
                ..Default::default()
            },
            bands: vec![],
            hourly: vec![],
            daily: vec![],
            snapshot_count,
            first_ts,
        });
    };

    let mut snap = QuotaSnapshot {
        request_count: latest.request_count,
        total_tokens: latest.total_tokens,
        cached_input_tokens: latest.cached_input_tokens,
        total_cost_usd: latest.total_cost_usd,
        paid_usd,
        savings_usd: latest.total_cost_usd - paid_usd,
        error: None,
        fetched_at: Some(latest.ts as u64),
    };
    snap.savings_usd = snap.total_cost_usd - paid_usd;

    let bands = vec![
        band(conn, "1h", 3600, &latest)?,
        band(conn, "5h", 5 * 3600, &latest)?,
        band(conn, "24h", 24 * 3600, &latest)?,
        band(conn, "7d", 7 * 24 * 3600, &latest)?,
        band(conn, "30d", 30 * 24 * 3600, &latest)?,
        band(conn, "all", 0, &latest)?,
    ];

    Ok(UsageStats {
        latest: snap,
        bands,
        hourly: buckets(conn, 3600, 48 * 3600)?,
        daily: buckets(conn, 86400, 30 * 86400)?,
        snapshot_count,
        first_ts,
    })
}
