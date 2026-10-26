// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use rezzer_limits::Budget;
use rezzer_model::{
    Completeness, Confidence, Detection, Document, FormatId, ReadOptions, ReadOutcome, Reader,
    Registry, Source, SourceInfo,
};

struct Dummy {
    id: &'static str,
    formats: &'static [FormatId],
}

impl Reader for Dummy {
    fn id(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        self.formats
    }
    fn read(&self, src: &Source, _opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        ReadOutcome {
            blobs: Vec::new(),
            document: Document::new(SourceInfo {
                path: None,
                name: src.name.clone(),
                sha256: src.sha256.clone(),
                size: src.size,
                format: src.format,
                detection: Detection {
                    confidence: Confidence::Certain,
                    evidence: "test".into(),
                },
                modified: None,
                created: None,
                downloaded_from: None,
                file_tags: Vec::new(),
                encoding: None,
                reader: None,
            }),
            completeness: Completeness::Complete,
        }
    }
}

#[test]
fn registry_finds_reader_by_format() {
    let mut r = Registry::new();
    r.register(Box::new(Dummy {
        id: "dummy",
        formats: &[FormatId::Text],
    }));
    let found = r.for_format(FormatId::Text).expect("registered");
    assert_eq!(found.id(), "dummy");
    assert_eq!(r.list(), vec![(FormatId::Text, "dummy", "1.0.0")]);
}

#[test]
fn registry_returns_none_for_unregistered() {
    let mut r = Registry::new();
    assert!(r.for_format(FormatId::Pdf).is_none());
    r.register(Box::new(Dummy {
        id: "dummy",
        formats: &[FormatId::Text],
    }));
    assert!(r.for_format(FormatId::Pdf).is_none());
}

#[test]
fn later_registration_wins() {
    let mut r = Registry::new();
    r.register(Box::new(Dummy {
        id: "first",
        formats: &[FormatId::Text, FormatId::Csv],
    }));
    r.register(Box::new(Dummy {
        id: "second",
        formats: &[FormatId::Text],
    }));
    assert_eq!(r.for_format(FormatId::Text).unwrap().id(), "second");
    assert_eq!(r.for_format(FormatId::Csv).unwrap().id(), "first");
    let list = r.list();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&(FormatId::Text, "second", "1.0.0")));
    assert!(list.contains(&(FormatId::Csv, "first", "1.0.0")));
    assert!(!list.iter().any(|e| e.0 == FormatId::Text && e.1 == "first"));
}

#[test]
fn list_dedupes_repeated_formats() {
    let mut r = Registry::new();
    r.register(Box::new(Dummy {
        id: "dup",
        formats: &[FormatId::Text, FormatId::Text],
    }));
    assert_eq!(r.list(), vec![(FormatId::Text, "dup", "1.0.0")]);
}
