//! Read Safari browsing history and bookmarks from the on-backup SQLite stores.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One Safari history visit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryVisit {
    /// Visited URL.
    pub url: String,
    /// Page title at visit time; empty when unknown.
    pub title: String,
    /// Visit time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub date: String,
    /// Total visit count for the URL (per `history_items`).
    pub visit_count: i64,
}

/// One Safari bookmark (leaf, i.e. has a URL).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bookmark {
    /// Bookmark title.
    pub title: String,
    /// Bookmarked URL.
    pub url: String,
    /// Containing folder's title; empty when at the root or unresolved.
    pub folder: String,
}

/// Parse Safari history: one record per visit, URL joined from `history_items`,
/// ordered by visit time. Tolerates missing optional columns across iOS versions.
pub fn parse_history(db_path: &Path) -> rusqlite::Result<Vec<HistoryVisit>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let hv_cols = table_columns(&conn, "history_visits")?;
    let hi_cols = table_columns(&conn, "history_items")?;
    let title_sel = if hv_cols.contains("title") { "hv.title" } else { "NULL" };
    let vc_sel = if hi_cols.contains("visit_count") { "hi.visit_count" } else { "NULL" };

    let sql = format!(
        "SELECT hi.url, {title_sel}, hv.visit_time, {vc_sel} \
         FROM history_visits hv JOIN history_items hi ON hi.id = hv.history_item \
         ORDER BY hv.visit_time"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let url: Option<String> = row.get(0)?;
        let title: Option<String> = row.get(1)?;
        let visit_time: Option<f64> = row.get(2)?;
        let visit_count: Option<i64> = row.get(3)?;
        Ok(HistoryVisit {
            url: url.unwrap_or_default(),
            title: title.unwrap_or_default(),
            date: visit_time.and_then(cocoa_to_iso).unwrap_or_default(),
            visit_count: visit_count.unwrap_or(0),
        })
    })?;
    rows.collect()
}

/// Parse Safari bookmarks: one record per leaf bookmark (has a URL), with the
/// containing folder resolved from the bookmark's `parent` title.
pub fn parse_bookmarks(db_path: &Path) -> rusqlite::Result<Vec<Bookmark>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let sql = "SELECT b.title, b.url, p.title \
         FROM bookmarks b LEFT JOIN bookmarks p ON p.id = b.parent \
         WHERE b.url IS NOT NULL AND b.url <> '' ORDER BY b.id";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let title: Option<String> = row.get(0)?;
        let url: Option<String> = row.get(1)?;
        let folder: Option<String> = row.get(2)?;
        Ok(Bookmark {
            title: title.unwrap_or_default(),
            url: url.unwrap_or_default(),
            folder: folder.unwrap_or_default(),
        })
    })?;
    rows.collect()
}

/// Host (authority) of a URL: drop the `scheme://` prefix, then take everything
/// up to the first `/`, `:` (port) or `?`. Empty when the URL has no authority.
fn host(url: &str) -> String {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    after_scheme
        .split(['/', ':', '?'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Build a customer-facing summary of the recovered Safari history.
pub fn summary(items: &[HistoryVisit]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};
    use std::collections::HashSet;

    let pages = items.iter().map(|v| v.url.clone()).collect::<HashSet<_>>().len();
    let sites = items
        .iter()
        .map(|v| host(&v.url))
        .filter(|h| !h.is_empty())
        .collect::<HashSet<_>>()
        .len();
    let dated = items.iter().filter(|v| !v.date.is_empty()).count();
    let top_sites: Vec<(String, usize)> = tally(items.iter().map(|v| host(&v.url)).filter(|h| !h.is_empty()))
        .into_iter()
        .take(15)
        .collect();

    Summary::new("safari-history", "Historie Safari", "návštěv", items.len())
        .count("Unikátních stránek", pages)
        .count("Unikátních webů", sites)
        .count("S datem", dated)
        .period_from(iso_range(items.iter().map(|v| v.date.as_str())))
        .breakdown("Po letech", year_rows(items.iter().map(|v| v.date.as_str())))
        .breakdown("Nejnavštěvovanější weby", top_sites)
        .note("Návštěva = jedno otevření stránky; web se může opakovat. Historii lze na zařízení mazat.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{make_safari_bookmarks, make_safari_history};

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("be-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn visit(url: &str, title: &str, date: &str, vc: i64) -> HistoryVisit {
        HistoryVisit { url: url.into(), title: title.into(), date: date.into(), visit_count: vc }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let visits = vec![
            visit("https://apple.com", "Apple", "2023-05-01T10:00:00+00:00", 5),
            visit("https://apple.com/store", "Store", "2024-06-01T10:00:00+00:00", 2),
            visit("https://bbc.com/news", "BBC News", "", 1),
        ];
        let s = summary(&visits);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "návštěv");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Unikátních stránek"), Some(3)); // three distinct URLs
        assert_eq!(get("Unikátních webů"), Some(2)); // apple.com, bbc.com
        assert_eq!(get("S datem"), Some(2)); // two dated visits
        let sites = s.breakdowns.iter().find(|b| b.title == "Nejnavštěvovanější weby").unwrap();
        assert_eq!(sites.rows[0], ("apple.com".to_string(), 2)); // host() collapses both apple paths
        assert!(s.period.is_some()); // derived from the two dated visits
    }

    #[test]
    fn parses_history_joined_and_ordered() {
        let dir = tmp("safari-hist");
        let db = dir.join("History.db");
        let _ = std::fs::remove_file(&db);
        make_safari_history(&db);

        let visits = parse_history(&db).unwrap();
        assert_eq!(visits.len(), 2);
        // Ordered by visit_time ascending.
        assert_eq!(visits[0].url, "https://apple.com");
        assert_eq!(visits[0].title, "Apple");
        assert_eq!(visits[0].date, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000
        assert_eq!(visits[0].visit_count, 5);
        assert_eq!(visits[1].url, "https://bbc.com");
        assert_eq!(visits[1].title, "BBC News");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_bookmarks_resolving_folder_and_skipping_folders() {
        let dir = tmp("safari-bm");
        let db = dir.join("Bookmarks.db");
        let _ = std::fs::remove_file(&db);
        make_safari_bookmarks(&db);

        let bms = parse_bookmarks(&db).unwrap();
        assert_eq!(bms.len(), 2); // only leaf bookmarks, not the two folders
        assert_eq!(bms[0].title, "Apple");
        assert_eq!(bms[0].url, "https://apple.com");
        assert_eq!(bms[0].folder, "Favorites");
        assert_eq!(bms[1].title, "BBC");
        assert_eq!(bms[1].folder, "News");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_history_without_optional_columns() {
        let dir = tmp("safari-hist-min");
        let db = dir.join("History.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE history_items (id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE history_visits (id INTEGER PRIMARY KEY, history_item INTEGER, visit_time REAL);
             INSERT INTO history_items VALUES (1, 'https://x.com');
             INSERT INTO history_visits VALUES (1, 1, 600000000.0);",
        )
        .unwrap();
        drop(conn);

        let visits = parse_history(&db).unwrap();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].url, "https://x.com");
        assert_eq!(visits[0].title, ""); // no title column
        assert_eq!(visits[0].visit_count, 0); // no visit_count column
        std::fs::remove_dir_all(&dir).ok();
    }
}
