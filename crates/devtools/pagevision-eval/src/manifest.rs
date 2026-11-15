// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Manifests: TOML lists of test pages with their ground truth.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::EvalError;

/// A set of test pages. Paths are resolved against the manifest's folder on load.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    /// "public" or "private".
    pub set: String,
    #[serde(default)]
    pub pages: Vec<PageEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PageEntry {
    pub id: String,
    pub image: PathBuf,
    /// Ground truth text, one line per written line.
    pub text: Option<PathBuf>,
    /// Ground truth edges of the diagrams.
    pub edges: Option<PathBuf>,
    /// Node map (`ID<TAB>label`) for a truth whose edges use ids.
    #[serde(default)]
    pub nodes: Option<PathBuf>,
    /// Number of diagrams drawn on the page.
    #[serde(default)]
    pub diagrams: u32,
    #[serde(default)]
    pub known_fail: bool,
}

pub fn load_manifest(path: &Path) -> Result<Manifest, EvalError> {
    let raw = std::fs::read_to_string(path).map_err(|source| EvalError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut manifest: Manifest = toml::from_str(&raw)?;
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    for page in &mut manifest.pages {
        page.image = base.join(&page.image);
        page.text = page.text.take().map(|p| base.join(p));
        page.edges = page.edges.take().map(|p| base.join(p));
        page.nodes = page.nodes.take().map(|p| base.join(p));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_resolves_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("manifest.toml");
        std::fs::write(
            &file,
            r#"
set = "public"

[[pages]]
id = "test2"
image = "img/test2.png"
text = "truth/test2.txt"
diagrams = 2

[[pages]]
id = "blank"
image = "img/blank.png"
known_fail = true
"#,
        )
        .unwrap();
        let m = load_manifest(&file).unwrap();
        assert_eq!(m.set, "public");
        assert_eq!(m.pages.len(), 2);
        assert_eq!(m.pages[0].image, dir.path().join("img/test2.png"));
        assert_eq!(m.pages[0].text, Some(dir.path().join("truth/test2.txt")));
        assert_eq!(m.pages[0].edges, None);
        assert_eq!(m.pages[0].diagrams, 2);
        assert!(!m.pages[0].known_fail);
        assert!(m.pages[1].known_fail);
        assert_eq!(m.pages[1].diagrams, 0);
    }

    #[test]
    fn missing_manifest_is_an_io_error() {
        let err = load_manifest(Path::new("/nonexistent/manifest.toml")).unwrap_err();
        assert!(matches!(err, EvalError::Io { .. }));
    }
}
