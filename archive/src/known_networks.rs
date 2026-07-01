//! Read the device's remembered Wi-Fi networks (the SSID *identity* layer) from
//! the system Wi-Fi preferences plist.
//!
//! This is complementary to the `wifi` command: `wifi` recovers Wi-Fi
//! **passwords** from the keychain and needs an **encrypted** backup, whereas
//! this reads the plaintext **list of saved networks** (SSID, BSSID, last-joined,
//! hidden/auto-join) from a normal plist, so it works on **any** backup. No
//! passwords are involved.
//!
//! The plist moved and changed shape across iOS versions, so two layouts are
//! handled: the older `com.apple.wifi.plist` (`List of known networks` array) and
//! the newer `com.apple.wifi-networks.plist[.backup]` (`KnownNetworks` dict /
//! `List of known networks` array). Both are read leniently — an unrecognized or
//! malformed entry is skipped, never panics.

use std::io::Cursor;

use plist::Value;
use serde::Serialize;

/// Backup domain holding the Wi-Fi preferences plist.
pub const DOMAIN: &str = "SystemPreferencesDomain";

/// Candidate relative paths for the saved-networks plist, newest-first by what we
/// prefer to read; probed in order until one exists (and yields networks).
pub const PATHS: &[&str] = &[
    "SystemConfiguration/com.apple.wifi-networks.plist.backup",
    "SystemConfiguration/com.apple.wifi-networks.plist",
    "SystemConfiguration/com.apple.wifi.plist",
];

/// One remembered Wi-Fi network (no password).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnownNetwork {
    /// Network name (SSID).
    pub ssid: String,
    /// Last-seen access-point MAC (BSSID), when recorded.
    pub bssid: String,
    /// Best-effort last-joined time, ISO 8601 UTC; empty if unknown.
    pub last_joined: String,
    /// Whether the network is hidden (non-broadcast); `None` when not recorded.
    pub hidden: Option<bool>,
    /// Best-effort security mode (e.g. "WPA2 Personal"); empty if unknown.
    pub security: String,
}

/// Parse the saved-networks list from a Wi-Fi preferences plist's bytes. Total:
/// returns an empty list on any parse failure rather than erroring.
pub fn parse(bytes: &[u8]) -> Vec<KnownNetwork> {
    match Value::from_reader(Cursor::new(bytes)) {
        Ok(v) => from_value(&v),
        Err(_) => Vec::new(),
    }
}

/// Extract networks from a parsed plist value. The saved networks live under one
/// of two container keys, and that container is an **array** (older iOS) or a
/// **dict keyed by network id** (iOS 16+) — both are handled.
///
/// Note: on iOS 16+ this plaintext list is typically **empty** — the saved-network
/// inventory moved out of `com.apple.wifi*.plist` (the SSIDs survive only in the
/// keychain, which the `wifi` command reads). This extractor is therefore most
/// useful on older-iOS backups; it returns an empty list, not an error, otherwise.
fn from_value(v: &Value) -> Vec<KnownNetwork> {
    let Some(dict) = v.as_dictionary() else {
        return Vec::new();
    };
    for key in ["List of known networks", "KnownNetworks"] {
        let Some(container) = dict.get(key) else { continue };
        let entries: Vec<&Value> = if let Some(arr) = container.as_array() {
            arr.iter().collect()
        } else if let Some(d) = container.as_dictionary() {
            d.values().collect()
        } else {
            continue;
        };
        let nets: Vec<KnownNetwork> = entries.into_iter().filter_map(network_from_entry).collect();
        if !nets.is_empty() {
            return nets;
        }
    }
    Vec::new()
}

/// Build a `KnownNetwork` from one entry dict, tolerating the various key names
/// used across iOS versions. Requires a non-empty SSID; otherwise the entry is
/// skipped.
fn network_from_entry(entry: &Value) -> Option<KnownNetwork> {
    let d = entry.as_dictionary()?;

    // SSID: a plain string under one of several keys, or raw bytes under "SSID".
    let ssid = str_key(d, &["SSID_STR", "SSIDString", "ssid"])
        .or_else(|| {
            d.get("SSID")
                .and_then(Value::as_data)
                .map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_string())
        })
        .filter(|s| !s.is_empty())?;

    let bssid = str_key(d, &["BSSID", "PRESENTED_BSSID"]).unwrap_or_default();
    let last_joined = date_key(
        d,
        &["lastJoined", "JoinedByUserAt", "lastAutoJoined", "AddedAt", "UpdatedAt", "updatedAt"],
    )
    .unwrap_or_default();
    let hidden = bool_key(d, &["HIDDEN_NETWORK", "Hidden", "hidden"]);
    let security = str_key(d, &["SecurityMode", "SupportedSecurityModes", "security"]).unwrap_or_default();

    Some(KnownNetwork { ssid, bssid, last_joined, hidden, security })
}

fn str_key(d: &plist::Dictionary, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(s) = d.get(k).and_then(Value::as_string) {
            return Some(s.to_string());
        }
    }
    None
}

fn bool_key(d: &plist::Dictionary, keys: &[&str]) -> Option<bool> {
    for &k in keys {
        if let Some(b) = d.get(k).and_then(Value::as_boolean) {
            return Some(b);
        }
    }
    None
}

fn date_key(d: &plist::Dictionary, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(date) = d.get(k).and_then(Value::as_date) {
            let st: std::time::SystemTime = date.into();
            let iso = st
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|dur| crate::datetime::unix_to_iso(dur.as_secs() as i64));
            if let Some(iso) = iso {
                return Some(iso);
            }
        }
    }
    None
}

/// Build a customer-facing summary of the recovered saved Wi-Fi networks.
pub fn summary(items: &[KnownNetwork]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};

    let hidden = items.iter().filter(|n| n.hidden == Some(true)).count();
    let with_time = items.iter().filter(|n| !n.last_joined.is_empty()).count();
    let with_security = items.iter().filter(|n| !n.security.is_empty()).count();
    let with_bssid = items.iter().filter(|n| !n.bssid.is_empty()).count();

    let security = |n: &KnownNetwork| -> String {
        if n.security.is_empty() { "Neznámé".to_string() } else { n.security.clone() }
    };

    Summary::new("known-networks", "Uložené Wi-Fi sítě", "sítí", items.len())
        .count("Skrytých", hidden)
        .count("S časem připojení", with_time)
        .count("Se známým zabezpečením", with_security)
        .count("Se záznamem MAC (BSSID)", with_bssid)
        .period_from(iso_range(items.iter().map(|n| n.last_joined.as_str())))
        .breakdown("Podle zabezpečení", tally(items.iter().map(security)))
        .breakdown("Po letech (připojeno)", year_rows(items.iter().map(|n| n.last_joined.as_str())))
        .note("Na iOS 16+ bývá prázdné (seznam se přesunul do klíčenky — viz příkaz wifi). Bez hesel.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::{Date, Dictionary, Value};

    fn net(ssid: &str, bssid: &str, last_joined: &str, hidden: Option<bool>, security: &str) -> KnownNetwork {
        KnownNetwork {
            ssid: ssid.into(),
            bssid: bssid.into(),
            last_joined: last_joined.into(),
            hidden,
            security: security.into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let nets = vec![
            net("HomeNet", "aa:bb:cc:dd:ee:ff", "2020-01-06T10:40:00+00:00", Some(true), "WPA2 Personal"),
            net("CafeWiFi", "11:22:33:44:55:66", "2022-05-01T08:00:00+00:00", Some(false), "WPA2 Personal"),
            net("OpenGuest", "", "", None, ""),
        ];
        let s = summary(&nets);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "sítí");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Skrytých"), Some(1));
        assert_eq!(get("S časem připojení"), Some(2));
        assert_eq!(get("Se známým zabezpečením"), Some(2));
        assert_eq!(get("Se záznamem MAC (BSSID)"), Some(2));
        let sec = s.breakdowns.iter().find(|b| b.title == "Podle zabezpečení").unwrap();
        assert_eq!(sec.rows[0], ("WPA2 Personal".to_string(), 2));
        assert!(s.period.is_some()); // derived from the two dated networks
    }

    fn to_bytes(v: &Value) -> Vec<u8> {
        let mut buf = Vec::new();
        v.to_writer_xml(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parses_old_list_layout() {
        // "List of known networks" → array of dicts with SSID_STR / BSSID / dates.
        let mut net = Dictionary::new();
        net.insert("SSID_STR".into(), Value::String("HomeNet".into()));
        net.insert("BSSID".into(), Value::String("aa:bb:cc:dd:ee:ff".into()));
        net.insert("HIDDEN_NETWORK".into(), Value::Boolean(true));
        // 2020-01-06T10:40:00Z (cocoa 600000000 in Unix terms).
        let st = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_578_307_200);
        net.insert("lastJoined".into(), Value::Date(Date::from(st)));

        let mut top = Dictionary::new();
        top.insert("List of known networks".into(), Value::Array(vec![Value::Dictionary(net)]));

        let nets = parse(&to_bytes(&Value::Dictionary(top)));
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].ssid, "HomeNet");
        assert_eq!(nets[0].bssid, "aa:bb:cc:dd:ee:ff");
        assert_eq!(nets[0].hidden, Some(true));
        assert!(nets[0].last_joined.starts_with("2020-01-06"), "got {}", nets[0].last_joined);
    }

    #[test]
    fn parses_new_knownnetworks_layout_with_data_ssid() {
        // "KnownNetworks" dict keyed by id; SSID stored as raw bytes (Data).
        let mut net = Dictionary::new();
        net.insert("SSID".into(), Value::Data(b"CafeWiFi".to_vec()));
        net.insert("BSSID".into(), Value::String("11:22:33:44:55:66".into()));
        net.insert("Hidden".into(), Value::Boolean(false));

        let mut known = Dictionary::new();
        known.insert("wifi.network.ssid.CafeWiFi".into(), Value::Dictionary(net));
        let mut top = Dictionary::new();
        top.insert("KnownNetworks".into(), Value::Dictionary(known));

        let nets = parse(&to_bytes(&Value::Dictionary(top)));
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].ssid, "CafeWiFi");
        assert_eq!(nets[0].bssid, "11:22:33:44:55:66");
        assert_eq!(nets[0].hidden, Some(false));
    }

    #[test]
    fn entry_without_ssid_is_skipped() {
        let mut net = Dictionary::new();
        net.insert("BSSID".into(), Value::String("00:00:00:00:00:00".into()));
        let mut top = Dictionary::new();
        top.insert("List of known networks".into(), Value::Array(vec![Value::Dictionary(net)]));
        assert!(parse(&to_bytes(&Value::Dictionary(top))).is_empty());
    }

    #[test]
    fn malformed_or_empty_never_panics() {
        assert!(parse(b"").is_empty());
        assert!(parse(b"not a plist").is_empty());
        // A plist that is not a dictionary at the top level.
        assert!(parse(&to_bytes(&Value::Array(vec![]))).is_empty());
    }
}
