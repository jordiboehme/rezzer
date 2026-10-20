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
//! Git is used only when the repository's top level is the Rezzer workspace itself, so a copy
//! of this crate inside another repository never takes that repository's state. Every git call
//! runs with `--no-optional-locks` and `core.fsmonitor=false`: a build never writes into
//! `.git` (plain `git status` refreshes and rewrites the index).
//!
//! The script reruns when the workspace sources, Cargo.lock, HEAD, the branch ref HEAD points
//! to, packed-refs or the index change.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let workspace = Path::new(&manifest).join("../..");
    let in_workspace = workspace.join("Cargo.toml").is_file() && workspace.join("crates").is_dir();
    if in_workspace {
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
    let id = in_workspace
        .then(|| git_id(&workspace, now))
        .flatten()
        .unwrap_or_else(|| format!("t{now}"));
    println!("cargo:rustc-env=REZZER_BUILD_ID={id}");
}

/// The id from git, when the repository's top level is `workspace`.
fn git_id(workspace: &Path, now: u64) -> Option<String> {
    let root = std::fs::canonicalize(workspace).ok()?;
    let top = git(&root, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(&top).ok()? != root {
        return None;
    }
    let hash = git(&root, &["rev-parse", "--short", "HEAD"])?;
    let watch = |p: &Path| {
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    };
    if let Some(dir) = git(&root, &["rev-parse", "--absolute-git-dir"]) {
        watch(&Path::new(&dir).join("HEAD"));
        watch(&Path::new(&dir).join("index"));
    }
    // On a branch HEAD only names the ref; a commit changes the ref file or packed-refs.
    if let (Some(common), Some(head_ref)) = (
        git(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ),
        git(&root, &["symbolic-ref", "-q", "HEAD"]),
    ) {
        watch(&Path::new(&common).join(&head_ref));
        watch(&Path::new(&common).join("packed-refs"));
    }
    let dirty = git(&root, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty());
    Some(if dirty {
        format!("{hash}-dirty-{now}")
    } else {
        hash
    })
}

/// Output of a git command, trimmed; None when git is missing or fails. Never takes optional
/// locks and never asks an fsmonitor, so it writes nothing into `.git`.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
