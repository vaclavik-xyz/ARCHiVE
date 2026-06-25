//! Build the bundled `webkit2pdf` helper used by the Quartz PDF engine.
//!
//! On macOS the helper is compiled from `native/webkit2pdf/main.swift` for the
//! target architecture and embedded into the binary via `include_bytes!` (see
//! `exporters/pdf/webkit.rs`). On other platforms — or when `swiftc` is missing
//! — an empty placeholder is written so the embed still compiles and the Quartz
//! engine reports itself unavailable at runtime.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/webkit2pdf/main.swift");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let helper = out_dir.join("webkit2pdf");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "arm64".to_string());
        let arch = match target_arch.as_str() {
            "aarch64" => "arm64",
            other => other,
        };
        let target = format!("{arch}-apple-macosx12.0");
        match Command::new("swiftc")
            .args(["-O", "-target", &target, "-o"])
            .arg(&helper)
            .arg("native/webkit2pdf/main.swift")
            .status()
        {
            Ok(status) if status.success() => return,
            Ok(status) => {
                println!(
                    "cargo:warning=swiftc exited with {status}; the Quartz PDF engine will be unavailable"
                );
            }
            Err(why) => {
                println!(
                    "cargo:warning=could not run swiftc ({why}); the Quartz PDF engine will be unavailable"
                );
            }
        }
    }

    // Guarantee the embed target exists so `include_bytes!` always compiles.
    if !helper.exists() {
        fs::write(&helper, b"").expect("write placeholder helper");
    }
}
