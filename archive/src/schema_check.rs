//! Self-check that a backup's live SQLite stores still carry the columns each
//! extractor depends on. iOS point releases occasionally rename, move or drop a
//! column; when that happens the affected extractor silently returns *empty*
//! rather than erroring, which is indistinguishable from "the user had no data".
//! This command surfaces the drift up front: for every store the tool reads, it
//! compares the live schema against the columns that store's extractor needs and
//! reports any that are missing — so an empty export can be explained as either a
//! genuine absence or a schema change.
//!
//! The expectations ([`EXPECTATIONS`]) are a static, hand-maintained mirror of the
//! `SELECT`s in the sibling extractor modules. They are deliberately limited to
//! the *load-bearing* columns (anchors, dates, the values rendered into output);
//! incidental columns are omitted so a cosmetic schema addition is never flagged.

use std::collections::HashSet;

use serde::Serialize;

/// One table an extractor depends on, named with the columns it reads.
pub struct TableNeed {
    pub table: &'static str,
    pub columns: &'static [&'static str],
}

/// A store the tool extracts from: which command consumes it, where the database
/// lives in the backup (`domain` + `rel_path`, resolved via the manifest), and the
/// tables/columns that command needs to function.
pub struct StoreSchema {
    pub command: &'static str,
    pub domain: &'static str,
    pub rel_path: &'static str,
    pub needs: &'static [TableNeed],
}

/// Per-table drift result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableReport {
    pub table: String,
    /// `ok` | `missing_columns` | `table_absent`.
    pub status: &'static str,
    /// Expected columns not found in the live schema (empty when `ok`).
    pub missing_columns: Vec<String>,
}

/// Per-store schema-check result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoreReport {
    pub command: String,
    pub domain: String,
    pub rel_path: String,
    /// `ok` (all needed columns present) | `drifted` (a table/column is missing) |
    /// `db_absent` (the database is not in this backup — not a drift, just absent).
    pub status: &'static str,
    pub tables: Vec<TableReport>,
}

/// Compare one table's required columns against its actual column set. `actual` is
/// `None` when the table does not exist in the database (a `PRAGMA table_info` that
/// returns no rows), in which case every needed column is reported missing.
pub fn check_table(need: &TableNeed, actual: Option<&HashSet<String>>) -> TableReport {
    match actual {
        None => TableReport {
            table: need.table.into(),
            status: "table_absent",
            missing_columns: need.columns.iter().map(|c| (*c).to_string()).collect(),
        },
        Some(cols) => {
            let missing: Vec<String> =
                need.columns.iter().filter(|c| !cols.contains(**c)).map(|c| (*c).to_string()).collect();
            let status = if missing.is_empty() { "ok" } else { "missing_columns" };
            TableReport { table: need.table.into(), status, missing_columns: missing }
        }
    }
}

/// Roll per-table reports up into a store status: `ok` only if every table is `ok`.
pub fn store_status(tables: &[TableReport]) -> &'static str {
    if tables.iter().all(|t| t.status == "ok") { "ok" } else { "drifted" }
}

/// Every SQLite store the tool extracts from, with its load-bearing schema. Kept
/// in sync by hand with the extractor modules' `SELECT`s and the canonical
/// domain/paths in `KNOWN_STORES`. Columns listed here are the stable core present
/// across the supported iOS range — version-conditional extras the extractors
/// already tolerate are deliberately omitted so a cosmetic schema change is never
/// flagged as drift.
pub const EXPECTATIONS: &[StoreSchema] = &[
    StoreSchema {
        command: "contacts",
        domain: "HomeDomain",
        rel_path: "Library/AddressBook/AddressBook.sqlitedb",
        needs: &[
            TableNeed { table: "ABPerson", columns: &["ROWID", "First", "Last", "Organization"] },
            TableNeed { table: "ABMultiValue", columns: &["record_id", "property", "value", "label"] },
            // ABMultiValueLabel has no named ROWID column (the join uses SQLite's
            // implicit rowid, which PRAGMA table_info does not list), only `value`.
            TableNeed { table: "ABMultiValueLabel", columns: &["value"] },
        ],
    },
    StoreSchema {
        command: "calls",
        domain: "HomeDomain",
        rel_path: "Library/CallHistoryDB/CallHistory.storedata",
        needs: &[TableNeed {
            table: "ZCALLRECORD",
            columns: &["Z_PK", "ZADDRESS", "ZDATE", "ZDURATION", "ZORIGINATED", "ZANSWERED", "ZCALLTYPE"],
        }],
    },
    StoreSchema {
        command: "accounts",
        domain: "HomeDomain",
        rel_path: "Library/Accounts/Accounts3.sqlite",
        needs: &[
            TableNeed { table: "ZACCOUNT", columns: &["Z_PK", "ZUSERNAME", "ZACCOUNTDESCRIPTION", "ZACCOUNTTYPE"] },
            TableNeed { table: "ZACCOUNTTYPE", columns: &["Z_PK", "ZACCOUNTTYPEDESCRIPTION"] },
        ],
    },
    StoreSchema {
        command: "data-usage",
        domain: "WirelessDomain",
        rel_path: "Library/Databases/DataUsage.sqlite",
        needs: &[
            TableNeed {
                table: "ZLIVEUSAGE",
                columns: &["Z_PK", "ZHASPROCESS", "ZWWANIN", "ZWWANOUT", "ZWIFIIN", "ZWIFIOUT", "ZTIMESTAMP"],
            },
            TableNeed { table: "ZPROCESS", columns: &["Z_PK", "ZPROCNAME", "ZBUNDLENAME"] },
        ],
    },
    StoreSchema {
        command: "voicemail",
        domain: "HomeDomain",
        rel_path: "Library/Voicemail/voicemail.db",
        needs: &[TableNeed {
            table: "voicemail",
            columns: &["ROWID", "sender", "date", "duration", "trashed_date", "flags"],
        }],
    },
    StoreSchema {
        command: "voice-memos",
        domain: "AppDomainGroup-group.com.apple.VoiceMemos",
        rel_path: "Recordings/CloudRecordings.db",
        needs: &[TableNeed { table: "ZCLOUDRECORDING", columns: &["Z_PK", "ZDATE", "ZDURATION"] }],
    },
    StoreSchema {
        command: "safari-history",
        domain: "AppDomain-com.apple.mobilesafari",
        rel_path: "Library/Safari/History.db",
        needs: &[
            TableNeed { table: "history_visits", columns: &["id", "history_item", "visit_time", "title"] },
            TableNeed { table: "history_items", columns: &["id", "url", "visit_count"] },
        ],
    },
    StoreSchema {
        command: "safari-bookmarks",
        domain: "AppDomain-com.apple.mobilesafari",
        rel_path: "Library/Safari/Bookmarks.db",
        needs: &[TableNeed { table: "bookmarks", columns: &["id", "title", "url", "parent"] }],
    },
    StoreSchema {
        command: "calendar",
        domain: "HomeDomain",
        rel_path: "Library/Calendar/Calendar.sqlitedb",
        needs: &[
            TableNeed { table: "CalendarItem", columns: &["summary", "start_date", "end_date", "calendar_id"] },
            TableNeed { table: "Calendar", columns: &["ROWID", "title"] },
        ],
    },
    StoreSchema {
        command: "notes",
        domain: "AppDomainGroup-group.com.apple.notes",
        rel_path: "NoteStore.sqlite",
        needs: &[
            TableNeed { table: "ZICCLOUDSYNCINGOBJECT", columns: &["Z_PK", "ZTITLE1", "ZSNIPPET", "ZNOTEDATA"] },
            TableNeed { table: "ZICNOTEDATA", columns: &["Z_PK", "ZDATA"] },
        ],
    },
    StoreSchema {
        command: "photos",
        domain: "CameraRollDomain",
        rel_path: "Media/PhotoData/Photos.sqlite",
        needs: &[TableNeed { table: "ZASSET", columns: &["Z_PK", "ZFILENAME", "ZDIRECTORY", "ZDATECREATED"] }],
    },
    StoreSchema {
        command: "whatsapp",
        domain: "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        rel_path: "ChatStorage.sqlite",
        needs: &[
            TableNeed {
                table: "ZWAMESSAGE",
                columns: &["Z_PK", "ZCHATSESSION", "ZMESSAGEDATE", "ZTEXT", "ZMEDIAITEM"],
            },
            TableNeed { table: "ZWACHATSESSION", columns: &["Z_PK", "ZPARTNERNAME"] },
        ],
    },
    StoreSchema {
        command: "health",
        domain: "HealthDomain",
        rel_path: "Health/healthdb_secure.sqlite",
        needs: &[
            TableNeed { table: "samples", columns: &["data_id", "data_type", "start_date", "end_date"] },
            // `workouts.start_date` is version-conditional (iOS 16 sources workout
            // dates from joined `samples`/`workout_activities`); only `data_id` is
            // the stable anchor here.
            TableNeed { table: "workouts", columns: &["data_id"] },
        ],
    },
    StoreSchema {
        command: "device-usage",
        domain: "AppDomainGroup-group.com.apple.coreduet",
        rel_path: "Library/Knowledge/knowledgeC.db",
        needs: &[TableNeed {
            table: "ZOBJECT",
            columns: &["Z_PK", "ZSTREAMNAME", "ZVALUESTRING", "ZSTARTDATE", "ZENDDATE"],
        }],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn all_columns_present_is_ok() {
        let need = TableNeed { table: "ZCALLRECORD", columns: &["ZDATE", "ZADDRESS"] };
        let actual = cols(&["Z_PK", "ZDATE", "ZADDRESS", "ZDURATION"]);
        let r = check_table(&need, Some(&actual));
        assert_eq!(r.status, "ok");
        assert!(r.missing_columns.is_empty());
    }

    #[test]
    fn missing_column_is_reported() {
        let need = TableNeed { table: "ZCALLRECORD", columns: &["ZDATE", "ZADDRESS"] };
        let actual = cols(&["Z_PK", "ZDATE"]); // ZADDRESS renamed/dropped
        let r = check_table(&need, Some(&actual));
        assert_eq!(r.status, "missing_columns");
        assert_eq!(r.missing_columns, vec!["ZADDRESS".to_string()]);
    }

    #[test]
    fn absent_table_lists_all_columns_missing() {
        let need = TableNeed { table: "ZCALLRECORD", columns: &["ZDATE", "ZADDRESS"] };
        let r = check_table(&need, None);
        assert_eq!(r.status, "table_absent");
        assert_eq!(r.missing_columns, vec!["ZDATE".to_string(), "ZADDRESS".to_string()]);
    }

    #[test]
    fn store_status_rolls_up() {
        let ok = TableReport { table: "a".into(), status: "ok", missing_columns: vec![] };
        let bad = TableReport { table: "b".into(), status: "missing_columns", missing_columns: vec!["x".into()] };
        assert_eq!(store_status(&[ok.clone()]), "ok");
        assert_eq!(store_status(&[ok, bad]), "drifted");
    }

    #[test]
    fn expectations_are_well_formed() {
        // Every expectation needs at least one table, and every table at least one
        // column — an empty need can never drift and is a maintenance mistake.
        for s in EXPECTATIONS {
            assert!(!s.command.is_empty() && !s.domain.is_empty() && !s.rel_path.is_empty());
            assert!(!s.needs.is_empty(), "{} has no tables", s.command);
            for n in s.needs {
                assert!(!n.columns.is_empty(), "{}.{} has no columns", s.command, n.table);
            }
        }
    }
}
