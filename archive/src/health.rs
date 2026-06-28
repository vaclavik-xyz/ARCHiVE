//! Read Apple Health (HealthKit) workouts and quantity summaries from an iOS
//! `healthdb_secure.sqlite`.
//!
//! Once a backup is decrypted this is an ordinary SQLite file. The HealthKit
//! schema is not publicly documented and drifts across iOS versions, so every
//! table and version-dependent column is discovered at runtime (`sqlite_master`
//! / `PRAGMA table_info`) and selected as column-or-`NULL`. A missing table
//! yields an empty section rather than an error.
//!
//! Layout (as reverse-engineered by the forensics/OSS community):
//! - `samples` — one row per measurement: `data_id` (PK), `data_type` (the
//!   numeric HK type id), `start_date`/`end_date` (Cocoa/2001 REAL seconds).
//! - `quantity_samples` — `data_id` → `samples.data_id`, `quantity` (the value
//!   in HealthKit canonical units).
//! - `workouts` — `data_id` → `samples.data_id`, `total_distance`. Older iOS
//!   denormalized more here (`activity_type`, `duration`, `total_energy_burned`,
//!   …); iOS 16+ split those into `workout_activities`.
//! - `workout_activities` — `owner_id` → `workouts.data_id`, `activity_type`,
//!   `duration`, `start_date`/`end_date` (modern split-out activity rows).
//!
//! Dates are Cocoa/2001-epoch REAL seconds → [`crate::datetime::cocoa_to_iso`].
//!
//! This module is the data layer only; wiring into the CLI lives elsewhere.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Backup location of the secure Health DB (holds samples/workouts). The
/// non-secure `Health/healthdb.sqlite` exists too but carries little of value.
pub const DB_LOCATIONS: [(&str, &str); 1] =
    [("HealthDomain", "Health/healthdb_secure.sqlite")];

/// One completed workout. Version-dependent fields are `None` when the source
/// column/table is absent for this iOS version.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Workout {
    /// Raw HealthKit `activity_type` id (kept even when unmapped). `None` when
    /// the backup exposes no activity type for this workout.
    pub activity_type_id: Option<i64>,
    /// Friendly name for `activity_type_id` from [`ACTIVITY_TYPES`]; `None` for
    /// unknown ids (the raw id is still in `activity_type_id`).
    pub activity_type: Option<String>,
    /// Start time as ISO 8601 UTC (Cocoa epoch); empty if unset/unconvertible.
    pub start: String,
    /// End time as ISO 8601 UTC (Cocoa epoch); empty if unset/unconvertible.
    pub end: String,
    /// Duration in seconds (rounded); `None` when not derivable.
    pub duration_seconds: Option<i64>,
    /// Total distance in HealthKit canonical units (meters); `None` when absent.
    pub total_distance: Option<f64>,
    /// Total active energy burned in canonical units (kcal); `None` when the
    /// column is absent (iOS 16+ no longer stores it on `workouts`).
    pub total_energy_burned: Option<f64>,
}

/// Aggregated summary for one known quantity type across the whole DB.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuantitySummary {
    /// Raw HealthKit `data_type` id (e.g. 7 = step count).
    pub data_type_id: i64,
    /// Friendly name for `data_type_id` from [`QUANTITY_TYPES`].
    pub name: String,
    /// Number of samples of this type.
    pub count: i64,
    /// Sum of all sample quantities (meaningful for cumulative types such as
    /// step count / distance / energy). `None` when no numeric quantity existed.
    pub sum: Option<f64>,
    /// Minimum sample quantity (useful for instantaneous types like heart rate).
    pub min: Option<f64>,
    /// Mean sample quantity.
    pub avg: Option<f64>,
    /// Maximum sample quantity.
    pub max: Option<f64>,
    /// Earliest sample start as ISO 8601 UTC; empty when unknown.
    pub first: String,
    /// Latest sample end as ISO 8601 UTC; empty when unknown.
    pub last: String,
}

/// The whole Health extraction: workouts plus per-type quantity summaries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthData {
    /// Completed workouts, ordered by start time.
    pub workouts: Vec<Workout>,
    /// One summary per known quantity type that has at least one sample,
    /// ordered by `data_type_id`.
    pub quantity_summary: Vec<QuantitySummary>,
}

/// Known HealthKit quantity `data_type` ids → friendly name. Ids are the
/// reverse-engineered `samples.data_type` values (stable across iOS versions).
/// Deliberately small and bounded; unknown/over-large types are skipped.
pub const QUANTITY_TYPES: &[(i64, &str)] = &[
    (5, "heart_rate"),
    (7, "step_count"),
    (8, "distance_walking_running"),
    (9, "basal_energy_burned"),
    (10, "active_energy_burned"),
    (12, "flights_climbed"),
];

/// Quantity ids treated as instantaneous (min/avg/max are meaningful, the sum
/// is not). Everything else in [`QUANTITY_TYPES`] is cumulative.
const INSTANTANEOUS_TYPES: &[i64] = &[5 /* heart_rate */];

/// Known `HKWorkoutActivityType` raw values → friendly name. These are the
/// stable Apple SDK enum values; unknown ids stay numeric in `activity_type_id`.
/// A representative common subset (the enum has ~80 members); kept bounded.
pub const ACTIVITY_TYPES: &[(i64, &str)] = &[
    (13, "cycling"),
    (16, "elliptical"),
    (20, "functional_strength_training"),
    (24, "hiking"),
    (35, "rowing"),
    (37, "running"),
    (46, "swimming"),
    (50, "traditional_strength_training"),
    (52, "walking"),
    (57, "yoga"),
    (59, "core_training"),
    (63, "high_intensity_interval_training"),
    (3000, "other"),
];

/// Friendly name for a HealthKit quantity `data_type` id, if known.
fn quantity_type_name(id: i64) -> Option<&'static str> {
    QUANTITY_TYPES.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

/// Friendly name for an `HKWorkoutActivityType` raw value, if known.
fn activity_type_name(id: i64) -> Option<&'static str> {
    ACTIVITY_TYPES.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

/// True when `table` exists in the database (via `sqlite_master`).
fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// Parse the Health store: workouts + a bounded per-type quantity summary.
/// Schema-tolerant — a missing `samples`/`workouts` table yields an empty
/// section, and every version-dependent column is selected only when present.
pub fn parse(db_path: &Path) -> rusqlite::Result<HealthData> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let workouts = parse_workouts(&conn)?;
    let quantity_summary = parse_quantity_summary(&conn)?;
    Ok(HealthData { workouts, quantity_summary })
}

/// Read every workout. Combines three sources, each tolerated when absent:
/// - `workouts` (always, when it exists): `data_id`, `total_distance`, and the
///   legacy denormalized columns (`activity_type`, `duration`,
///   `total_energy_burned`, `start_date`, `end_date`) when this iOS kept them;
/// - `samples` (LEFT JOIN on `data_id`): canonical `start_date`/`end_date`;
/// - `workout_activities` (LEFT JOIN on `owner_id`, iOS 16+): the split-out
///   `activity_type`/`duration`/`start_date`/`end_date`.
///
/// Per field, the legacy `workouts` value wins when present, else the modern
/// `workout_activities`/`samples` value is used.
fn parse_workouts(conn: &Connection) -> rusqlite::Result<Vec<Workout>> {
    if !table_exists(conn, "workouts")? {
        return Ok(Vec::new());
    }
    let w = table_columns(conn, "workouts")?;
    let has_samples = table_exists(conn, "samples")?;
    let has_acts = table_exists(conn, "workout_activities")?;
    let a = if has_acts { table_columns(conn, "workout_activities")? } else { Default::default() };

    // `workouts` columns (legacy ones are version-dependent → select-or-NULL).
    let w_distance = sel(&w, "w.total_distance", "total_distance");
    let w_energy = sel(&w, "w.total_energy_burned", "total_energy_burned");
    let w_activity = sel(&w, "w.activity_type", "activity_type");
    let w_duration = sel(&w, "w.duration", "duration");
    let w_start = sel(&w, "w.start_date", "start_date");
    let w_end = sel(&w, "w.end_date", "end_date");

    // `samples` dates (only when the table exists).
    let (s_join, s_start, s_end) = if has_samples {
        (" LEFT JOIN samples s ON s.data_id = w.data_id", "s.start_date", "s.end_date")
    } else {
        ("", "NULL", "NULL")
    };

    // `workout_activities` (iOS 16+). One owner can have several activity rows;
    // pick the primary/earliest deterministically with MIN(...) GROUP BY owner.
    let (a_join, a_activity, a_duration, a_start, a_end) = if has_acts {
        (
            " LEFT JOIN (SELECT owner_id, \
               MIN(ROWID) AS _r, \
               MIN(start_date) AS a_start, \
               MAX(end_date) AS a_end, \
               SUM(duration) AS a_duration, \
               MIN(activity_type) AS a_activity \
             FROM workout_activities GROUP BY owner_id) wa ON wa.owner_id = w.data_id",
            if a.contains("activity_type") { "wa.a_activity" } else { "NULL" },
            if a.contains("duration") { "wa.a_duration" } else { "NULL" },
            if a.contains("start_date") { "wa.a_start" } else { "NULL" },
            if a.contains("end_date") { "wa.a_end" } else { "NULL" },
        )
    } else {
        ("", "NULL", "NULL", "NULL", "NULL")
    };

    // Order by whichever start is available (legacy workouts, then samples, then
    // activities) so output is stable regardless of schema generation.
    let order = format!("COALESCE({w_start}, {s_start}, {a_start})");

    let sql = format!(
        "SELECT \
           {w_activity} AS w_activity, {a_activity} AS a_activity, \
           {w_start} AS w_start, {s_start} AS s_start, {a_start} AS a_start, \
           {w_end} AS w_end, {s_end} AS s_end, {a_end} AS a_end, \
           {w_duration} AS w_duration, {a_duration} AS a_duration, \
           {w_distance} AS distance, {w_energy} AS energy \
         FROM workouts w{s_join}{a_join} \
         ORDER BY {order}"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let w_activity: Option<i64> = row.get("w_activity")?;
        let a_activity: Option<i64> = row.get("a_activity")?;
        let w_start: Option<f64> = row.get("w_start")?;
        let s_start: Option<f64> = row.get("s_start")?;
        let a_start: Option<f64> = row.get("a_start")?;
        let w_end: Option<f64> = row.get("w_end")?;
        let s_end: Option<f64> = row.get("s_end")?;
        let a_end: Option<f64> = row.get("a_end")?;
        let w_duration: Option<f64> = row.get("w_duration")?;
        let a_duration: Option<f64> = row.get("a_duration")?;
        let distance: Option<f64> = row.get("distance")?;
        let energy: Option<f64> = row.get("energy")?;

        // Legacy `workouts` value wins; else fall back to the modern source.
        let activity_type_id = w_activity.or(a_activity);
        let start = w_start.or(s_start).or(a_start);
        let end = w_end.or(s_end).or(a_end);
        let duration = w_duration.or(a_duration);

        Ok(Workout {
            activity_type_id,
            activity_type: activity_type_id
                .and_then(activity_type_name)
                .map(str::to_string),
            start: start.and_then(cocoa_to_iso).unwrap_or_default(),
            end: end.and_then(cocoa_to_iso).unwrap_or_default(),
            duration_seconds: duration.map(|d| d.round() as i64),
            total_distance: distance,
            total_energy_burned: energy,
        })
    })?;
    rows.collect()
}

/// Accumulator for one quantity type's running aggregates.
#[derive(Default)]
struct Agg {
    count: i64,
    sum: f64,
    have_value: bool,
    min: f64,
    max: f64,
    first: Option<f64>,
    last: Option<f64>,
}

impl Agg {
    fn push(&mut self, quantity: Option<f64>, start: Option<f64>, end: Option<f64>) {
        self.count += 1;
        if let Some(q) = quantity.filter(|q| q.is_finite()) {
            if !self.have_value {
                self.min = q;
                self.max = q;
                self.have_value = true;
            } else {
                self.min = self.min.min(q);
                self.max = self.max.max(q);
            }
            self.sum += q;
        }
        if let Some(s) = start.filter(|s| s.is_finite()) {
            self.first = Some(self.first.map_or(s, |f| f.min(s)));
        }
        // Range end tracks the latest end, falling back to start when end is unset.
        if let Some(e) = end.or(start).filter(|e| e.is_finite()) {
            self.last = Some(self.last.map_or(e, |l| l.max(e)));
        }
    }
}

/// Aggregate the known quantity types across the whole DB. Requires both
/// `samples` and `quantity_samples`; either missing yields an empty section.
/// Only the bounded [`QUANTITY_TYPES`] set is scanned (over-large/unknown types
/// are skipped). Cumulative types report `sum`; instantaneous report min/avg/max.
fn parse_quantity_summary(conn: &Connection) -> rusqlite::Result<Vec<QuantitySummary>> {
    if !table_exists(conn, "samples")? || !table_exists(conn, "quantity_samples")? {
        return Ok(Vec::new());
    }
    let s = table_columns(conn, "samples")?;
    // `data_type`, `start_date`, `end_date` are the long-stable `samples`
    // columns, but stay tolerant: select-or-NULL keeps a quirky schema parsing.
    let s_type = sel(&s, "s.data_type", "data_type");
    let s_start = sel(&s, "s.start_date", "start_date");
    let s_end = sel(&s, "s.end_date", "end_date");

    // Restrict to the known ids so the scan stays bounded.
    let ids: Vec<String> = QUANTITY_TYPES.iter().map(|(id, _)| id.to_string()).collect();
    let in_list = ids.join(", ");

    let sql = format!(
        "SELECT {s_type} AS data_type, q.quantity AS quantity, \
                {s_start} AS start_date, {s_end} AS end_date \
         FROM samples s JOIN quantity_samples q ON q.data_id = s.data_id \
         WHERE {s_type} IN ({in_list})"
    );

    let mut by_type: BTreeMap<i64, Agg> = BTreeMap::new();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let data_type: Option<i64> = row.get("data_type")?;
        let Some(data_type) = data_type else { continue };
        let quantity: Option<f64> = row.get("quantity")?;
        let start: Option<f64> = row.get("start_date")?;
        let end: Option<f64> = row.get("end_date")?;
        by_type.entry(data_type).or_default().push(quantity, start, end);
    }

    let mut out = Vec::new();
    for (id, agg) in by_type {
        let Some(name) = quantity_type_name(id) else { continue };
        let instantaneous = INSTANTANEOUS_TYPES.contains(&id);
        let avg = if agg.count > 0 && agg.have_value {
            Some(agg.sum / agg.count as f64)
        } else {
            None
        };
        out.push(QuantitySummary {
            data_type_id: id,
            name: name.to_string(),
            count: agg.count,
            // Sum is meaningless for instantaneous types (e.g. heart rate).
            sum: if instantaneous || !agg.have_value { None } else { Some(agg.sum) },
            min: agg.have_value.then_some(agg.min),
            avg,
            max: agg.have_value.then_some(agg.max),
            first: agg.first.and_then(cocoa_to_iso).unwrap_or_default(),
            last: agg.last.and_then(cocoa_to_iso).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// `expr` when `col` is present in `cols`, else the literal `NULL` — the
/// select-column-or-NULL idiom for version-dependent columns. `expr`/`col` are
/// trusted compile-time literals (column references), never user input.
fn sel(cols: &std::collections::HashSet<String>, expr: &str, col: &str) -> &'static str {
    if cols.contains(col) { intern(expr) } else { "NULL" }
}

/// Map a known column reference literal to its `&'static str` form. Only the
/// fixed set of references this module emits is recognized; anything else is a
/// programmer error and falls back to `NULL` rather than leaking memory.
fn intern(expr: &str) -> &'static str {
    match expr {
        "w.total_distance" => "w.total_distance",
        "w.total_energy_burned" => "w.total_energy_burned",
        "w.activity_type" => "w.activity_type",
        "w.duration" => "w.duration",
        "w.start_date" => "w.start_date",
        "w.end_date" => "w.end_date",
        "s.data_type" => "s.data_type",
        "s.start_date" => "s.start_date",
        "s.end_date" => "s.end_date",
        _ => "NULL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic modern-ish Health DB: a `samples` table (dates + type),
    /// `quantity_samples`, a `workouts` table, and a `workout_activities` table
    /// (iOS 16+ split). Cocoa/2001 REAL seconds throughout.
    fn make_health(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE samples (data_id INTEGER PRIMARY KEY, start_date REAL, end_date REAL, data_type INTEGER);
             CREATE TABLE quantity_samples (data_id INTEGER PRIMARY KEY, quantity REAL, original_quantity REAL, original_unit INTEGER);
             CREATE TABLE workouts (data_id INTEGER PRIMARY KEY, total_distance REAL, goal_type INTEGER, goal REAL);
             CREATE TABLE workout_activities (
                ROWID INTEGER PRIMARY KEY, owner_id INTEGER, activity_type INTEGER,
                duration REAL, start_date REAL, end_date REAL);

             -- Two workouts (data_type 79). Dates live in samples; activity in workout_activities.
             INSERT INTO samples (data_id, start_date, end_date, data_type) VALUES
                (1000, 600000000.0, 600001800.0, 79),
                (1001, 600100000.0, 600103600.0, 79);
             INSERT INTO workouts (data_id, total_distance) VALUES
                (1000, 5000.0),
                (1001, 10000.0);
             INSERT INTO workout_activities (ROWID, owner_id, activity_type, duration, start_date, end_date) VALUES
                (1, 1000, 37, 1800.0, 600000000.0, 600001800.0),   -- running
                (2, 1001, 9999, 3600.0, 600100000.0, 600103600.0); -- unknown activity id

             -- Quantity samples: step_count (7) cumulative, heart_rate (5) instantaneous.
             INSERT INTO samples (data_id, start_date, end_date, data_type) VALUES
                (1, 600000000.0, 600000060.0, 7),
                (2, 600000100.0, 600000160.0, 7),
                (3, 600000000.0, 600000000.0, 5),
                (4, 600000100.0, 600000100.0, 5),
                (5, 600000200.0, 600000200.0, 5),
                (9, 600000300.0, 600000300.0, 999);  -- over-large/unknown → skipped
             INSERT INTO quantity_samples (data_id, quantity) VALUES
                (1, 100.0),
                (2, 150.0),
                (3, 60.0),
                (4, 80.0),
                (5, 70.0),
                (9, 1.0);",
        )
        .unwrap();
    }

    #[test]
    fn parses_workouts_with_modern_split_schema() {
        let dir = std::env::temp_dir().join(format!("be-health-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("healthdb_secure.sqlite");
        let _ = std::fs::remove_file(&db);
        make_health(&db);

        let data = parse(&db).unwrap();
        assert_eq!(data.workouts.len(), 2);

        let w0 = &data.workouts[0];
        assert_eq!(w0.activity_type_id, Some(37));
        assert_eq!(w0.activity_type.as_deref(), Some("running"));
        assert_eq!(w0.start, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000
        assert_eq!(w0.end, "2020-01-06T11:10:00+00:00"); // +1800s
        assert_eq!(w0.duration_seconds, Some(1800));
        assert_eq!(w0.total_distance, Some(5000.0));
        assert_eq!(w0.total_energy_burned, None); // no column in modern schema

        // Unknown activity id stays numeric, name is None.
        let w1 = &data.workouts[1];
        assert_eq!(w1.activity_type_id, Some(9999));
        assert_eq!(w1.activity_type, None);
        assert_eq!(w1.total_distance, Some(10000.0));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn aggregates_known_quantity_types() {
        let dir = std::env::temp_dir().join(format!("be-health-q-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("healthdb_secure.sqlite");
        let _ = std::fs::remove_file(&db);
        make_health(&db);

        let data = parse(&db).unwrap();
        // Ordered by data_type_id: heart_rate (5), step_count (7). Type 999 skipped.
        assert_eq!(data.quantity_summary.len(), 2);

        let hr = &data.quantity_summary[0];
        assert_eq!(hr.data_type_id, 5);
        assert_eq!(hr.name, "heart_rate");
        assert_eq!(hr.count, 3);
        assert_eq!(hr.sum, None); // instantaneous → no sum
        assert_eq!(hr.min, Some(60.0));
        assert_eq!(hr.max, Some(80.0));
        assert_eq!(hr.avg, Some(70.0)); // (60+80+70)/3
        assert_eq!(hr.first, "2020-01-06T10:40:00+00:00"); // 600_000_000
        assert_eq!(hr.last, "2020-01-06T10:43:20+00:00"); // 600_000_200

        let steps = &data.quantity_summary[1];
        assert_eq!(steps.data_type_id, 7);
        assert_eq!(steps.name, "step_count");
        assert_eq!(steps.count, 2);
        assert_eq!(steps.sum, Some(250.0)); // cumulative → 100 + 150
        assert_eq!(steps.avg, Some(125.0));
        assert_eq!(steps.first, "2020-01-06T10:40:00+00:00");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_legacy_denormalized_workouts() {
        // Older iOS kept everything on `workouts`, with no workout_activities and
        // dates duplicated on the workout row.
        let dir = std::env::temp_dir().join(format!("be-health-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("healthdb_secure.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE samples (data_id INTEGER PRIMARY KEY, start_date REAL, end_date REAL, data_type INTEGER);
             CREATE TABLE quantity_samples (data_id INTEGER PRIMARY KEY, quantity REAL);
             CREATE TABLE workouts (
                data_id INTEGER PRIMARY KEY, activity_type INTEGER, duration REAL,
                total_distance REAL, total_energy_burned REAL, start_date REAL, end_date REAL);
             INSERT INTO samples (data_id, start_date, end_date, data_type) VALUES (1, 600000000.0, 600001800.0, 79);
             INSERT INTO workouts (data_id, activity_type, duration, total_distance, total_energy_burned, start_date, end_date)
                VALUES (1, 52, 1800.0, 2000.0, 150.5, 600000000.0, 600001800.0);",
        )
        .unwrap();
        drop(conn);

        let data = parse(&db).unwrap();
        assert_eq!(data.workouts.len(), 1);
        let w = &data.workouts[0];
        assert_eq!(w.activity_type_id, Some(52));
        assert_eq!(w.activity_type.as_deref(), Some("walking"));
        assert_eq!(w.duration_seconds, Some(1800));
        assert_eq!(w.total_distance, Some(2000.0));
        assert_eq!(w.total_energy_burned, Some(150.5)); // legacy column present
        assert_eq!(w.start, "2020-01-06T10:40:00+00:00");
        // No quantity types present → empty summary.
        assert!(data.quantity_summary.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tolerates_missing_optional_columns_and_tables() {
        // A minimal `workouts` with ONLY data_id + total_distance, no
        // workout_activities table, and no samples table at all. Must still parse
        // (distance present, everything else None/empty) and yield empty summary.
        let dir = std::env::temp_dir().join(format!("be-health-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("healthdb_secure.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE workouts (data_id INTEGER PRIMARY KEY, total_distance REAL);
             INSERT INTO workouts (data_id, total_distance) VALUES (1, 1234.5);",
        )
        .unwrap();
        drop(conn);

        let data = parse(&db).unwrap();
        assert_eq!(data.workouts.len(), 1);
        let w = &data.workouts[0];
        assert_eq!(w.activity_type_id, None);
        assert_eq!(w.activity_type, None);
        assert_eq!(w.start, ""); // no dates anywhere
        assert_eq!(w.end, "");
        assert_eq!(w.duration_seconds, None);
        assert_eq!(w.total_distance, Some(1234.5));
        assert_eq!(w.total_energy_burned, None);
        assert!(data.quantity_summary.is_empty()); // no samples/quantity_samples
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_when_no_workouts_table() {
        // Database without a `workouts` table → empty workouts section, no error.
        let dir = std::env::temp_dir().join(format!("be-health-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("healthdb_secure.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE unrelated (x INTEGER);").unwrap();
        drop(conn);

        let data = parse(&db).unwrap();
        assert!(data.workouts.is_empty());
        assert!(data.quantity_summary.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
