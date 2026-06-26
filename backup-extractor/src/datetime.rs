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
pub fn cocoa_to_iso(seconds: f64) -> Option<String> {
    unix_to_iso(seconds.trunc() as i64 + COCOA_EPOCH_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
