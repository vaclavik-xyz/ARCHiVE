//! Bundle a directory of exports into a single **WinZip AES-256 encrypted** `.zip`
//! for secure delivery — any standard zip tool opens it with the password. Used by
//! the `package` command to wrap a prior `recover`/export output into one
//! protected file.
//!
//! Scope of protection: each file's **contents** are AES-256 encrypted, so the data
//! cannot be read without the password. As with any standard encrypted zip, the ZIP
//! central directory is **not** encrypted — `unzip -l` still lists the entry names
//! and sizes — so the filenames (which here are export/store names like
//! `contacts.csv`) are visible, only their contents are not. Hiding the names too
//! would require a non-standard container that ordinary zip tools could not open.

use std::io;
use std::path::Path;

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{AesMode, CompressionMethod};

/// Outcome of packaging, for the JSON envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageSummary {
    /// Number of files written into the archive.
    pub files: usize,
    /// Total uncompressed bytes packaged.
    pub bytes: u64,
}

/// Zip every regular file under `source` (recursively, paths kept relative to
/// `source`) into `zip_path`, each entry AES-256 encrypted with `password`. If the
/// output file happens to live under `source`, it is skipped so the archive never
/// tries to contain itself. Files are added in a deterministic (sorted) order.
pub fn package_dir(source: &Path, zip_path: &Path, password: &str) -> io::Result<PackageSummary> {
    let mut files = Vec::new();
    collect_files(source, &mut files)?;
    let skip = zip_path.canonicalize().ok();

    let file = std::fs::File::create(zip_path)?;
    let mut zip = ZipWriter::new(file);
    // `options` borrows `password`; keeping it in this scope (not threaded through
    // recursion) avoids forcing a 'static password.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .with_aes_encryption(AesMode::Aes256, password);

    let mut summary = PackageSummary { files: 0, bytes: 0 };
    for path in files {
        if skip.is_some() && path.canonicalize().ok() == skip {
            continue; // never package the output archive into itself
        }
        let rel = path.strip_prefix(source).unwrap_or(&path);
        let name = rel.to_string_lossy().replace('\\', "/");
        zip.start_file(name, options).map_err(|e| io::Error::other(e.to_string()))?;
        let mut f = std::fs::File::open(&path)?;
        summary.bytes += io::copy(&mut f, &mut zip)?;
        summary.files += 1;
    }
    zip.finish().map_err(|e| io::Error::other(e.to_string()))?;
    Ok(summary)
}

/// Recursively collect every regular file under `dir` into `out`, sorted within
/// each directory for a reproducible archive order.
fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_files(&path, out)?;
        } else if ft.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn packages_files_into_encrypted_zip() {
        let dir = std::env::temp_dir().join(format!("be-pkg-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"world!!").unwrap();
        let zip_path = dir.join("out.zip");

        let summary = package_dir(&src, &zip_path, "s3cret").unwrap();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 5 + 7);
        assert!(zip_path.exists() && std::fs::metadata(&zip_path).unwrap().len() > 0);

        // The entries are encrypted: opening without the password fails to read,
        // with the right password the bytes come back.
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        // Wrong password is rejected.
        assert!(archive.by_name_decrypt("a.txt", b"wrong").is_err());
        let mut f = archive.by_name_decrypt("a.txt", b"s3cret").unwrap();
        let mut buf = String::new();
        f.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "hello");

        std::fs::remove_dir_all(&dir).ok();
    }
}
