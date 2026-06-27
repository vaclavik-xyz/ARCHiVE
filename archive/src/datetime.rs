//! Convert iOS timestamp epochs to ISO 8601 (RFC 3339) UTC strings.

use chrono::DateTime;

/// Seconds between the Cocoa / Core Data reference date (2001-01-01 UTC) and the
/// Unix epoch (1970-01-01 UTC).
const COCOA_EPOCH_OFFSET: i64 = 978_307_200;

/// Seconds since the Unix epoch → ISO 8601 (RFC 3339) UTC, or `None` when out of
/// the representable range.
pub fn unix_to_iso(seconds: i64) -> Option<String> {
    DateTime::from_timestamp(seconds, 0).map(|dt| dt.to_rfc3339())
}

/// Seconds since the Cocoa / Core Data reference date (2001-01-01 UTC) → ISO 8601
/// (RFC 3339) UTC, or `None` when out of range. Fractional seconds are truncated.
/// Non-finite (`NaN`/`inf`) inputs and values that overflow on conversion to the
/// Unix epoch both yield `None` rather than a bogus or panicking result.
pub fn cocoa_to_iso(seconds: f64) -> Option<String> {
    if !seconds.is_finite() {
        return None;
    }
    let unix = (seconds.trunc() as i64).checked_add(COCOA_EPOCH_OFFSET)?;
    unix_to_iso(unix)
}

/// Cocoa/Core-Data epoch value that may be stored in **seconds** or
/// **nanoseconds** (iOS-version dependent, e.g. `sms.db` dates) → ISO 8601 UTC.
/// Values with magnitude ≥ 1e12 are treated as nanoseconds; smaller as seconds.
pub fn cocoa_any_to_iso(value: i64) -> Option<String> {
    const NS_THRESHOLD: i64 = 1_000_000_000_000; // 1e12
    let seconds = if value.abs() >= NS_THRESHOLD {
        value / 1_000_000_000
    } else {
        value
    };
    cocoa_to_iso(seconds as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cocoa_any_detects_seconds_and_nanoseconds() {
        // 600_000_000 s == 600_000_000_000_000_000 ns → same instant.
        let s = cocoa_any_to_iso(600_000_000).unwrap();
        let ns = cocoa_any_to_iso(600_000_000_000_000_000).unwrap();
        assert_eq!(s, "2020-01-06T10:40:00+00:00");
        assert_eq!(s, ns);
        // Just under the threshold stays seconds.
        assert_eq!(cocoa_any_to_iso(0), cocoa_to_iso(0.0));
    }

    #[test]
    fn unix_epoch_zero_is_1970() {
        assert_eq!(unix_to_iso(0).unwrap(), "1970-01-01T00:00:00+00:00");
    }

    #[test]
    fn cocoa_epoch_zero_is_2001() {
        assert_eq!(cocoa_to_iso(0.0).unwrap(), "2001-01-01T00:00:00+00:00");
    }

    #[test]
    fn cocoa_offset_matches_unix() {
        assert_eq!(cocoa_to_iso(0.0), unix_to_iso(COCOA_EPOCH_OFFSET));
    }

    #[test]
    fn cocoa_truncates_fractional_seconds() {
        assert_eq!(cocoa_to_iso(100.9).unwrap(), "2001-01-01T00:01:40+00:00");
    }

    #[test]
    fn cocoa_rejects_non_finite_and_overflow() {
        assert_eq!(cocoa_to_iso(f64::NAN), None);
        assert_eq!(cocoa_to_iso(f64::INFINITY), None);
        assert_eq!(cocoa_to_iso(f64::NEG_INFINITY), None);
        assert_eq!(cocoa_to_iso(f64::MAX), None);
    }
}
