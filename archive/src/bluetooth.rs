//! Paired and previously-seen **Bluetooth devices** from the system Bluetooth
//! databases. Two complementary sources, both in the shared Bluetooth container:
//!
//! * `com.apple.MobileBluetooth.ledevices.paired.db` / `…ledevices.other.db` —
//!   Bluetooth **LE** devices the phone has paired with (`PairedDevices`) or merely
//!   encountered nearby (`OtherDevices`). Columns of interest: `Name`, `Address`,
//!   `ResolvedAddress` (the identity address behind a rotating random one).
//! * `com.apple.MobileBluetooth.devices.plist` — **classic** Bluetooth devices
//!   (car kits, headsets, keyboards…), a dict keyed by MAC address with a `Name` /
//!   `DefaultName`.
//!
//! Forensic value: a roster of every accessory and nearby device the phone knows,
//! with MAC addresses. The databases also carry `LastSeenTime`/`LastConnectionTime`
//! columns, but on the backups inspected these hold small device-relative counters
//! (not a Unix/Cocoa wall-clock epoch), so they are deliberately **not** surfaced
//! as timestamps — exporting them as dates would be false precision.
//!
//! Every reader is schema-tolerant: an unexpected schema or a malformed plist
//! yields an empty list rather than an error.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;

use plist::Value;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::sqlite_util::table_columns;

/// Shared backup domain for the Bluetooth databases and the classic plist.
pub const DOMAIN: &str = "SysSharedContainerDomain-systemgroup.com.apple.bluetooth";

/// LE-device databases: (relative path, table name, kind label).
pub const LE_DATABASES: &[(&str, &str, &str)] = &[
    ("Library/Database/com.apple.MobileBluetooth.ledevices.paired.db", "PairedDevices", "paired"),
    ("Library/Database/com.apple.MobileBluetooth.ledevices.other.db", "OtherDevices", "other"),
];

/// Classic-Bluetooth known-devices plist (dict keyed by MAC address).
pub const CLASSIC_PLIST: &str = "Library/Preferences/com.apple.MobileBluetooth.devices.plist";

/// One Bluetooth device known to the phone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BluetoothDevice {
    /// Advertised / assigned device name; empty when the device has no name.
    pub name: String,
    /// Bluetooth address (MAC for classic, the advertised address for LE — which
    /// may be a rotating random address).
    pub address: String,
    /// Resolved identity address behind a rotating LE address, when known and
    /// different from `address`; empty otherwise.
    pub resolved_address: String,
    /// How the phone knows this device: `paired`, `other` (seen nearby), or
    /// `classic` (classic-Bluetooth known device).
    pub kind: &'static str,
}

impl BluetoothDevice {
    /// Whether the device carries a non-empty name (used to surface named devices
    /// ahead of the many anonymous nearby-LE entries).
    pub fn is_named(&self) -> bool {
        !self.name.is_empty()
    }
}

/// Read one LE-device table (`PairedDevices` / `OtherDevices`) into devices of the
/// given `kind`. Schema-tolerant: the table must exist and have an `Address`
/// column, else an empty list is returned. `Name` / `ResolvedAddress` are used
/// when present.
pub fn parse_ledevices(db_path: &Path, table: &str, kind: &'static str) -> rusqlite::Result<Vec<BluetoothDevice>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = match table_columns(&conn, table) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()), // table absent → nothing to read
    };
    if !cols.contains("Address") {
        return Ok(Vec::new());
    }
    let has_name = cols.contains("Name");
    let has_resolved = cols.contains("ResolvedAddress");
    let sql = format!(
        "SELECT Address, {}, {} FROM \"{table}\"",
        if has_name { "Name" } else { "NULL" },
        if has_resolved { "ResolvedAddress" } else { "NULL" },
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let address: Option<String> = row.get(0)?;
        let name: Option<String> = row.get(1)?;
        let resolved: Option<String> = row.get(2)?;
        Ok((address.unwrap_or_default(), name.unwrap_or_default(), resolved.unwrap_or_default()))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (address, name, resolved) = r?;
        if address.is_empty() && name.is_empty() {
            continue; // a row with neither an address nor a name carries nothing
        }
        // Only surface a resolved address when it adds information.
        let resolved_address = if resolved.is_empty() || resolved == address { String::new() } else { resolved };
        out.push(BluetoothDevice { name, address, resolved_address, kind });
    }
    Ok(out)
}

/// Parse the classic-Bluetooth known-devices plist: a top-level dict keyed by MAC
/// address, each value a dict that may hold `Name` / `DefaultName`. Returns an
/// empty list on any parse failure.
pub fn parse_classic(bytes: &[u8]) -> Vec<BluetoothDevice> {
    let Ok(Value::Dictionary(top)) = Value::from_reader(Cursor::new(bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (mac, entry) in top.iter() {
        let Some(d) = entry.as_dictionary() else { continue };
        let name = d
            .get("Name")
            .and_then(Value::as_string)
            .or_else(|| d.get("DefaultName").and_then(Value::as_string))
            .unwrap_or_default()
            .to_string();
        out.push(BluetoothDevice {
            name,
            address: mac.to_string(),
            resolved_address: String::new(),
            kind: "classic",
        });
    }
    out
}

/// Merge devices from every source into one roster, de-duplicated by
/// (address, kind) and ordered for the human views: paired first, then classic,
/// then named "other" devices, then the anonymous nearby entries — each group by
/// name then address. Paired/classic always win over a duplicate "other" row.
pub fn merge(mut devices: Vec<BluetoothDevice>) -> Vec<BluetoothDevice> {
    fn rank(kind: &str) -> u8 {
        match kind {
            "paired" => 0,
            "classic" => 1,
            _ => 2,
        }
    }
    // Strongest-known kind wins per address; drop weaker duplicates.
    devices.sort_by(|a, b| {
        rank(a.kind).cmp(&rank(b.kind)).then_with(|| a.address.cmp(&b.address))
    });
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped: Vec<BluetoothDevice> = Vec::new();
    for d in devices {
        // An empty address can legitimately repeat (nameless beacons); only
        // de-duplicate non-empty addresses.
        if !d.address.is_empty() && !seen.insert(d.address.clone()) {
            continue;
        }
        deduped.push(d);
    }
    // Final display order: paired, classic, named-other, anonymous-other.
    deduped.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then_with(|| b.is_named().cmp(&a.is_named()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.address.cmp(&b.address))
    });
    deduped
}

/// Build a customer-facing summary of the known Bluetooth devices.
pub fn summary(items: &[BluetoothDevice]) -> crate::summary::Summary {
    use crate::summary::{tally, Summary};

    let named = items.iter().filter(|d| d.is_named()).count();
    let paired = items.iter().filter(|d| d.kind == "paired").count();
    let classic = items.iter().filter(|d| d.kind == "classic").count();
    let other = items.iter().filter(|d| d.kind == "other").count();
    let resolved = items.iter().filter(|d| !d.resolved_address.is_empty()).count();

    // Named-vs-anonymous split, summed (not occurrence-counted) and built by hand
    // so empty buckets are simply not pushed and the breakdown degrades cleanly.
    let mut named_vs: Vec<(String, usize)> = Vec::new();
    if named > 0 {
        named_vs.push(("Pojmenovaná".to_string(), named));
    }
    if items.len() - named > 0 {
        named_vs.push(("Anonymní".to_string(), items.len() - named));
    }

    Summary::new("bluetooth-devices", "Bluetooth zařízení", "zařízení", items.len())
        .count("Pojmenovaných", named)
        .count("Spárovaných", paired)
        .count("Klasických", classic)
        .count("Ostatních/v okolí", other)
        .count("S rozpoznanou identitou", resolved)
        .breakdown("Podle typu", tally(items.iter().map(|d| d.kind.to_string())))
        .breakdown("Pojmenovaná vs anonymní", named_vs)
        .note("Zdroj nemá použitelné časové razítko. „Ostatní\" jsou často anonymní zařízení v okolí.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Dictionary;

    fn make_le_db(path: &Path, table: &str) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE \"{table}\" (Uuid TEXT, Name TEXT, NameOrigin INTEGER,
                Address TEXT, ResolvedAddress TEXT, LastSeenTime INTEGER);
             INSERT INTO \"{table}\" VALUES
                ('u1', 'AirPods Pro', 1, '11:22:33:44:55:66', '', 9000),
                ('u2', NULL, 0, 'aa:bb:cc:dd:ee:ff', '00:11:22:33:44:55', 9001),
                ('u3', '', 0, '', NULL, 9002);"
        ))
        .unwrap();
    }

    #[test]
    fn reads_named_and_resolved_le_devices() {
        let dir = std::env::temp_dir().join(format!("be-bt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("paired.db");
        let _ = std::fs::remove_file(&db);
        make_le_db(&db, "PairedDevices");

        let devs = parse_ledevices(&db, "PairedDevices", "paired").unwrap();
        // The third row (no address, no name) is dropped.
        assert_eq!(devs.len(), 2);
        assert_eq!(devs[0].name, "AirPods Pro");
        assert_eq!(devs[0].address, "11:22:33:44:55:66");
        assert_eq!(devs[0].resolved_address, "");
        assert_eq!(devs[0].kind, "paired");
        // Second device: nameless, but a resolved identity address is surfaced.
        assert_eq!(devs[1].name, "");
        assert_eq!(devs[1].resolved_address, "00:11:22:33:44:55");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ledevices_absent_table_is_empty_not_error() {
        let dir = std::env::temp_dir().join(format!("be-bt-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("other.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE Unrelated (x INTEGER);").unwrap();
        drop(conn);
        assert!(parse_ledevices(&db, "OtherDevices", "other").unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_classic_plist_keyed_by_mac() {
        let mut car = Dictionary::new();
        car.insert("Name".into(), Value::String("Honda CarPlay".into()));
        let mut head = Dictionary::new();
        head.insert("DefaultName".into(), Value::String("WH-1000XM4".into()));
        let mut bare = Dictionary::new();
        bare.insert("initiateSDPMirroringState".into(), Value::Boolean(false));

        let mut top = Dictionary::new();
        top.insert("64:17:CD:F6:54:D1".into(), Value::Dictionary(car));
        top.insert("EC:CE:D7:EE:91:1C".into(), Value::Dictionary(head));
        top.insert("44:8C:00:A2:4A:AA".into(), Value::Dictionary(bare));

        let mut buf = Vec::new();
        Value::Dictionary(top).to_writer_xml(&mut buf).unwrap();
        let mut devs = parse_classic(&buf);
        devs.sort_by(|a, b| a.address.cmp(&b.address));
        assert_eq!(devs.len(), 3);
        // Falls back to DefaultName when Name is absent; a bare entry has no name.
        let by_addr: std::collections::HashMap<_, _> = devs.iter().map(|d| (d.address.as_str(), d.name.as_str())).collect();
        assert_eq!(by_addr["64:17:CD:F6:54:D1"], "Honda CarPlay");
        assert_eq!(by_addr["EC:CE:D7:EE:91:1C"], "WH-1000XM4");
        assert_eq!(by_addr["44:8C:00:A2:4A:AA"], "");
        assert!(devs.iter().all(|d| d.kind == "classic"));
    }

    #[test]
    fn classic_malformed_never_panics() {
        assert!(parse_classic(b"").is_empty());
        assert!(parse_classic(b"not a plist").is_empty());
    }

    #[test]
    fn merge_dedupes_and_orders() {
        let devices = vec![
            BluetoothDevice { name: String::new(), address: "AA".into(), resolved_address: String::new(), kind: "other" },
            BluetoothDevice { name: "Watch".into(), address: "AA".into(), resolved_address: String::new(), kind: "paired" },
            BluetoothDevice { name: "Zeppelin".into(), address: "BB".into(), resolved_address: String::new(), kind: "other" },
            BluetoothDevice { name: String::new(), address: "CC".into(), resolved_address: String::new(), kind: "other" },
            BluetoothDevice { name: "Car".into(), address: "DD".into(), resolved_address: String::new(), kind: "classic" },
        ];
        let merged = merge(devices);
        // AA collapses to the paired row (strongest kind wins).
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].kind, "paired"); // AA/Watch
        assert_eq!(merged[0].address, "AA");
        assert_eq!(merged[1].kind, "classic"); // DD/Car
        // Named "other" (Zeppelin) comes before the anonymous "other" (CC).
        assert_eq!(merged[2].name, "Zeppelin");
        assert_eq!(merged[3].address, "CC");
        assert_eq!(merged[3].name, "");
    }

    fn dev(name: &str, kind: &'static str, address: &str, resolved: &str) -> BluetoothDevice {
        BluetoothDevice { name: name.into(), address: address.into(), resolved_address: resolved.into(), kind }
    }

    #[test]
    fn summary_counts_and_breakdowns() {
        let devices = vec![
            dev("AirPods Pro", "paired", "11:22:33:44:55:66", ""),
            dev("Honda CarPlay", "classic", "64:17:CD:F6:54:D1", ""),
            dev("", "other", "aa:bb:cc:dd:ee:ff", "00:11:22:33:44:55"),
        ];
        let s = summary(&devices);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "zařízení");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Pojmenovaných"), Some(2));
        assert_eq!(get("Spárovaných"), Some(1));
        assert_eq!(get("Klasických"), Some(1));
        assert_eq!(get("Ostatních/v okolí"), Some(1));
        assert_eq!(get("S rozpoznanou identitou"), Some(1));
        let by_kind = s.breakdowns.iter().find(|b| b.title == "Podle typu").unwrap();
        assert!(by_kind.rows.contains(&("paired".to_string(), 1)));
        let named_vs = s.breakdowns.iter().find(|b| b.title == "Pojmenovaná vs anonymní").unwrap();
        assert_eq!(named_vs.rows, vec![("Pojmenovaná".to_string(), 2), ("Anonymní".to_string(), 1)]);
        assert!(s.period.is_none()); // no usable timestamp → never temporal
    }
}
