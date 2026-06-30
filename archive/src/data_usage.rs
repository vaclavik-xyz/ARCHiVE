//! Per-process network data usage from `DataUsage.sqlite` (the system network
//! accounting database). Two Core Data tables: `ZPROCESS` (one row per process /
//! bundle) and `ZLIVEUSAGE` (time-windowed byte counters referencing a process).
//! We aggregate the live-usage rows per process into total Wi-Fi / cellular bytes
//! plus the first/last seen times. Schema-tolerant across iOS versions: missing
//! optional columns degrade to zero/empty rather than erroring.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Backup domain + path of the data-usage database.
pub const DOMAIN: &str = "WirelessDomain";
pub const PATH: &str = "Library/Databases/DataUsage.sqlite";

/// Aggregated network usage for one process / app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataUsage {
    /// Process name (`ZPROCNAME`), e.g. `com.apple.WebKit.Networking`.
    pub process: String,
    /// Bundle name (`ZBUNDLENAME`) when recorded; empty otherwise.
    pub bundle: String,
    /// Cellular bytes received / sent (summed across windows).
    pub wwan_in: i64,
    pub wwan_out: i64,
    /// Wi-Fi bytes received / sent (summed across windows).
    pub wifi_in: i64,
    pub wifi_out: i64,
    /// Total cellular / Wi-Fi bytes (in + out); precomputed so every output
    /// format (incl. JSON) carries them.
    pub wwan_total: i64,
    pub wifi_total: i64,
    /// First / last usage-window timestamp, ISO 8601 UTC; empty when unknown.
    pub first_seen: String,
    pub last_seen: String,
}

impl DataUsage {
    /// Human-readable cellular / Wi-Fi totals for the HTML view.
    pub fn wwan_human(&self) -> String {
        human_bytes(self.wwan_total)
    }
    pub fn wifi_human(&self) -> String {
        human_bytes(self.wifi_total)
    }
}

/// Format a byte count as a human-readable size (binary units).
pub fn human_bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// `SUM(col)` when the column exists, else the literal `0` — keeps the aggregate
/// query valid on schemas missing a counter.
fn sum_or_zero(cols: &std::collections::HashSet<String>, col: &str) -> String {
    if cols.contains(col) {
        format!("COALESCE(SUM(l.{col}), 0)")
    } else {
        "0".to_string()
    }
}

/// Parse and aggregate per-process usage, ordered by total bytes descending.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<DataUsage>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let live = table_columns(&conn, "ZLIVEUSAGE")?;
    let proc = table_columns(&conn, "ZPROCESS")?;
    if live.is_empty() || proc.is_empty() {
        return Ok(Vec::new());
    }

    // The live-usage → process foreign key is ZHASPROCESS on every known schema.
    let fk = if live.contains("ZHASPROCESS") { "ZHASPROCESS" } else { return Ok(Vec::new()) };
    let procname = if proc.contains("ZPROCNAME") { "p.ZPROCNAME" } else { "NULL" };
    let bundle = if proc.contains("ZBUNDLENAME") { "p.ZBUNDLENAME" } else { "NULL" };
    let has_ts = live.contains("ZTIMESTAMP");
    let (min_ts, max_ts) = if has_ts { ("MIN(l.ZTIMESTAMP)", "MAX(l.ZTIMESTAMP)") } else { ("NULL", "NULL") };

    let wwan_in = sum_or_zero(&live, "ZWWANIN");
    let wwan_out = sum_or_zero(&live, "ZWWANOUT");
    let wifi_in = sum_or_zero(&live, "ZWIFIIN");
    let wifi_out = sum_or_zero(&live, "ZWIFIOUT");

    // All interpolated names are schema-derived literals, not user input.
    let sql = format!(
        "SELECT {procname}, {bundle}, {wwan_in}, {wwan_out}, {wifi_in}, {wifi_out}, {min_ts}, {max_ts} \
         FROM ZLIVEUSAGE l JOIN ZPROCESS p ON p.Z_PK = l.{fk} \
         GROUP BY p.Z_PK \
         ORDER BY ({wwan_in} + {wwan_out} + {wifi_in} + {wifi_out}) DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let process: Option<String> = row.get(0)?;
        let bundle: Option<String> = row.get(1)?;
        let wwan_in: f64 = row.get(2)?;
        let wwan_out: f64 = row.get(3)?;
        let wifi_in: f64 = row.get(4)?;
        let wifi_out: f64 = row.get(5)?;
        let first: Option<f64> = row.get(6)?;
        let last: Option<f64> = row.get(7)?;
        let (wwan_in, wwan_out) = (wwan_in.round() as i64, wwan_out.round() as i64);
        let (wifi_in, wifi_out) = (wifi_in.round() as i64, wifi_out.round() as i64);
        Ok(DataUsage {
            process: process.unwrap_or_default(),
            bundle: bundle.unwrap_or_default(),
            wwan_in,
            wwan_out,
            wifi_in,
            wifi_out,
            wwan_total: wwan_in + wwan_out,
            wifi_total: wifi_in + wifi_out,
            first_seen: first.and_then(cocoa_to_iso).unwrap_or_default(),
            last_seen: last.and_then(cocoa_to_iso).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

/// Build a customer-facing summary of recovered per-process data usage.
pub fn summary(items: &[DataUsage]) -> crate::summary::Summary {
    use crate::summary::{iso_range, Summary};

    const MB: i64 = 1_048_576;
    let wwan_sum: i64 = items.iter().map(|d| d.wwan_total).sum();
    let wifi_sum: i64 = items.iter().map(|d| d.wifi_total).sum();

    // Biggest consumers: label by bundle (falling back to process), value in
    // whole MB; drop sub-MB rows and keep the top 15 by volume.
    let mut top: Vec<(String, usize)> = items
        .iter()
        .map(|d| {
            let label = if !d.bundle.is_empty() { d.bundle.clone() } else { d.process.clone() };
            (label, ((d.wwan_total + d.wifi_total) / MB) as usize)
        })
        .filter(|(_, mb)| *mb > 0)
        .collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top: Vec<(String, usize)> = top.into_iter().take(15).collect();

    let by_network = vec![
        ("Mobilní data".to_string(), (wwan_sum / MB) as usize),
        ("Wi-Fi data".to_string(), (wifi_sum / MB) as usize),
    ];

    Summary::new("data-usage", "Přenosy dat", "procesů", items.len())
        .count("Celkem dat (MB)", ((wwan_sum + wifi_sum) / MB) as usize)
        .count("Mobilní data (MB)", (wwan_sum / MB) as usize)
        .count("Wi-Fi data (MB)", (wifi_sum / MB) as usize)
        .period_from(iso_range(items.iter().flat_map(|d| [d.first_seen.as_str(), d.last_seen.as_str()])))
        .breakdown("Největší spotřebitelé", top)
        .breakdown("Podle sítě", by_network)
        .note("Čítače jsou jen za nedávné období (iOS je periodicky nuluje), ne celoživotní.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn du(process: &str, bundle: &str, wwan_total: i64, wifi_total: i64, first: &str, last: &str) -> DataUsage {
        DataUsage {
            process: process.into(),
            bundle: bundle.into(),
            wwan_in: wwan_total,
            wwan_out: 0,
            wifi_in: wifi_total,
            wifi_out: 0,
            wwan_total,
            wifi_total,
            first_seen: first.into(),
            last_seen: last.into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        const MB: i64 = 1_048_576;
        let items = vec![
            du("Safari", "com.apple.mobilesafari", 5 * MB, 10 * MB, "2023-05-01T10:00:00+00:00", "2023-06-01T10:00:00+00:00"),
            du("mediaserverd", "", 2 * MB, 0, "2024-01-01T10:00:00+00:00", "2024-02-01T10:00:00+00:00"),
            du("tiny", "", 100, 0, "", ""), // sub-MB → dropped from biggest-consumers
        ];
        let s = summary(&items);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "procesů");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Celkem dat (MB)"), Some(17)); // (7*MB+100 + 10*MB) / MB
        assert_eq!(get("Mobilní data (MB)"), Some(7)); // (5*MB + 2*MB + 100) / MB
        assert_eq!(get("Wi-Fi data (MB)"), Some(10));
        let top = s.breakdowns.iter().find(|b| b.title == "Největší spotřebitelé").unwrap();
        assert_eq!(top.rows[0], ("com.apple.mobilesafari".to_string(), 15)); // 5+10 MB, bundle label
        let net = s.breakdowns.iter().find(|b| b.title == "Podle sítě").unwrap();
        assert_eq!(net.rows, vec![("Mobilní data".to_string(), 7), ("Wi-Fi data".to_string(), 10)]);
        assert!(s.period.is_some()); // derived from the two dated processes
    }

    fn make_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZPROCESS (Z_PK INTEGER PRIMARY KEY, ZPROCNAME TEXT, ZBUNDLENAME TEXT);
             INSERT INTO ZPROCESS VALUES (1, 'mediaserverd', NULL), (2, 'Safari', 'com.apple.mobilesafari');
             CREATE TABLE ZLIVEUSAGE (Z_PK INTEGER PRIMARY KEY, ZHASPROCESS INTEGER,
                ZWWANIN REAL, ZWWANOUT REAL, ZWIFIIN REAL, ZWIFIOUT REAL, ZTIMESTAMP REAL);
             INSERT INTO ZLIVEUSAGE VALUES
                (1, 1, 100, 200, 0, 0, 600000000.0),
                (2, 1, 50, 0, 0, 0, 600000100.0),
                (3, 2, 0, 0, 1000, 500, 600000200.0);",
        )
        .unwrap();
    }

    #[test]
    fn aggregates_per_process_sorted_by_total() {
        let dir = std::env::temp_dir().join(format!("be-du-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("DataUsage.sqlite");
        let _ = std::fs::remove_file(&db);
        make_db(&db);

        let rows = parse(&db).unwrap();
        assert_eq!(rows.len(), 2);
        // Safari has 1500 wifi bytes; mediaserverd 350 wwan → Safari first.
        assert_eq!(rows[0].process, "Safari");
        assert_eq!(rows[0].bundle, "com.apple.mobilesafari");
        assert_eq!(rows[0].wifi_in, 1000);
        assert_eq!(rows[0].wifi_out, 500);
        assert_eq!(rows[0].wifi_total, 1500);
        assert_eq!(rows[0].wwan_total, 0);

        let media = &rows[1];
        assert_eq!(media.process, "mediaserverd");
        assert_eq!(media.bundle, "");
        assert_eq!(media.wwan_in, 150); // 100 + 50
        assert_eq!(media.wwan_out, 200);
        assert_eq!(media.first_seen, "2020-01-06T10:40:00+00:00");
        assert_eq!(media.last_seen, "2020-01-06T10:41:40+00:00"); // +100s
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_includes_precomputed_totals() {
        let dir = std::env::temp_dir().join(format!("be-du-json-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("DataUsage.sqlite");
        let _ = std::fs::remove_file(&db);
        make_db(&db);
        let rows = parse(&db).unwrap();
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
        assert_eq!(v[0]["wifi_total"], 1500);
        assert_eq!(v[0]["wwan_total"], 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn human_bytes_scales_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn missing_tables_yield_empty() {
        let dir = std::env::temp_dir().join(format!("be-du-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("DataUsage.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE Unrelated (x INTEGER);").unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
