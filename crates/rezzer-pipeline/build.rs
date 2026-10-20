// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sets `REZZER_BUILD_ID` for the cache key, so an entry written by an older development
//! build of the same version is never served after a reader fix.
//!
//! - Clean git checkout: the short commit hash, for example `1a2b3c4`
//! - Checkout with changes: `<hash>-dirty-<unix seconds of the build>`
//! - No git (a source archive): `t<unix seconds of the build>`
//!
//! The script reruns when the workspace sources or the git HEAD or index change.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let workspace = Path::new(&manifest).join("../..");
    if workspace.join("Cargo.toml").is_file() && workspace.join("crates").is_dir() {
        println!(
            "cargo:rerun-if-changed={}",
            workspace.join("crates").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            workspace.join("Cargo.lock").display()
        );
    }
    println!("cargo:rerun-if-changed=build.rs");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let id = match git(&manifest, &["rev-parse", "--short", "HEAD"]) {
        Some(hash) => {
            if let Some(dir) = git(&manifest, &["rev-parse", "--absolute-git-dir"]) {
                for file in ["HEAD", "index"] {
                    let path = Path::new(&dir).join(file);
                    if path.exists() {
                        println!("cargo:rerun-if-changed={}", path.display());
                    }
                }
            }
            let dirty = git(&manifest, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty());
            if dirty {
                format!("{hash}-dirty-{now}")
            } else {
                hash
            }
        }
        None => format!("t{now}"),
    };
    println!("cargo:rustc-env=REZZER_BUILD_ID={id}");
}

/// Output of a git command, trimmed; None when git is missing or fails.
fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
