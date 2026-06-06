/*!
 Structured entities detected within message body text by Apple's
 `DataDetectorsCore` framework.

 Unlike the sender-applied formatting in the parent module ([`animation`] and
 [`style`]), each of these types is recognized automatically from the message
 text and parses itself from a `DDScannerResult` payload via
 [`FromScannerResult`](crate::util::data_detected::FromScannerResult).

 [`animation`]: super::animation
 [`style`]: super::style
*/

/// Detected postal addresses.
pub mod address;
/// Detected currency amounts.
pub mod currency;
/// Detected flight references.
pub mod flight;
/// Detected package-tracking numbers.
pub mod shipment_tracking;
/// Detected unit conversions.
pub mod unit;
