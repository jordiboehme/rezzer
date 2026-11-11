// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A tagged PDF without its tags: the same pages, with `/StructTreeRoot` and `/MarkInfo`
//! taken out of the catalog. Any build of the reader then reads the file by its layout alone,
//! and the facts its tags gave are the truth for that read. The file is written anew, so its
//! bytes differ; its pages, fonts and content streams are the same objects.

/// What [`strip_tags`] made of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stripped {
    /// The file without its tags.
    Done(Vec<u8>),
    /// The file has no tags to take out.
    NotTagged,
}

/// `bytes` without the tag tree. An encrypted file and a file that does not load are errors.
pub fn strip_tags(bytes: &[u8]) -> Result<Stripped, String> {
    let mut doc = lopdf::Document::load_mem(bytes).map_err(|e| format!("it does not load: {e}"))?;
    if doc.is_encrypted() {
        return Err("it is encrypted".into());
    }
    let catalog = doc
        .catalog_mut()
        .map_err(|e| format!("it has no catalog: {e}"))?;
    let tree = catalog.remove(b"StructTreeRoot").is_some();
    let marked = catalog.remove(b"MarkInfo").is_some();
    if !tree && !marked {
        return Ok(Stripped::NotTagged);
    }
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| format!("it is not written: {e}"))?;
    Ok(Stripped::Done(out))
}
