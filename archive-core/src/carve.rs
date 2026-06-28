//! A generic, schema-less SQLite "carver" that recovers **deleted** records from
//! the raw bytes of a `.sqlite` database file (and, optionally, its `-wal`
//! sidecar).
//!
//! When SQLite deletes a row it does not zero the bytes; it merely unlinks the
//! cell from the b-tree and returns its space to one of several free regions.
//! The old payload lingers until the page is reused. This module walks those
//! free regions byte-for-byte and tries to re-decode any SQLite record it finds,
//! recovering the column values of rows that the live schema no longer lists.
//!
//! It is deliberately **schema-less**: SQLite records are self-describing (each
//! cell carries a serial-type header that names the type and width of every
//! column), so we can decode values without knowing which table they came from.
//! Per-store attribution (matching a carved record back to a specific table /
//! column layout) is left to the caller — this layer only yields raw
//! [`CarvedValue`] vectors plus a hint of *where* each record was found.
//!
//! # What is parsed
//!
//! All four classic "deleted data" regions of a rollback-journal database, plus
//! the WAL:
//!
//! * **Freeblocks** — gaps *inside* an otherwise live leaf page, chained from the
//!   page header's first-freeblock pointer.
//! * **Unallocated gap** — the slack between a leaf page's cell-pointer array and
//!   its cell-content area, where freshly deleted cells often still sit intact.
//! * **Freelist pages** — whole pages returned to the database's free list; their
//!   former contents usually survive until reallocation.
//! * **WAL frames** — every frame body in the write-ahead log, *including*
//!   superseded frames for the same page number, which hold pre-deletion page
//!   images.
//!
//! # What is **not** parsed
//!
//! * Overflow pages are **not** reassembled. A record whose payload spills onto
//!   an overflow page is recovered only up to its on-page prefix and flagged
//!   [`CarvedRecord::truncated`].
//! * No schema / table attribution (the controller does that).
//! * `rusqlite` is **not** used for carving — this is pure big-endian byte
//!   parsing. (`rusqlite` appears only in the test module, to build fixtures.)
//!
//! # Safety against hostile input
//!
//! Every multi-byte read is bounds-checked via slice `.get(..)`, returning
//! `None`/`Err` instead of panicking. All chain walks (freeblock, freelist trunk)
//! carry a visited-set and a hard cap, and the public entry point applies global
//! caps on pages scanned, candidates per region, record size and total records,
//! so a corrupt or adversarial file cannot exhaust memory or time.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Global caps. These bound the work done on a single (possibly hostile) file.
// They are deliberately generous for real databases yet small enough that a
// crafted file cannot blow up memory or wall-clock time.
// ---------------------------------------------------------------------------

/// Largest database/WAL page we will honour. SQLite's own maximum is 65536.
const MAX_PAGE_SIZE: usize = 65_536;
/// Smallest legal page size per the SQLite spec.
const MIN_PAGE_SIZE: usize = 512;
/// Hard ceiling on pages visited across *all* sources for one file. Far above
/// any realistic phone-era database, yet bounds work on a corrupt header that
/// claims a tiny page size over a huge file.
const MAX_PAGES_SCANNED: usize = 1_000_000;
/// Cap on freeblock-chain / freelist-trunk hops before we give up (defends
/// against cyclic or absurdly long chains that slipped past the visited-set).
const MAX_CHAIN_HOPS: usize = 1_000_000;
/// Largest on-page record payload we will materialize, in bytes. Overflow
/// payloads are truncated to their on-page prefix anyway, so this also caps the
/// per-record allocation.
const MAX_RECORD_SIZE: usize = 1_000_000;
/// Maximum columns in a single decoded record (a sane upper bound; SQLite's own
/// hard limit is 32767).
const MAX_COLUMNS: usize = 100_000;
/// Maximum candidate offsets we will *try* to decode within a single free region
/// (the sliding window stops after this many starting positions).
const MAX_CANDIDATES_PER_REGION: usize = 1_000_000;
/// Maximum total records returned from one [`carve_sqlite`] call.
const MAX_TOTAL_RECORDS: usize = 5_000_000;
/// Largest single TEXT/BLOB column we will accept from a *raw* (unframed) sliding
/// window scan. A lone, enormous text/blob value is the classic signature of a
/// mis-framed candidate (the scanner accidentally swallowed a whole free region
/// as one column), so the raw-scan quality gate rejects it. Cell-framed decodes
/// (real cell-pointer entries) are exempt — there the boundaries are trustworthy.
const MAX_RAW_VALUE_LEN: usize = 4_096;

// ---------------------------------------------------------------------------
// Public output types.
// ---------------------------------------------------------------------------

/// A single decoded column value. Mirrors SQLite's storage classes. `Int`
/// absorbs every integer serial type (1-byte through 8-byte, plus the constant
/// `0`/`1` encodings); `Real` is an IEEE-754 double.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum CarvedValue {
    /// SQL `NULL` (serial type 0).
    Null,
    /// An integer (serial types 1–6, 8, 9), widened to `i64`.
    Int(i64),
    /// An IEEE-754 double (serial type 7).
    Real(f64),
    /// UTF-8 text (odd serial types ≥ 13). Invalid bytes are replaced lossily.
    Text(String),
    /// A binary blob (even serial types ≥ 12).
    Blob(Vec<u8>),
}

/// Which free region a record was carved from. Purely informational; useful to a
/// caller deciding how much to trust a candidate (e.g. unallocated-gap hits tend
/// to be the most intact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CarveSource {
    /// A page returned wholesale to the database free list.
    Freelist,
    /// A freeblock chained inside an otherwise live leaf page.
    Freeblock,
    /// The unallocated gap between a leaf page's cell pointers and cell content.
    Unallocated,
    /// A frame body in the write-ahead log (`-wal`).
    Wal,
}

/// One plausibly-decoded deleted record.
///
/// `rowid` is the cell's rowid varint when the candidate was decoded as a full
/// table-leaf cell (payload-len + rowid + record); it is `None` for records
/// recovered by scanning raw bytes where no cell framing was present.
/// `truncated` is set when the record's declared payload extends beyond the
/// bytes actually available on the page (an overflow spill we do not follow).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CarvedRecord {
    /// The cell rowid, when recovered from a full table-leaf cell framing.
    pub rowid: Option<i64>,
    /// Which free region this record came from.
    pub source: CarveSource,
    /// The decoded column values, in storage order.
    pub values: Vec<CarvedValue>,
    /// `true` when the on-disk payload was longer than the bytes available here
    /// (overflow pages are not reassembled — only the on-page prefix is decoded).
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Varint decoding.
// ---------------------------------------------------------------------------

/// Decode a SQLite big-endian base-128 varint starting at `data[0]`.
///
/// Returns `(value, bytes_consumed)`, or `None` if `data` is empty. Varints are
/// 1–9 bytes: the high bit of each of the first eight bytes is a continuation
/// flag and the low seven bits are payload; a ninth byte (reached only when the
/// first eight all had their continuation bit set) contributes all eight bits.
/// The decoded magnitude is interpreted as a two's-complement `i64`, matching
/// SQLite (varints can encode negative integers).
///
/// This never reads past `data` and never panics: a truncated varint (high bit
/// set on the last available byte) is decoded from the bytes present.
pub(crate) fn read_varint(data: &[u8]) -> Option<(i64, usize)> {
    let mut result: u64 = 0;
    let mut i = 0;
    while i < 8 {
        let byte = *data.get(i)?;
        result = (result << 7) | u64::from(byte & 0x7f);
        i += 1;
        if byte & 0x80 == 0 {
            return Some((result as i64, i));
        }
    }
    // Ninth byte (if present) contributes all 8 bits.
    let byte = *data.get(8)?;
    result = (result << 8) | u64::from(byte);
    Some((result as i64, 9))
}

// ---------------------------------------------------------------------------
// Serial-type decoding.
// ---------------------------------------------------------------------------

/// Byte width of the value for a given serial type, or `None` for types that are
/// illegal in a real record (10 and 11 are reserved for internal use). Constant
/// encodings (0, 8, 9) and `NULL` occupy zero content bytes.
fn serial_type_len(serial: u64) -> Option<usize> {
    Some(match serial {
        0 => 0,            // NULL
        1 => 1,            // i8
        2 => 2,            // i16
        3 => 3,            // i24
        4 => 4,            // i32
        5 => 6,            // i48
        6 => 8,            // i64
        7 => 8,            // f64
        8 | 9 => 0,        // constant 0 / 1
        10 | 11 => return None, // reserved — a record containing these is invalid
        n if n >= 12 => ((n - 12) / 2) as usize, // BLOB or TEXT
        _ => return None,
    })
}

/// Read one column value of serial type `serial` from `bytes` (which must be
/// exactly the content slice for this column). Returns `None` on any
/// inconsistency. Text is decoded UTF-8-lossy so partially-overwritten strings
/// still yield something useful rather than being dropped.
fn decode_value(serial: u64, bytes: &[u8]) -> Option<CarvedValue> {
    let val = match serial {
        0 => CarvedValue::Null,
        1 => CarvedValue::Int(i64::from(read_be_int(bytes, 1)? as i8)),
        2 => CarvedValue::Int(i64::from(read_be_int(bytes, 2)? as i16)),
        3 => CarvedValue::Int(sign_extend(read_be_int(bytes, 3)?, 3)),
        4 => CarvedValue::Int(i64::from(read_be_int(bytes, 4)? as i32)),
        5 => CarvedValue::Int(sign_extend(read_be_int(bytes, 6)?, 6)),
        6 => CarvedValue::Int(read_be_int(bytes, 8)? as i64),
        7 => {
            let raw = read_be_int(bytes, 8)?;
            CarvedValue::Real(f64::from_bits(raw))
        }
        8 => CarvedValue::Int(0),
        9 => CarvedValue::Int(1),
        n if n >= 12 && n % 2 == 0 => {
            let len = ((n - 12) / 2) as usize;
            CarvedValue::Blob(bytes.get(..len)?.to_vec())
        }
        n if n >= 13 => {
            let len = ((n - 13) / 2) as usize;
            let slice = bytes.get(..len)?;
            CarvedValue::Text(String::from_utf8_lossy(slice).into_owned())
        }
        _ => return None,
    };
    Some(val)
}

/// Read `n` (≤ 8) big-endian bytes from the front of `bytes` into a `u64`.
/// Returns `None` if fewer than `n` bytes are present.
fn read_be_int(bytes: &[u8], n: usize) -> Option<u64> {
    let slice = bytes.get(..n)?;
    let mut acc: u64 = 0;
    for &b in slice {
        acc = (acc << 8) | u64::from(b);
    }
    Some(acc)
}

/// Sign-extend an `n`-byte (n < 8) magnitude held in the low bits of `raw` to a
/// full two's-complement `i64`. Used for the 3- and 6-byte integer serial types.
fn sign_extend(raw: u64, n: usize) -> i64 {
    let bits = n * 8;
    let shift = 64 - bits;
    ((raw << shift) as i64) >> shift
}

// ---------------------------------------------------------------------------
// Record decoding (the schema-less core).
// ---------------------------------------------------------------------------

/// Outcome of decoding the *record body* (header + values) at the start of
/// `data`: the column values, plus whether the payload was truncated.
struct DecodedRecord {
    values: Vec<CarvedValue>,
    truncated: bool,
}

/// Try to decode a SQLite **record** (header-size varint, serial-type varints,
/// then the column contents) at the very start of `data`.
///
/// `avail` is the number of payload bytes actually present on this page; when the
/// record's declared content runs past `avail` we recover the columns that *do*
/// fit and set `truncated` (an overflow spill we do not follow). Returns `None`
/// when the bytes are not a self-consistent record header, which is how the
/// sliding-window scanner rejects the overwhelming majority of false offsets.
fn decode_record(data: &[u8], avail: usize) -> Option<DecodedRecord> {
    let (header_size_i, hs_len) = read_varint(data)?;
    if header_size_i < 0 {
        return None;
    }
    let header_size = header_size_i as usize;
    // The header-size varint is itself counted inside header_size, so the field
    // must be at least as large as that varint and fit within the buffer.
    if header_size < hs_len || header_size > data.len() || header_size > MAX_RECORD_SIZE {
        return None;
    }

    // Parse the serial-type array that fills the remainder of the header.
    let header = data.get(..header_size)?;
    let mut serials: Vec<u64> = Vec::new();
    let mut pos = hs_len;
    let mut content_len: usize = 0;
    while pos < header_size {
        let (serial_i, used) = read_varint(header.get(pos..)?)?;
        if serial_i < 0 {
            return None;
        }
        let serial = serial_i as u64;
        // Reserved serial types 10/11 make the whole record invalid.
        let vlen = serial_type_len(serial)?;
        content_len = content_len.checked_add(vlen)?;
        if content_len > MAX_RECORD_SIZE {
            return None;
        }
        serials.push(serial);
        if serials.len() > MAX_COLUMNS {
            return None;
        }
        pos = pos.checked_add(used)?;
    }
    // The serial-type array must consume the header exactly; a varint that
    // straddles the header boundary means we mis-framed the candidate.
    if pos != header_size {
        return None;
    }
    // A record with no columns is not a useful recovery and is almost always a
    // false positive (e.g. a run of zero bytes decoding as header_size=1).
    if serials.is_empty() {
        return None;
    }

    // Reject a record whose declared header+content size overflows `usize`
    // (corrupt framing); the value itself is not needed beyond this check.
    header_size.checked_add(content_len)?;
    // How many content bytes are actually readable here, bounded by both the
    // buffer and the page's available payload.
    let usable_end = avail.min(data.len());
    let mut cursor = header_size;
    let mut values = Vec::with_capacity(serials.len());
    let mut truncated = false;
    for serial in serials {
        let vlen = serial_type_len(serial)?;
        let end = cursor.checked_add(vlen)?;
        if end > usable_end {
            // Payload spilled past what we have (overflow page or overwritten
            // tail). Recover the prefix decoded so far and mark truncation.
            truncated = true;
            break;
        }
        let slice = data.get(cursor..end)?;
        let value = decode_value(serial, slice)?;
        values.push(value);
        cursor = end;
    }
    if values.is_empty() {
        return None;
    }

    Some(DecodedRecord { values, truncated })
}

/// Try to decode a full **table b-tree leaf cell** at the start of `data`:
/// payload-length varint, rowid varint, then the record. Returns the carved
/// record on success. `avail` bounds the readable payload (for truncation
/// detection). This is the high-confidence path used when we can frame on real
/// cell boundaries (e.g. the cell-pointer array of a freelist page image).
fn decode_cell(data: &[u8], avail: usize, source: CarveSource) -> Option<CarvedRecord> {
    let (payload_len_i, pl_used) = read_varint(data)?;
    if payload_len_i <= 0 {
        return None;
    }
    let payload_len = payload_len_i as usize;
    if payload_len > MAX_RECORD_SIZE {
        return None;
    }
    let (rowid, rid_used) = read_varint(data.get(pl_used..)?)?;
    let record_off = pl_used.checked_add(rid_used)?;
    let record = data.get(record_off..)?;
    // The payload region available for this cell, clipped to what's on the page.
    let payload_avail = avail.saturating_sub(record_off).min(payload_len);
    let decoded = decode_record(record, payload_avail)?;
    Some(CarvedRecord {
        rowid: Some(rowid),
        source,
        values: decoded.values,
        truncated: decoded.truncated || payload_len > payload_avail,
    })
}

// ---------------------------------------------------------------------------
// Region scanning (sliding window).
// ---------------------------------------------------------------------------

/// Whether a record decoded by the *raw* sliding window is plausible enough to
/// keep. Raw scans (no cell framing) throw off many false positives — a run of
/// zero bytes happily "decodes" as a record of NULLs, and a greedy header can
/// swallow a whole free region as one giant TEXT/BLOB. This gate rejects those:
///
/// * every value being NULL (almost always a zero-fill artefact),
/// * any single TEXT/BLOB longer than [`MAX_RAW_VALUE_LEN`] (a swallowed region).
///
/// Cell-framed decodes bypass this gate — there the boundaries are real.
fn raw_record_is_plausible(values: &[CarvedValue]) -> bool {
    if values.iter().all(|v| matches!(v, CarvedValue::Null)) {
        return false;
    }
    for v in values {
        let len = match v {
            CarvedValue::Text(s) => s.len(),
            CarvedValue::Blob(b) => b.len(),
            _ => 0,
        };
        if len > MAX_RAW_VALUE_LEN {
            return false;
        }
    }
    true
}

/// Slide over `region` one byte at a time, attempting to decode a record at each
/// offset and pushing every self-consistent, plausible hit into `out`. Used for
/// free regions where the cell framing may be damaged (freeblocks, the
/// unallocated gap, whole free pages, WAL frame bodies): cell boundaries are
/// unknown, so we probe every position rather than trusting a greedy advance,
/// which would let one bogus large record mask all the genuine ones behind it.
///
/// Both a full cell framing (payload-len + rowid + record, which recovers the
/// rowid) and a bare record are attempted at each offset. Identical records are
/// de-duplicated within the region. The loop is bounded by
/// [`MAX_CANDIDATES_PER_REGION`] and the global record cap.
fn scan_region(region: &[u8], source: CarveSource, out: &mut Vec<CarvedRecord>) {
    // De-duplicate within the region. `CarvedValue` holds an `f64` (so it is not
    // `Eq`/`Hash`); we therefore key on a cheap byte fingerprint of the values
    // rather than the values themselves. The set is bounded by the candidate cap.
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut offset = 0usize;
    while offset < region.len() {
        if out.len() >= MAX_TOTAL_RECORDS || offset >= MAX_CANDIDATES_PER_REGION {
            return;
        }
        let window = &region[offset..];
        // Full cell framing first (recovers the rowid). Even here, gate on
        // plausibility: a raw region offset that happens to satisfy the framing
        // can still produce a swallowed-region value.
        if let Some(rec) = decode_cell(window, window.len(), source)
            && raw_record_is_plausible(&rec.values)
            && seen.insert(fingerprint(&rec.values))
        {
            out.push(rec);
        } else if let Some(decoded) = decode_record(window, window.len())
            && raw_record_is_plausible(&decoded.values)
            && seen.insert(fingerprint(&decoded.values))
        {
            out.push(CarvedRecord {
                rowid: None,
                source,
                values: decoded.values,
                truncated: decoded.truncated,
            });
        }
        offset += 1;
    }
}

/// A cheap, order-sensitive fingerprint of a value list, used only to suppress
/// duplicate carves within one region (an FNV-1a hash over a compact encoding of
/// each value). Collisions merely drop a near-identical duplicate, which is
/// harmless for recovery; it never affects correctness of kept records.
fn fingerprint(values: &[CarvedValue]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for v in values {
        match v {
            CarvedValue::Null => mix(&[0]),
            CarvedValue::Int(i) => {
                mix(&[1]);
                mix(&i.to_le_bytes());
            }
            CarvedValue::Real(r) => {
                mix(&[2]);
                mix(&r.to_bits().to_le_bytes());
            }
            CarvedValue::Text(s) => {
                mix(&[3]);
                mix(s.as_bytes());
            }
            CarvedValue::Blob(b) => {
                mix(&[4]);
                mix(b);
            }
        }
        mix(&[0xff]); // value separator
    }
    h
}

/// Decode a table-leaf page using its (possibly stale) **cell-pointer array** —
/// the high-confidence path. A page freed wholesale (freelist leaf) or captured
/// in a WAL frame keeps its old 0x0d header and pointer array intact, so each
/// pointed cell still has correct framing. We honour the recorded cell count and
/// each pointer, decoding the cell at its true boundary (no plausibility gate —
/// these boundaries are trustworthy, so even single-column rows are kept).
///
/// `header_offset` is 0 for a standalone page image and 100 only for database
/// page 1. Returns silently on any inconsistency (bounds-checked throughout).
fn carve_leaf_cells(
    page: &[u8],
    usable_size: usize,
    header_offset: usize,
    source: CarveSource,
    out: &mut Vec<CarvedRecord>,
) {
    let Some(h) = page.get(header_offset..) else {
        return;
    };
    if h.first().copied() != Some(0x0d) {
        return;
    }
    let cell_count = match (h.get(3), h.get(4)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
        _ => return,
    };
    // The pointer array sits right after the 8-byte leaf header and must fit in
    // the usable area; clamp the honoured count defensively.
    let ptr_base = header_offset + 8;
    let max_ptrs = usable_size.saturating_sub(ptr_base) / 2;
    let cell_count = cell_count.min(max_ptrs);
    for i in 0..cell_count {
        if out.len() >= MAX_TOTAL_RECORDS {
            return;
        }
        let po = ptr_base + i * 2;
        let cell_off = match (page.get(po), page.get(po + 1)) {
            (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
            _ => return,
        };
        if cell_off >= usable_size || cell_off >= page.len() {
            continue;
        }
        let avail = usable_size.min(page.len()).saturating_sub(cell_off);
        if let Some(window) = page.get(cell_off..)
            && let Some(rec) = decode_cell(window, avail, source)
        {
            out.push(rec);
        }
    }
}

// ---------------------------------------------------------------------------
// Page-level parsing.
// ---------------------------------------------------------------------------

/// The parsed database header fields we need.
struct DbHeader {
    page_size: usize,
    reserved: usize,
    freelist_trunk: u32,
    #[allow(dead_code)]
    freelist_count: u32,
}

/// The SQLite file magic (16 bytes including the trailing NUL).
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Whether `page_size` is a legal SQLite page size: a power of two within
/// `[MIN_PAGE_SIZE, MAX_PAGE_SIZE]`. Used for both the database and WAL headers.
fn is_valid_page_size(page_size: usize) -> bool {
    (MIN_PAGE_SIZE..=MAX_PAGE_SIZE).contains(&page_size) && page_size.is_power_of_two()
}

/// Parse the 100-byte database header. Returns `None` for anything that is not a
/// plausible SQLite file (bad magic, illegal page size, reserved region larger
/// than the page).
fn parse_db_header(main: &[u8]) -> Option<DbHeader> {
    let header = main.get(..100)?;
    if header.get(..16)? != SQLITE_MAGIC {
        return None;
    }
    let raw_ps = u16::from_be_bytes([*header.get(16)?, *header.get(17)?]);
    // A page_size field of 1 means 65536 (it cannot be stored in the u16).
    let page_size = if raw_ps == 1 {
        MAX_PAGE_SIZE
    } else {
        raw_ps as usize
    };
    if !is_valid_page_size(page_size) {
        return None;
    }
    let reserved = *header.get(20)? as usize;
    if reserved >= page_size {
        return None;
    }
    let freelist_trunk = u32::from_be_bytes([
        *header.get(32)?,
        *header.get(33)?,
        *header.get(34)?,
        *header.get(35)?,
    ]);
    let freelist_count = u32::from_be_bytes([
        *header.get(36)?,
        *header.get(37)?,
        *header.get(38)?,
        *header.get(39)?,
    ]);
    Some(DbHeader {
        page_size,
        reserved,
        freelist_trunk,
        freelist_count,
    })
}

/// Byte slice of 1-indexed `page_no` within `data`, or `None` if it does not fit.
fn page_bytes(data: &[u8], page_no: u32, page_size: usize) -> Option<&[u8]> {
    if page_no == 0 {
        return None;
    }
    let start = (page_no as usize).checked_sub(1)?.checked_mul(page_size)?;
    let end = start.checked_add(page_size)?;
    data.get(start..end)
}

/// Parse a single table b-tree **leaf** page (type byte `0x0d`), harvesting both
/// its freeblock chain and its unallocated gap. `page_start` is the byte offset
/// of this page within the file; `header_offset` is 0 for every page except
/// page 1, whose b-tree header begins after the 100-byte database header.
fn carve_leaf_page(
    page: &[u8],
    usable_size: usize,
    header_offset: usize,
    out: &mut Vec<CarvedRecord>,
) {
    let h = match page.get(header_offset..) {
        Some(h) => h,
        None => return,
    };
    // Only table-leaf pages (0x0d) carry the cells we decode here. Interior and
    // index pages are skipped (their freed cells, if any, surface via freelist /
    // WAL scanning of the raw page anyway).
    if h.first().copied() != Some(0x0d) {
        return;
    }
    let cell_count = match (h.get(3), h.get(4)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
        _ => return,
    };
    let first_freeblock = match (h.get(1), h.get(2)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
        _ => 0,
    };
    let raw_content_start = match (h.get(5), h.get(6)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
        _ => return,
    };
    // A stored content-start of 0 means 65536.
    let content_start = if raw_content_start == 0 {
        MAX_PAGE_SIZE
    } else {
        raw_content_start
    };

    // --- Unallocated gap: between the end of the cell-pointer array and the
    // start of the cell-content area. Freshly deleted cells often sit here. ---
    // Leaf header is 8 bytes; cell pointers are 2 bytes each, right after it.
    let ptr_array_end = header_offset
        .checked_add(8)
        .and_then(|x| x.checked_add(cell_count.checked_mul(2)?));
    if let Some(gap_start) = ptr_array_end {
        let gap_end = content_start.min(usable_size).min(page.len());
        if gap_start < gap_end
            && let Some(gap) = page.get(gap_start..gap_end)
        {
            scan_region(gap, CarveSource::Unallocated, out);
        }
    }

    // --- Freeblock chain: starting at first-freeblock, each freeblock is
    // next(u16) + size(u16) then free bytes. Walk with a visited-set + cap. ---
    if first_freeblock != 0 {
        let mut seen = std::collections::HashSet::new();
        let mut fb = first_freeblock;
        let mut hops = 0usize;
        while fb != 0 && fb + 4 <= usable_size.min(page.len()) {
            if !seen.insert(fb) {
                break; // cycle
            }
            hops += 1;
            if hops > MAX_CHAIN_HOPS || out.len() >= MAX_TOTAL_RECORDS {
                break;
            }
            let next = match (page.get(fb), page.get(fb + 1)) {
                (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
                _ => break,
            };
            let size = match (page.get(fb + 2), page.get(fb + 3)) {
                (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
                _ => break,
            };
            // The freeblock's free bytes follow the 4-byte header; the old cell
            // that lived here starts at `fb` (the next/size words overwrote only
            // its first 4 bytes, so we also probe from fb to catch the rest).
            let block_end = fb.checked_add(size.max(4)).unwrap_or(usable_size);
            let region_end = block_end.min(usable_size).min(page.len());
            if let Some(region) = page.get(fb..region_end) {
                scan_region(region, CarveSource::Freeblock, out);
            }
            fb = next;
        }
    }
}

// ---------------------------------------------------------------------------
// Freelist walking.
// ---------------------------------------------------------------------------

/// Walk the freelist trunk chain from `header.freelist_trunk`, scanning every
/// freelist **leaf** page (a wholly-freed page whose former contents may remain).
/// Trunk pages are walked with a visited-set + cap; each trunk lists up to L leaf
/// page numbers.
fn carve_freelist(main: &[u8], header: &DbHeader, out: &mut Vec<CarvedRecord>) {
    let page_size = header.page_size;
    let usable = page_size - header.reserved;
    let mut seen = std::collections::HashSet::new();
    let mut trunk = header.freelist_trunk;
    let mut hops = 0usize;
    let mut pages_scanned = 0usize;

    while trunk != 0 {
        if !seen.insert(trunk) {
            break; // cycle in trunk chain
        }
        hops += 1;
        if hops > MAX_CHAIN_HOPS || out.len() >= MAX_TOTAL_RECORDS {
            break;
        }
        let Some(tpage) = page_bytes(main, trunk, page_size) else {
            break;
        };
        let next_trunk = match tpage.get(..4) {
            Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            None => break,
        };
        let leaf_count = match tpage.get(4..8) {
            Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize,
            None => break,
        };
        // The leaf-pointer array must fit in the page; clamp defensively.
        let max_leaves = (usable.saturating_sub(8)) / 4;
        let leaf_count = leaf_count.min(max_leaves);
        for i in 0..leaf_count {
            if out.len() >= MAX_TOTAL_RECORDS || pages_scanned >= MAX_PAGES_SCANNED {
                return;
            }
            let off = 8 + i * 4;
            let leaf_no = match tpage.get(off..off + 4) {
                Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                None => break,
            };
            if leaf_no == 0 || !seen.insert(leaf_no) {
                continue;
            }
            if let Some(lpage) = page_bytes(main, leaf_no, page_size) {
                pages_scanned += 1;
                // A page freed wholesale usually keeps its old 0x0d header and
                // cell-pointer array intact, so decode it via that array first
                // (high confidence, recovers rowids). Then also slide over the
                // whole usable area to catch records the array no longer points
                // at (e.g. cells freed before the page itself was freed).
                carve_leaf_cells(lpage, usable, 0, CarveSource::Freelist, out);
                if let Some(region) = lpage.get(..usable.min(lpage.len())) {
                    scan_region(region, CarveSource::Freelist, out);
                }
            }
        }
        trunk = next_trunk;
    }
}

// ---------------------------------------------------------------------------
// WAL walking.
// ---------------------------------------------------------------------------

/// WAL header magic, big-endian variants (0x377f0682 / 0x377f0683).
const WAL_MAGIC_BE_EVEN: u32 = 0x377f_0682;
const WAL_MAGIC_BE_ODD: u32 = 0x377f_0683;
/// WAL file header length.
const WAL_HEADER_LEN: usize = 32;
/// Per-frame header length.
const WAL_FRAME_HEADER_LEN: usize = 24;

/// Scan every frame body in a `-wal` sidecar, including superseded frames for the
/// same page number (they hold pre-deletion page images). The frame's
/// page-number field is informational only; we scan each body as a raw region.
fn carve_wal(wal: &[u8], out: &mut Vec<CarvedRecord>) {
    let header = match wal.get(..WAL_HEADER_LEN) {
        Some(h) => h,
        None => return,
    };
    let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if magic != WAL_MAGIC_BE_EVEN && magic != WAL_MAGIC_BE_ODD {
        return;
    }
    // Page size is at offset 8 in the WAL header (big-endian u32; 1 → 65536).
    let raw_ps = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let page_size = if raw_ps == 1 {
        MAX_PAGE_SIZE
    } else {
        raw_ps as usize
    };
    if !is_valid_page_size(page_size) {
        return;
    }

    let frame_size = WAL_FRAME_HEADER_LEN + page_size;
    let mut off = WAL_HEADER_LEN;
    let mut frames = 0usize;
    while off + frame_size <= wal.len() {
        if out.len() >= MAX_TOTAL_RECORDS || frames >= MAX_PAGES_SCANNED {
            return;
        }
        frames += 1;
        let body_start = off + WAL_FRAME_HEADER_LEN;
        let body_end = body_start + page_size;
        if let Some(body) = wal.get(body_start..body_end) {
            // A WAL frame body is a full page image. Decode it three ways:
            //   1. via its cell-pointer array (intact pre-deletion cells),
            //   2. its freeblocks + unallocated gap (cells deleted before this
            //      frame was written),
            //   3. a raw slide over the whole body as a backstop.
            carve_leaf_cells(body, page_size, 0, CarveSource::Wal, out);
            carve_leaf_page(body, page_size, 0, out);
            scan_region(body, CarveSource::Wal, out);
        }
        off += frame_size;
    }
}

// ---------------------------------------------------------------------------
// Public entry point.
// ---------------------------------------------------------------------------

/// Carve every plausibly-decoded **deleted** record out of a raw SQLite database
/// (`main`) and, optionally, its write-ahead log (`wal`).
///
/// Scans all four free regions — freelist pages, in-page freeblocks, the
/// unallocated gap of every table-leaf page, and (when supplied) every WAL frame
/// body — and returns each self-consistent record it can decode. The result may
/// contain duplicates (the same deleted row can appear in several regions, e.g.
/// in both a freeblock and a WAL frame); de-duplication and table attribution are
/// the caller's job.
///
/// Robust against hostile input: a file with a bad header yields an empty `Vec`
/// (never a panic), and the global caps documented at the top of this module
/// bound memory and time regardless of how corrupt the bytes are.
pub fn carve_sqlite(main: &[u8], wal: Option<&[u8]>) -> Vec<CarvedRecord> {
    let mut out = Vec::new();

    if let Some(header) = parse_db_header(main) {
        let page_size = header.page_size;
        let usable = page_size - header.reserved;

        // Walk every page as a potential table-leaf page (harvesting freeblocks
        // and the unallocated gap). Page 1's b-tree header sits after the
        // 100-byte database header.
        let total_pages = main.len() / page_size;
        let page_cap = total_pages.min(MAX_PAGES_SCANNED);
        for idx in 0..page_cap {
            if out.len() >= MAX_TOTAL_RECORDS {
                break;
            }
            let page_no = (idx + 1) as u32;
            if let Some(page) = page_bytes(main, page_no, page_size) {
                let header_offset = if page_no == 1 { 100 } else { 0 };
                carve_leaf_page(page, usable, header_offset, &mut out);
            }
        }

        // Freelist pages (wholly freed pages whose old content lingers).
        if out.len() < MAX_TOTAL_RECORDS {
            carve_freelist(main, &header, &mut out);
        }
    }

    // WAL sidecar (independent of the main header parse — a WAL can be present
    // even when the main file's header looks odd).
    if let Some(wal_bytes) = wal
        && out.len() < MAX_TOTAL_RECORDS
    {
        carve_wal(wal_bytes, &mut out);
    }

    out
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // ---- Low-level: varint edge cases ------------------------------------

    #[test]
    fn varint_single_byte() {
        assert_eq!(read_varint(&[0x00]), Some((0, 1)));
        assert_eq!(read_varint(&[0x7f]), Some((127, 1)));
    }

    #[test]
    fn varint_two_bytes() {
        // 0x81 0x00 => continuation, value 128.
        assert_eq!(read_varint(&[0x81, 0x00]), Some((128, 2)));
    }

    #[test]
    fn varint_nine_bytes_uses_full_last_byte() {
        // Eight 0xFF continuation bytes then a final 0xFF: all bits set → -1.
        let bytes = [0xff; 9];
        let (val, len) = read_varint(&bytes).unwrap();
        assert_eq!(len, 9);
        assert_eq!(val, -1);
    }

    #[test]
    fn varint_empty_is_none() {
        assert_eq!(read_varint(&[]), None);
    }

    #[test]
    fn varint_truncated_continuation_is_none() {
        // High bit set but no following byte within the 8-byte window.
        assert_eq!(read_varint(&[0x81]), None);
    }

    // ---- Low-level: serial-type / value decoding -------------------------

    #[test]
    fn serial_type_widths() {
        assert_eq!(serial_type_len(0), Some(0));
        assert_eq!(serial_type_len(6), Some(8));
        assert_eq!(serial_type_len(7), Some(8));
        assert_eq!(serial_type_len(8), Some(0));
        assert_eq!(serial_type_len(9), Some(0));
        assert_eq!(serial_type_len(10), None); // reserved
        assert_eq!(serial_type_len(11), None); // reserved
        assert_eq!(serial_type_len(12), Some(0)); // empty blob
        assert_eq!(serial_type_len(13), Some(0)); // empty text
        assert_eq!(serial_type_len(14), Some(1)); // 1-byte blob
        assert_eq!(serial_type_len(15), Some(1)); // 1-byte text
    }

    #[test]
    fn decode_negative_one_byte_int() {
        assert_eq!(decode_value(1, &[0xff]), Some(CarvedValue::Int(-1)));
    }

    #[test]
    fn decode_three_byte_negative_int_sign_extends() {
        // 0xFFFFFF as a 3-byte signed int is -1.
        assert_eq!(decode_value(3, &[0xff, 0xff, 0xff]), Some(CarvedValue::Int(-1)));
    }

    #[test]
    fn decode_constant_ints() {
        assert_eq!(decode_value(8, &[]), Some(CarvedValue::Int(0)));
        assert_eq!(decode_value(9, &[]), Some(CarvedValue::Int(1)));
    }

    #[test]
    fn decode_real_roundtrips() {
        let bits = 12345.678_f64.to_bits().to_be_bytes();
        assert_eq!(decode_value(7, &bits), Some(CarvedValue::Real(12345.678)));
    }

    #[test]
    fn decode_text_and_blob() {
        // serial 19 = (19-13)/2 = 3-byte text.
        assert_eq!(
            decode_value(19, b"abc"),
            Some(CarvedValue::Text("abc".to_string()))
        );
        // serial 18 = (18-12)/2 = 3-byte blob.
        assert_eq!(
            decode_value(18, b"\x01\x02\x03"),
            Some(CarvedValue::Blob(vec![1, 2, 3]))
        );
    }

    #[test]
    fn decode_text_invalid_utf8_is_lossy_not_dropped() {
        // serial 15 = 1-byte text; 0xFF is invalid UTF-8 → replacement char.
        let v = decode_value(15, &[0xff]).unwrap();
        match v {
            CarvedValue::Text(s) => assert!(s.contains('\u{FFFD}')),
            other => panic!("expected lossy text, got {other:?}"),
        }
    }

    // ---- Fixture helpers --------------------------------------------------

    /// Build a fresh on-disk SQLite file, run `setup`, and return its path +
    /// tempdir guard.
    fn build_db(
        setup: impl FnOnce(&Connection),
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let conn = Connection::open(&path).unwrap();
        setup(&conn);
        conn.close().unwrap();
        (dir, path)
    }

    /// True if any carved record's values contain a Text equal to `needle`.
    fn has_text(records: &[CarvedRecord], needle: &str) -> bool {
        records.iter().any(|r| {
            r.values
                .iter()
                .any(|v| matches!(v, CarvedValue::Text(s) if s == needle))
        })
    }

    /// True if any carved record contains the given integer.
    fn has_int(records: &[CarvedRecord], n: i64) -> bool {
        records.iter().any(|r| {
            r.values.iter().any(|v| matches!(v, CarvedValue::Int(x) if *x == n))
        })
    }

    // ---- Integration: freelist / unallocated recovery --------------------

    #[test]
    fn recovers_dropped_table_rows_from_freelist() {
        // Dropping a populated table returns its leaf pages to the freelist with
        // their cell-pointer arrays (and thus complete cells) intact — the
        // textbook freelist-recovery case.
        let (_dir, path) = build_db(|conn| {
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, age INTEGER);",
            )
            .unwrap();
            for i in 0..200 {
                conn.execute(
                    "INSERT INTO people (name, age) VALUES (?1, ?2)",
                    rusqlite::params![format!("Person_{i:03}_SECRET"), 1000 + i],
                )
                .unwrap();
            }
            conn.execute("DROP TABLE people", []).unwrap();
        });

        let bytes = std::fs::read(&path).unwrap();
        let records = carve_sqlite(&bytes, None);

        // Most of the 200 dropped names should be recovered from freelist pages.
        let recovered = (0..200)
            .filter(|i| has_text(&records, &format!("Person_{i:03}_SECRET")))
            .count();
        assert!(
            recovered >= 100,
            "expected to recover most dropped names, got {recovered} of 200 \
             ({} records)",
            records.len()
        );
        // A specific deleted age integer must also come back.
        assert!(has_int(&records, 1042), "expected to recover age 1042");
        // Freelist-sourced records carry their rowid (intact cell framing).
        assert!(
            records
                .iter()
                .any(|r| r.source == CarveSource::Freelist && r.rowid.is_some()),
            "freelist records should recover rowids"
        );
    }

    #[test]
    fn recovers_deleted_row_from_unallocated_gap() {
        // Deleting the most-recently-inserted row (the cell sitting at the top of
        // the cell-content area) leaves its bytes, with intact framing, in the
        // page's unallocated gap — SQLite simply lowers the cell count and
        // content-start rather than writing a freeblock header over it. The raw
        // sliding window over the gap recovers that intact cell.
        let (_dir, path) = build_db(|conn| {
            conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, label TEXT);")
                .unwrap();
            for i in 0..10 {
                conn.execute(
                    "INSERT INTO t (label) VALUES (?1)",
                    rusqlite::params![format!("ROW_{i}_GAPMARKER")],
                )
                .unwrap();
            }
            // id 10 was inserted last → its cell is at the top of content; freeing
            // it leaves it intact in the now-larger unallocated gap.
            conn.execute("DELETE FROM t WHERE id = 10", []).unwrap();
        });
        let bytes = std::fs::read(&path).unwrap();
        let records = carve_sqlite(&bytes, None);
        // The deleted row's label must be recovered with its exact value.
        assert!(
            has_text(&records, "ROW_9_GAPMARKER"),
            "expected to recover the deleted top-of-page row, got {} records",
            records.len()
        );
    }

    // ---- Integration: WAL recovery ---------------------------------------

    #[test]
    fn recovers_deleted_rows_from_wal_without_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("waltest.sqlite");
        let conn = Connection::open(&path).unwrap();
        // WAL mode; the DELETE lands in the -wal and we must NOT checkpoint.
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0i64).unwrap();
        conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
            .unwrap();
        for i in 0..200 {
            conn.execute(
                "INSERT INTO notes (body) VALUES (?1)",
                rusqlite::params![format!("WAL_NOTE_{i:03}_CONFIDENTIAL")],
            )
            .unwrap();
        }
        conn.execute("DELETE FROM notes WHERE id <= 150", []).unwrap();
        // Read the -wal while the connection is still open so SQLite cannot
        // checkpoint-and-truncate it on close. (Closing the last connection would
        // otherwise run a passive checkpoint and reset the WAL.)
        let wal_path = dir.path().join("waltest.sqlite-wal");
        assert!(wal_path.exists(), "WAL file should exist (no checkpoint)");
        let main = std::fs::read(&path).unwrap();
        let wal = std::fs::read(&wal_path).unwrap();
        drop(conn);

        let records = carve_sqlite(&main, Some(&wal));
        let recovered = (0..200)
            .filter(|i| has_text(&records, &format!("WAL_NOTE_{i:03}_CONFIDENTIAL")))
            .count();
        assert!(
            recovered > 0,
            "expected to recover a deleted note from the WAL, got {} records",
            records.len()
        );
        // The cell-pointer-array pass recovers the notes with exact boundaries:
        // at least one carved record must be the clean note text, with its rowid.
        assert!(
            records.iter().any(|r| {
                r.source == CarveSource::Wal
                    && r.rowid.is_some()
                    && r.values.iter().any(|v| {
                        matches!(v, CarvedValue::Text(s) if s == "WAL_NOTE_001_CONFIDENTIAL")
                    })
            }),
            "expected a cleanly-framed WAL note with its rowid"
        );
    }

    // ---- Integration: a record we hand-deleted survives in the gap -------

    #[test]
    fn recovers_specific_deleted_record_values() {
        // A row carrying TEXT *and* REAL columns, deleted last (so it lingers
        // intact in the unallocated gap): we assert the carver brings back both
        // the marker text and the exact floating-point score, proving end-to-end
        // serial-type decoding of a real deleted record.
        let (_dir, path) = build_db(|conn| {
            conn.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, label TEXT, score REAL);",
            )
            .unwrap();
            // A first row that stays live, so the page is not empty after delete.
            conn.execute(
                "INSERT INTO t (label, score) VALUES (?1, ?2)",
                rusqlite::params!["KEEP_ME", 9.0f64],
            )
            .unwrap();
            // The target row, inserted last → top of content area.
            conn.execute(
                "INSERT INTO t (label, score) VALUES (?1, ?2)",
                rusqlite::params!["UNIQUE_MARKER_XYZZY", 2.5f64],
            )
            .unwrap();
            conn.execute("DELETE FROM t WHERE label = 'UNIQUE_MARKER_XYZZY'", [])
                .unwrap();
        });

        let bytes = std::fs::read(&path).unwrap();
        let records = carve_sqlite(&bytes, None);
        assert!(
            has_text(&records, "UNIQUE_MARKER_XYZZY"),
            "deleted marker text must be carved back, got {} records",
            records.len()
        );
        // The deleted REAL value must also be recovered, in the same record.
        assert!(
            records.iter().any(|r| {
                r.values.iter().any(|v| matches!(v, CarvedValue::Text(s) if s == "UNIQUE_MARKER_XYZZY"))
                    && r.values.iter().any(|v| matches!(v, CarvedValue::Real(x) if (*x - 2.5).abs() < 1e-9))
            }),
            "expected the deleted row's text and REAL score together"
        );
    }

    // ---- Robustness: never panic on arbitrary / truncated input ----------

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Deterministic pseudo-random stream (no rand dep): a simple LCG.
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u8
        };
        for size in [0usize, 1, 7, 31, 100, 101, 512, 513, 4096, 9000] {
            let buf: Vec<u8> = (0..size).map(|_| next()).collect();
            // As main only.
            let _ = carve_sqlite(&buf, None);
            // As both main and WAL.
            let wal: Vec<u8> = (0..size).map(|_| next()).collect();
            let _ = carve_sqlite(&buf, Some(&wal));
        }
    }

    #[test]
    fn valid_header_then_garbage_never_panics() {
        // Real magic + a plausible 4096 page size, then random bytes.
        let mut buf = Vec::new();
        buf.extend_from_slice(SQLITE_MAGIC);
        buf.extend_from_slice(&[0x10, 0x00]); // page_size = 4096
        buf.resize(100, 0); // rest of header zeroed
        buf.extend((0..20_000u32).map(|i| (i % 256) as u8));
        let _ = carve_sqlite(&buf, None);
    }

    #[test]
    fn truncated_after_header_never_panics() {
        let mut buf = Vec::new();
        buf.extend_from_slice(SQLITE_MAGIC);
        buf.extend_from_slice(&[0x10, 0x00]); // page_size = 4096
        buf.resize(50, 0); // truncated mid-header
        let _ = carve_sqlite(&buf, None);
    }

    #[test]
    fn page_size_one_means_65536_and_does_not_overflow() {
        let mut buf = Vec::new();
        buf.extend_from_slice(SQLITE_MAGIC);
        buf.extend_from_slice(&[0x00, 0x01]); // page_size field = 1 → 65536
        buf.resize(100, 0);
        // Far smaller than one page; must simply find nothing, not panic.
        let recs = carve_sqlite(&buf, None);
        assert!(recs.is_empty());
    }

    #[test]
    fn bad_magic_returns_empty() {
        let buf = vec![0u8; 4096];
        assert!(carve_sqlite(&buf, None).is_empty());
    }

    #[test]
    fn wal_with_bad_magic_is_ignored() {
        let wal = vec![0u8; 4096];
        // Main is also junk; both ignored, no panic, empty result.
        let recs = carve_sqlite(&[], Some(&wal));
        assert!(recs.is_empty());
    }

    #[test]
    fn carved_record_serializes_to_json() {
        let rec = CarvedRecord {
            rowid: Some(7),
            source: CarveSource::Freeblock,
            values: vec![
                CarvedValue::Null,
                CarvedValue::Int(42),
                CarvedValue::Text("hi".into()),
                CarvedValue::Blob(vec![1, 2]),
            ],
            truncated: false,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"rowid\":7"));
        assert!(json.contains("Freeblock"));
        assert!(json.contains("\"Int\":42"));
    }
}
