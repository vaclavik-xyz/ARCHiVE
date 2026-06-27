//! Read Apple Notes from an iOS `NoteStore.sqlite`. Titles/snippets/folders/dates
//! are plain columns; the note body is a gzip-compressed protobuf blob decoded
//! here (best-effort, with a graceful fallback to the plaintext snippet).

use std::io::Read;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One Apple Note.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    /// Title (`ZTITLE1`); empty when untitled.
    pub title: String,
    /// Containing folder's name; empty when none.
    pub folder: String,
    /// Creation time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub created: String,
    /// Last-modified time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub modified: String,
    /// Note text: the decoded body, the snippet fallback, or empty.
    pub body: String,
    /// Which path produced `body`: `decoded` | `snippet` | `empty`.
    pub body_source: String,
}

/// Parse all notes, decoding each body and ordering by last-modified (newest
/// first). Returns an empty list when the schema has no `ZNOTEDATA` column
/// (nothing recognizable as a note); tolerates other optional columns.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Note>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZICCLOUDSYNCINGOBJECT")?;
    if !cols.contains("ZNOTEDATA") {
        return Ok(Vec::new());
    }
    let title_sel = if cols.contains("ZTITLE1") { "c.ZTITLE1" } else { "NULL" };
    let snippet_sel = if cols.contains("ZSNIPPET") { "c.ZSNIPPET" } else { "NULL" };
    let created_sel = if cols.contains("ZCREATIONDATE") { "c.ZCREATIONDATE" } else { "NULL" };
    let modified_sel = if cols.contains("ZMODIFICATIONDATE1") { "c.ZMODIFICATIONDATE1" } else { "NULL" };
    let has_folder = cols.contains("ZFOLDER") && cols.contains("ZTITLE2");
    let folder_sel = if has_folder { "f.ZTITLE2" } else { "NULL" };
    let folder_join = if has_folder {
        "LEFT JOIN ZICCLOUDSYNCINGOBJECT f ON f.Z_PK = c.ZFOLDER"
    } else {
        ""
    };
    let order = if cols.contains("ZMODIFICATIONDATE1") {
        "ORDER BY c.ZMODIFICATIONDATE1 DESC"
    } else {
        ""
    };

    let sql = format!(
        "SELECT {title_sel}, {snippet_sel}, {folder_sel}, {created_sel}, {modified_sel}, d.ZDATA \
         FROM ZICCLOUDSYNCINGOBJECT c \
         LEFT JOIN ZICNOTEDATA d ON d.Z_PK = c.ZNOTEDATA \
         {folder_join} \
         WHERE c.ZNOTEDATA IS NOT NULL {order}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let title: Option<String> = row.get(0)?;
        let snippet: Option<String> = row.get(1)?;
        let folder: Option<String> = row.get(2)?;
        let created: Option<f64> = row.get(3)?;
        let modified: Option<f64> = row.get(4)?;
        let zdata: Option<Vec<u8>> = row.get(5)?;
        let (body, body_source) = resolve_body(zdata.as_deref(), snippet.as_deref());
        Ok(Note {
            title: title.unwrap_or_default(),
            folder: folder.unwrap_or_default(),
            created: created.and_then(cocoa_to_iso).unwrap_or_default(),
            modified: modified.and_then(cocoa_to_iso).unwrap_or_default(),
            body,
            body_source,
        })
    })?;
    rows.collect()
}

/// Choose the note body: the decoded blob if available, else the snippet, else
/// empty. Returns `(body, body_source)`.
fn resolve_body(zdata: Option<&[u8]>, snippet: Option<&str>) -> (String, String) {
    if let Some(blob) = zdata
        && let Some(text) = decode_body(blob)
    {
        return (text, "decoded".to_string());
    }
    match snippet {
        Some(s) if !s.trim().is_empty() => (s.to_string(), "snippet".to_string()),
        _ => (String::new(), "empty".to_string()),
    }
}

/// Decode an Apple Notes body blob: gunzip, then walk the protobuf to the note
/// text at field path 2 → 3 → 2 (`document` → `note` → `note_text`). Returns
/// `None` on any failure (caller falls back to the snippet). Never panics.
pub(crate) fn decode_body(blob: &[u8]) -> Option<String> {
    let inflated = inflate(blob)?;
    let text = field_bytes(&inflated, 2)
        .and_then(|doc| field_bytes(doc, 3))
        .and_then(|note| field_bytes(note, 2))?;
    let s = String::from_utf8_lossy(text).into_owned();
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Decompress a gzip (or, as a fallback, zlib) stream. `None` on failure. The
/// output is capped (a note body is text — far below the cap) so a crafted blob
/// cannot decompress to unbounded memory.
fn inflate(blob: &[u8]) -> Option<Vec<u8>> {
    const MAX_DECOMPRESSED: u64 = 64 * 1024 * 1024;
    let mut out = Vec::new();
    if flate2::read::GzDecoder::new(blob).take(MAX_DECOMPRESSED).read_to_end(&mut out).is_ok()
        && !out.is_empty()
    {
        return Some(out);
    }
    out.clear();
    if flate2::read::ZlibDecoder::new(blob).take(MAX_DECOMPRESSED).read_to_end(&mut out).is_ok()
        && !out.is_empty()
    {
        return Some(out);
    }
    None
}

/// Read a protobuf varint at `*pos`, advancing it. `None` on truncation/overflow.
fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let &b = buf.get(*pos)?;
        *pos += 1;
        result |= u64::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Bytes of the first length-delimited (wire type 2) field numbered `field` in a
/// protobuf message; skips other fields and wire types. Bounds-checked — returns
/// `None` rather than panicking on malformed input.
fn field_bytes(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut pos = 0;
    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let fnum = tag >> 3;
        let wire = tag & 0x7;
        match wire {
            0 => {
                read_varint(buf, &mut pos)?;
            }
            1 => pos = pos.checked_add(8)?,
            5 => pos = pos.checked_add(4)?,
            2 => {
                let len = read_varint(buf, &mut pos)? as usize;
                let end = pos.checked_add(len)?;
                if end > buf.len() {
                    return None;
                }
                if fnum == field {
                    return Some(&buf[pos..end]);
                }
                pos = end;
            }
            _ => return None, // groups (3/4) / unknown — bail out
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_notes;
    use std::io::Write;

    /// Encode one length-delimited protobuf field (field number < 16).
    fn encode_ld(field: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![((field << 3) | 2) as u8];
        let mut len = payload.len() as u64;
        loop {
            let mut byte = (len & 0x7f) as u8;
            len >>= 7;
            if len != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(payload);
        out
    }

    /// Build `gzip(protobuf(2 → 3 → 2 = text))`, the Apple Notes body shape.
    fn gzip_note_text(text: &str) -> Vec<u8> {
        let note = encode_ld(2, text.as_bytes());
        let document = encode_ld(3, &note);
        let top = encode_ld(2, &document);
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&top).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn field_bytes_skips_other_fields_and_wire_types() {
        // field 1 varint = 7, field 2 (64-bit) = 8 bytes, field 3 (len) = "hi".
        let mut buf = vec![1 << 3, 7]; // field 1 varint (tag = field<<3 | wire 0)
        buf.extend_from_slice(&[(2 << 3) | 1, 0, 0, 0, 0, 0, 0, 0, 0]); // field 2, 64-bit
        buf.extend(encode_ld(3, b"hi")); // field 3 len-delimited
        assert_eq!(field_bytes(&buf, 3), Some(&b"hi"[..]));
        assert_eq!(field_bytes(&buf, 9), None);
    }

    #[test]
    fn field_bytes_is_bounds_safe_on_truncation() {
        // tag says field 2 len-delimited, length 50, but no payload follows.
        let buf = vec![(2 << 3) | 2, 50];
        assert_eq!(field_bytes(&buf, 2), None); // no panic
        assert_eq!(read_varint(&[0xff], &mut 0), None); // truncated varint
    }

    #[test]
    fn decode_body_extracts_nested_text() {
        let blob = gzip_note_text("Nákupní seznam\nmléko");
        assert_eq!(decode_body(&blob).as_deref(), Some("Nákupní seznam\nmléko"));
    }

    #[test]
    fn decode_body_returns_none_for_non_gzip() {
        assert_eq!(decode_body(b"not gzip at all"), None);
        assert_eq!(decode_body(&[]), None);
    }

    #[test]
    fn parses_notes_with_body_and_snippet_fallback() {
        let dir = std::env::temp_dir().join(format!("be-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("NoteStore.sqlite");
        let _ = std::fs::remove_file(&db);
        let blob = gzip_note_text("Plný text první poznámky");
        make_notes(&db, &blob);

        let notes = parse(&db).unwrap();
        assert_eq!(notes.len(), 2);
        // ORDER BY ZMODIFICATIONDATE1 DESC → the snippet-only note (600000600) first.
        assert_eq!(notes[0].title, "Druhá");
        assert_eq!(notes[0].body, "jen náhled");
        assert_eq!(notes[0].body_source, "snippet");
        assert_eq!(notes[0].folder, "Práce");
        // The note with a decodable blob.
        assert_eq!(notes[1].title, "Nákup");
        assert_eq!(notes[1].body, "Plný text první poznámky");
        assert_eq!(notes[1].body_source, "decoded");
        assert_eq!(notes[1].created, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_returns_empty_without_znotedata_column() {
        let dir = std::env::temp_dir().join(format!("be-notes-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("NoteStore.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE ZICCLOUDSYNCINGOBJECT (Z_PK INTEGER PRIMARY KEY, ZTITLE1 TEXT);")
            .unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
