//! Self-check that a backup's live SQLite stores still carry the columns each
//! extractor depends on. iOS point releases occasionally rename, move or drop a
//! column; when that happens the affected extractor silently returns *empty*
//! rather than erroring, which is indistinguishable from "the user had no data".
//! This command surfaces the drift up front: for every store the tool reads, it
//! compares the live schema against the columns that store's extractor needs and
//! reports any that are missing — so an empty export can be explained as either a
//! genuine absence or a schema change.
//!
//! Columns are split into two tiers, mirroring how the extractors actually build
//! their SQL. **Required** columns appear unconditionally in the query (or guard
//! an early return), so their absence breaks extraction and counts as drift.
//! **Optional** columns are gated behind a `table_columns(...)` check with a
//! `NULL`/`COALESCE`/skip fallback — the export still succeeds without them, so a
//! missing optional column is reported for visibility but never flags drift. This
//! keeps the check honest: it fires only when a store would genuinely break.

use std::collections::HashSet;

use serde::Serialize;

/// One table an extractor depends on, split into hard-required and tolerated
/// columns. A table whose `required` is empty is *informational-only*: the
/// extractor tolerates it being absent entirely (e.g. Health), so it never drifts
/// — its optional columns are still reported when missing.
pub struct TableNeed {
    pub table: &'static str,
    /// Columns whose absence breaks extraction: they appear unconditionally in the
    /// SQL (so `prepare()` fails without them — note a LEFT JOIN still requires its
    /// table and referenced columns to *exist*), or guard an early return. SQLite's
    /// implicit `ROWID` is excluded — it is not a declared column and never appears
    /// in `PRAGMA table_info`, yet joins/SELECTs can still reference it.
    pub required: &'static [&'static str],
    /// Columns the extractor tolerates absent (gated with a `table_columns(...)`
    /// check and a NULL/COALESCE/skip fallback). Reported when missing, but never
    /// cause drift.
    pub optional: &'static [&'static str],
}

/// A store the tool extracts from: which command consumes it, where the database
/// lives (one or more candidate `(domain, rel_path)` locations, tried in order —
/// multi-entry for stores whose DB moved across iOS versions), and the
/// tables/columns that command needs.
pub struct StoreSchema {
    pub command: &'static str,
    pub locations: &'static [(&'static str, &'static str)],
    pub needs: &'static [TableNeed],
}

/// Per-table drift result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableReport {
    pub table: String,
    /// `ok` | `missing_columns` (a required column is gone — drift) | `table_absent`
    /// (a required table is gone — drift) | `table_absent_optional` (an
    /// informational-only table the extractor tolerates is gone — not drift).
    pub status: &'static str,
    /// Required columns not found in the live schema (drift-causing).
    pub missing_required: Vec<String>,
    /// Optional columns not found — informational only, never drift.
    pub missing_optional: Vec<String>,
}

/// Per-store schema-check result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoreReport {
    pub command: String,
    /// The matched location's domain (or the first candidate's, when absent).
    pub domain: String,
    /// The matched location's relative path (or the first candidate's, when absent).
    pub rel_path: String,
    /// `ok` (all required columns present) | `drifted` (a required column/table is
    /// missing) | `db_absent` (no candidate location is in this backup).
    pub status: &'static str,
    pub tables: Vec<TableReport>,
}

/// Compare one table's columns against its actual column set. `actual` is `None`
/// when the table does not exist (a `PRAGMA table_info` that returns no rows): a
/// table with required columns is then `table_absent` (drift); an
/// informational-only table (no required columns) is `table_absent_optional`
/// (tolerated — not drift).
pub fn check_table(need: &TableNeed, actual: Option<&HashSet<String>>) -> TableReport {
    let names = |cols: &[&'static str]| cols.iter().map(|c| (*c).to_string()).collect::<Vec<_>>();
    match actual {
        None => TableReport {
            table: need.table.into(),
            status: if need.required.is_empty() { "table_absent_optional" } else { "table_absent" },
            missing_required: names(need.required),
            missing_optional: names(need.optional),
        },
        Some(cols) => {
            let missing_required: Vec<String> =
                need.required.iter().filter(|c| !cols.contains(**c)).map(|c| (*c).to_string()).collect();
            let missing_optional: Vec<String> =
                need.optional.iter().filter(|c| !cols.contains(**c)).map(|c| (*c).to_string()).collect();
            let status = if missing_required.is_empty() { "ok" } else { "missing_columns" };
            TableReport { table: need.table.into(), status, missing_required, missing_optional }
        }
    }
}

/// Roll per-table reports up into a store status: `drifted` if any table lost a
/// required column (`missing_columns`) or a required table (`table_absent`);
/// otherwise `ok` (a tolerated `table_absent_optional` does not drift the store).
pub fn store_status(tables: &[TableReport]) -> &'static str {
    if tables.iter().any(|t| t.status == "missing_columns" || t.status == "table_absent") {
        "drifted"
    } else {
        "ok"
    }
}

/// Every SQLite store the tool extracts from, with its load-bearing schema. Kept
/// in sync by hand with the extractor modules' `SELECT`s and the canonical
/// domain/paths in `KNOWN_STORES`; the required/optional split mirrors each
/// module's `table_columns(...)` gating. `required` lists only columns that
/// `PRAGMA table_info` actually returns — SQLite's implicit `ROWID` (used by some
/// joins) is never listed, so tables whose only hard dependency is an implicit
/// rowid are omitted rather than forced to false-drift.
pub const EXPECTATIONS: &[StoreSchema] = &[
    StoreSchema {
        command: "contacts",
        locations: &[("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb")],
        // contacts::parse uses fixed SQL with no table_columns gating, so every
        // selected column and joined table must exist or prepare() fails — all are
        // required. (Implicit ROWID, used only in joins, is excluded.)
        needs: &[
            TableNeed { table: "ABPerson", required: &["First", "Last", "Organization", "Note"], optional: &[] },
            TableNeed {
                table: "ABMultiValue",
                required: &["UID", "property", "value", "label", "record_id"],
                optional: &[],
            },
            TableNeed { table: "ABMultiValueLabel", required: &["value"], optional: &[] },
            TableNeed { table: "ABMultiValueEntry", required: &["key", "value", "parent_id"], optional: &[] },
            TableNeed { table: "ABMultiValueEntryKey", required: &["value"], optional: &[] },
        ],
    },
    StoreSchema {
        command: "calls",
        locations: &[("HomeDomain", "Library/CallHistoryDB/CallHistory.storedata")],
        needs: &[TableNeed {
            table: "ZCALLRECORD",
            required: &["ZADDRESS", "ZDATE", "ZDURATION", "ZORIGINATED", "ZANSWERED", "ZCALLTYPE"],
            optional: &["ZSERVICE_PROVIDER", "ZLOCATION", "ZISO_COUNTRY_CODE"],
        }],
    },
    StoreSchema {
        command: "accounts",
        locations: &[("HomeDomain", "Library/Accounts/Accounts3.sqlite")],
        needs: &[
            TableNeed {
                table: "ZACCOUNT",
                required: &["ZACCOUNTTYPE"], // the LEFT JOIN key; all value columns are select-or-NULL
                optional: &["ZUSERNAME", "ZACCOUNTDESCRIPTION", "ZOWNINGBUNDLEID", "ZDATE", "ZACTIVE"],
            },
            TableNeed { table: "ZACCOUNTTYPE", required: &["Z_PK"], optional: &["ZACCOUNTTYPEDESCRIPTION", "ZIDENTIFIER"] },
        ],
    },
    StoreSchema {
        command: "data-usage",
        locations: &[("WirelessDomain", "Library/Databases/DataUsage.sqlite")],
        needs: &[
            TableNeed {
                table: "ZLIVEUSAGE",
                required: &["ZHASPROCESS"],
                optional: &["ZWWANIN", "ZWWANOUT", "ZWIFIIN", "ZWIFIOUT", "ZTIMESTAMP"],
            },
            TableNeed { table: "ZPROCESS", required: &["Z_PK"], optional: &["ZPROCNAME", "ZBUNDLENAME"] },
        ],
    },
    StoreSchema {
        command: "voicemail",
        locations: &[("HomeDomain", "Library/Voicemail/voicemail.db")],
        needs: &[TableNeed {
            table: "voicemail",
            required: &["sender", "date", "duration", "trashed_date", "flags"],
            optional: &["expiration"],
        }],
    },
    StoreSchema {
        command: "voice-memos",
        // Modern store only; the legacy MediaDomain/Recordings.db (iOS ≤ 11) has a
        // different schema and is out of scope for these backups.
        locations: &[("AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db")],
        needs: &[TableNeed {
            table: "ZCLOUDRECORDING",
            required: &["ZDATE", "ZDURATION"],
            optional: &["ZCUSTOMLABEL", "ZENCRYPTEDTITLE", "ZPATH"],
        }],
    },
    StoreSchema {
        command: "safari-history",
        locations: &[("AppDomain-com.apple.mobilesafari", "Library/Safari/History.db")],
        needs: &[
            TableNeed { table: "history_visits", required: &["history_item", "visit_time"], optional: &["title"] },
            TableNeed { table: "history_items", required: &["id", "url"], optional: &["visit_count"] },
        ],
    },
    StoreSchema {
        command: "safari-bookmarks",
        locations: &[("AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db")],
        needs: &[TableNeed { table: "bookmarks", required: &["title", "url", "parent"], optional: &[] }],
    },
    StoreSchema {
        command: "calendar",
        locations: &[("HomeDomain", "Library/Calendar/Calendar.sqlitedb")],
        needs: &[
            TableNeed {
                table: "CalendarItem",
                required: &["summary", "start_date", "calendar_id"],
                optional: &["end_date", "all_day"],
            },
            // c.title is selected unconditionally; the join uses implicit ROWID.
            TableNeed { table: "Calendar", required: &["title"], optional: &[] },
        ],
    },
    StoreSchema {
        command: "notes",
        locations: &[("AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite")],
        needs: &[
            TableNeed {
                table: "ZICCLOUDSYNCINGOBJECT",
                required: &["ZNOTEDATA"], // early-return guard
                optional: &["ZTITLE1", "ZSNIPPET", "ZCREATIONDATE", "ZMODIFICATIONDATE1", "ZFOLDER", "ZTITLE2"],
            },
            // The body table + d.ZDATA are joined/selected unconditionally.
            TableNeed { table: "ZICNOTEDATA", required: &["ZDATA"], optional: &[] },
        ],
    },
    StoreSchema {
        command: "photos",
        locations: &[("CameraRollDomain", "Media/PhotoData/Photos.sqlite")],
        needs: &[TableNeed {
            table: "ZASSET",
            required: &["Z_PK", "ZFILENAME", "ZDIRECTORY"], // ZKIND is select-or-NULL
            optional: &["ZKIND", "ZDATECREATED", "ZADDEDDATE", "ZFAVORITE", "ZHIDDEN", "ZTRASHEDSTATE", "ZLATITUDE", "ZLONGITUDE"],
        }],
    },
    StoreSchema {
        command: "whatsapp",
        locations: &[("AppDomainGroup-group.net.whatsapp.WhatsApp.shared", "ChatStorage.sqlite")],
        // The two LEFT JOINs and their selected columns are unconditional, so those
        // tables/columns and the join keys must exist; only ZTEXT/ZISFROMME/ZFROMJID
        // are select-or-NULL.
        needs: &[
            TableNeed {
                table: "ZWAMESSAGE",
                required: &["ZMESSAGEDATE", "ZCHATSESSION", "ZMEDIAITEM"],
                optional: &["ZTEXT", "ZISFROMME", "ZFROMJID"],
            },
            TableNeed { table: "ZWACHATSESSION", required: &["ZPARTNERNAME"], optional: &[] },
            TableNeed { table: "ZWAMEDIAITEM", required: &["ZMEDIALOCALPATH"], optional: &[] },
        ],
    },
    StoreSchema {
        command: "health",
        locations: &[("HealthDomain", "Health/healthdb_secure.sqlite")],
        // health::parse is fully schema-tolerant: a missing workouts/samples/
        // quantity_samples table yields empty data, never an error. So none of its
        // tables are required — they are informational-only (their optional columns
        // are still reported when missing, but the store never drifts).
        needs: &[
            TableNeed { table: "workouts", required: &[], optional: &["data_id", "start_date", "end_date", "activity_type", "duration", "total_distance"] },
            TableNeed { table: "samples", required: &[], optional: &["data_id", "data_type", "start_date", "end_date"] },
            TableNeed { table: "quantity_samples", required: &[], optional: &["data_id", "quantity"] },
        ],
    },
    StoreSchema {
        command: "device-usage",
        // knowledgeC.db has lived at several domain/paths; all share the ZOBJECT
        // schema, so every candidate is tried and the first present one is checked.
        locations: &[
            ("AppDomainGroup-group.com.apple.coreduet", "Library/Knowledge/knowledgeC.db"),
            ("AppDomainGroup-group.com.apple.coreduet", "Library/CoreDuet/Knowledge/knowledgeC.db"),
            ("AppDomainGroup-group.com.apple.coreduetd", "Library/Knowledge/knowledgeC.db"),
            ("AppDomainGroup-group.com.apple.coreduetd", "Library/CoreDuet/Knowledge/knowledgeC.db"),
            ("HomeDomain", "Library/CoreDuet/Knowledge/knowledgeC.db"),
            ("HomeDomain", "Library/CoreDuet/knowledgeC.db"),
        ],
        needs: &[TableNeed {
            table: "ZOBJECT",
            required: &["ZSTREAMNAME", "ZVALUESTRING", "ZSTARTDATE", "ZENDDATE"],
            optional: &[],
        }],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    fn need(required: &'static [&'static str], optional: &'static [&'static str]) -> TableNeed {
        TableNeed { table: "ZCALLRECORD", required, optional }
    }

    #[test]
    fn all_required_present_is_ok() {
        let actual = cols(&["Z_PK", "ZDATE", "ZADDRESS", "ZDURATION"]);
        let r = check_table(&need(&["Z_PK", "ZDATE"], &["ZADDRESS"]), Some(&actual));
        assert_eq!(r.status, "ok");
        assert!(r.missing_required.is_empty() && r.missing_optional.is_empty());
    }

    #[test]
    fn missing_required_column_is_drift() {
        let actual = cols(&["Z_PK"]); // ZDATE renamed/dropped
        let r = check_table(&need(&["Z_PK", "ZDATE"], &[]), Some(&actual));
        assert_eq!(r.status, "missing_columns");
        assert_eq!(r.missing_required, vec!["ZDATE".to_string()]);
    }

    #[test]
    fn missing_optional_column_is_not_drift() {
        let actual = cols(&["Z_PK", "ZDATE"]); // optional ZADDRESS absent
        let r = check_table(&need(&["Z_PK", "ZDATE"], &["ZADDRESS"]), Some(&actual));
        assert_eq!(r.status, "ok"); // tolerated → still ok
        assert_eq!(r.missing_optional, vec!["ZADDRESS".to_string()]);
        assert!(r.missing_required.is_empty());
    }

    #[test]
    fn absent_table_is_drift_listing_all_columns() {
        let r = check_table(&need(&["Z_PK", "ZDATE"], &["ZADDRESS"]), None);
        assert_eq!(r.status, "table_absent");
        assert_eq!(r.missing_required, vec!["Z_PK".to_string(), "ZDATE".to_string()]);
        assert_eq!(r.missing_optional, vec!["ZADDRESS".to_string()]);
    }

    #[test]
    fn informational_only_table_absent_is_not_drift() {
        // A table with no required columns is tolerated absent (Health-style).
        let info = TableNeed { table: "workouts", required: &[], optional: &["data_id", "start_date"] };
        let r = check_table(&info, None);
        assert_eq!(r.status, "table_absent_optional");
        assert!(r.missing_required.is_empty());
        assert_eq!(r.missing_optional, vec!["data_id".to_string(), "start_date".to_string()]);
        assert_eq!(store_status(&[r]), "ok"); // does not drift the store
    }

    #[test]
    fn store_status_rolls_up() {
        let ok = TableReport { table: "a".into(), status: "ok", missing_required: vec![], missing_optional: vec![] };
        let bad = TableReport {
            table: "b".into(),
            status: "missing_columns",
            missing_required: vec!["x".into()],
            missing_optional: vec![],
        };
        // An optional-only miss does not drift the store.
        let opt = TableReport {
            table: "c".into(),
            status: "ok",
            missing_required: vec![],
            missing_optional: vec!["y".into()],
        };
        assert_eq!(store_status(&[ok.clone(), opt]), "ok");
        assert_eq!(store_status(&[ok, bad]), "drifted");
    }

    #[test]
    fn expectations_are_well_formed() {
        // Every expectation needs a location and at least one table; every table at
        // least one column total (required for drift-bearing tables, or optional for
        // informational-only ones — an entirely empty table is a modelling mistake).
        for s in EXPECTATIONS {
            assert!(!s.command.is_empty(), "empty command");
            assert!(!s.locations.is_empty(), "{} has no locations", s.command);
            for (d, p) in s.locations {
                assert!(!d.is_empty() && !p.is_empty(), "{} has an empty location", s.command);
            }
            assert!(!s.needs.is_empty(), "{} has no tables", s.command);
            for n in s.needs {
                assert!(
                    !n.required.is_empty() || !n.optional.is_empty(),
                    "{}.{} has no columns at all",
                    s.command,
                    n.table
                );
            }
        }
    }
}
