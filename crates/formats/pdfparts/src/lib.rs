// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The object layer of a PDF file, read with a stream guard.
//!
//! A PDF library gives you objects. This crate reads what they mean: document metadata (Info
//! and XMP), language, pages with their labels, sizes and images, the outline, named
//! destinations and links.
//!
//! [`Pdf::open`] measures every stream once before anything else reads the file: a stream that
//! unpacks past the size or ratio caps is refused, and so is every page that uses it. An
//! object stream bomb ends the open with [`OpenError::Limit`]. A problem never panics. It
//! becomes an [`Issue`], and the other parts are still read.
//!
//! ```
//! use pdfparts::objects::{pdf_date, text_string};
//! assert_eq!(text_string(b"Is\xEDs"), "Is\u{ed}s");
//! assert_eq!(pdf_date("D:20151230152248+01'00'").as_deref(), Some("2015-12-30T15:22:48+01:00"));
//! ```

mod guard;
mod meta;
mod nav;
pub mod objects;

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use lopdf::{Document, LoadOptions, Object, ObjectId};

pub use meta::{Info, PdfDate};
pub use nav::{ImageInfo, Link, LinkTarget, OutlineItem, PageInfo};

/// Bounds for reading one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Deepest nesting of a tree (page tree, outline, number and name trees, XML).
    pub max_depth: usize,
    /// Most items in one walk (tree entries, XML events, objects visited).
    pub max_items: usize,
    /// Most bytes one stream may unpack to.
    pub max_stream_bytes: u64,
    /// Most bytes all streams together may unpack to.
    pub max_total_bytes: u64,
    /// Most times a stream may grow when it is unpacked.
    pub max_ratio: u64,
    /// Most revisions (incremental updates) that are read.
    pub max_revisions: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 256,
            max_items: 2_000_000,
            max_stream_bytes: 1 << 30,
            max_total_bytes: 4 << 30,
            max_ratio: 100,
            max_revisions: 64,
        }
    }
}

/// Why a part, or a piece of it, could not be read in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// The part is broken or points at something that is not there.
    Damaged,
    /// A limit was hit or the caller asked to stop.
    Limit,
    /// The part uses something that is not read, on purpose.
    Unsupported,
}

/// One thing that went wrong while reading, with the place it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Where: `document`, `page 3`, `field Total`, `object 12 0`, `outline`, `metadata`.
    pub location: String,
    /// What happened, in plain words.
    pub what: String,
    /// The class of the problem.
    pub kind: IssueKind,
}

impl Issue {
    pub(crate) fn new(location: &str, kind: IssueKind, what: impl Into<String>) -> Issue {
        Issue {
            location: location.to_string(),
            what: what.into(),
            kind,
        }
    }
}

/// Why a file could not be opened. The text never contains the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The bytes are not a PDF file.
    NotPdf,
    /// The file is too broken to read.
    Damaged(String),
    /// The file is encrypted and does not open without a password.
    NeedsPassword,
    /// The given password opens neither as user nor as owner password.
    WrongPassword,
    /// The file uses encryption that is not supported.
    UnsupportedEncryption(String),
    /// A limit was hit or the caller asked to stop.
    Limit(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NotPdf => write!(f, "not a PDF file"),
            OpenError::Damaged(why) => write!(f, "the file is damaged: {why}"),
            OpenError::NeedsPassword => write!(f, "the file needs a password"),
            OpenError::WrongPassword => write!(f, "the password is wrong"),
            OpenError::UnsupportedEncryption(why) => {
                write!(f, "the encryption is not supported: {why}")
            }
            OpenError::Limit(why) => write!(f, "a limit was hit: {why}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// How a file is encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encryption {
    /// `AES-128`, `AES-256`, `RC4-40`, `RC4-128` and so on.
    pub method: String,
    /// The file opens without a password. The empty string may match the user or the owner
    /// password; the file does not say which.
    pub empty_user_password: bool,
}

/// An open PDF file.
pub struct Pdf {
    doc: Document,
    limits: Limits,
    encryption: Option<Encryption>,
    /// Page objects in page order; page number = index + 1.
    pages: Vec<nav::PageNode>,
    page_numbers: HashMap<ObjectId, u32>,
    verdict: guard::Verdict,
    issues: Mutex<Vec<Issue>>,
}

impl fmt::Debug for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pdf")
            .field("pages", &self.pages.len())
            .field("encryption", &self.encryption)
            .field("refused_pages", &self.verdict.refused_pages)
            .field("unpacked", &self.verdict.unpacked)
            .finish_non_exhaustive()
    }
}

/// How far into the file the `%PDF-` header may start.
const HEADER_WINDOW: usize = 1024;

impl Pdf {
    /// Opens a PDF file from memory. `password` is tried as user and as owner password when the
    /// file does not open with an empty one. Every stream is measured before this returns.
    pub fn open(
        bytes: &[u8],
        password: Option<&str>,
        limits: &Limits,
        stop: &dyn Fn() -> bool,
    ) -> Result<Pdf, OpenError> {
        let head = &bytes[..bytes.len().min(HEADER_WINDOW)];
        if !head.windows(5).any(|w| w == b"%PDF-") {
            return Err(OpenError::NotPdf);
        }
        if stop() {
            return Err(OpenError::Limit("stopped by the caller".into()));
        }
        let pre = guard::prescan(bytes, limits, stop)?;
        // lopdf inflates object and cross-reference streams while it loads. The scan above
        // has measured those it could read; this cap bounds the rest (the object streams of
        // an encrypted file) to the ratio limit of the largest such stream.
        let largest = if pre.largest_raw > 0 {
            pre.largest_raw
        } else {
            bytes.len() as u64
        };
        let cap = limits
            .max_stream_bytes
            .min(limits.max_ratio.saturating_mul(largest));
        let cap = usize::try_from(cap).unwrap_or(usize::MAX);
        let options = |password: Option<&str>| LoadOptions {
            password: password.map(str::to_string),
            max_decompressed_size: Some(cap),
            ..LoadOptions::default()
        };

        let mut doc = load(bytes, options(None), password)?;
        let mut empty_user_password = true;
        if doc.trailer.get(b"Encrypt").is_ok() {
            // lopdf could not open it with an empty password.
            check_handler(&doc)?;
            let Some(pw) = password else {
                return Err(OpenError::NeedsPassword);
            };
            doc = load(bytes, options(Some(pw)), password)?;
            if doc.trailer.get(b"Encrypt").is_ok() {
                return Err(OpenError::WrongPassword);
            }
            empty_user_password = false;
        }
        let encryption = doc.encryption_state.as_ref().map(|state| Encryption {
            method: method(state),
            empty_user_password,
        });
        if stop() {
            return Err(OpenError::Limit("stopped by the caller".into()));
        }

        let mut issues = Vec::new();
        let catalog_ok = doc
            .trailer
            .get(b"Root")
            .ok()
            .and_then(|r| objects::dict(&doc, r))
            .is_some();
        if !catalog_ok {
            return Err(OpenError::Damaged(
                "the file has no document catalog".into(),
            ));
        }
        missing_members(&doc, &mut issues);
        let pages = nav::page_tree(&doc, limits, &mut issues);
        let ids: Vec<ObjectId> = pages.iter().map(|p| p.id).collect();
        let roots: Vec<(ObjectId, Option<&Object>)> =
            pages.iter().map(|p| (p.id, p.resources.as_ref())).collect();
        let verdict = guard::measure(&doc, &roots, &pre, limits, stop, &mut issues)?;
        let page_numbers = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i as u32 + 1))
            .collect();
        Ok(Pdf {
            doc,
            limits: *limits,
            encryption,
            pages,
            page_numbers,
            verdict,
            issues: Mutex::new(issues),
        })
    }

    /// How the file is encrypted, or `None` for a plain file.
    pub fn encryption(&self) -> Option<Encryption> {
        self.encryption.clone()
    }

    /// For an encrypted file, the same document saved without encryption, so other parsers can
    /// read it. `None` for a plain file.
    pub fn decrypted(&self) -> Option<Vec<u8>> {
        self.encryption.as_ref()?;
        let mut doc = self.doc.clone();
        doc.encryption_state = None;
        doc.trailer.remove(b"Encrypt");
        // The members of object streams are loaded as plain objects; the containers and old
        // cross-reference streams are not needed in the copy.
        doc.objects.retain(|_, o| match o {
            Object::Stream(s) => !(s.dict.has_type(b"ObjStm") || s.dict.has_type(b"XRef")),
            _ => true,
        });
        let mut out = Vec::new();
        doc.save_to(&mut out).ok()?;
        Some(out)
    }

    /// Pages (1-based) whose streams broke a limit. They must not be handed to other parsers.
    pub fn refused_pages(&self) -> &[u32] {
        &self.verdict.refused_pages
    }

    /// Bytes all measured streams unpack to.
    pub fn unpacked(&self) -> u64 {
        self.verdict.unpacked
    }

    /// Every issue found so far: while opening, and by the readers called since.
    pub fn issues(&self) -> Vec<Issue> {
        self.issues.lock().map(|i| i.clone()).unwrap_or_default()
    }

    /// Number of pages found in the page tree.
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    /// Records issues found by a reader, each only once.
    pub(crate) fn note(&self, found: Vec<Issue>) {
        if found.is_empty() {
            return;
        }
        if let Ok(mut all) = self.issues.lock() {
            for issue in found {
                if !all.contains(&issue) {
                    all.push(issue);
                }
            }
        }
    }

    pub(crate) fn catalog(&self) -> Option<&lopdf::Dictionary> {
        objects::dict(&self.doc, self.doc.trailer.get(b"Root").ok()?)
    }

    /// The unpacked bytes of a stream, `None` when the guard refused it or it does not decode.
    pub(crate) fn plain(&self, id: ObjectId) -> Option<Vec<u8>> {
        if self.verdict.refused_objects.contains(&id) {
            return None;
        }
        let stream = self.doc.objects.get(&id)?.as_stream().ok()?;
        let cap = usize::try_from(self.limits.max_stream_bytes).unwrap_or(usize::MAX);
        stream.get_plain_content_with_limit(cap).ok()
    }

    /// The page number of a page object.
    pub(crate) fn page_number(&self, id: ObjectId) -> Option<u32> {
        self.page_numbers.get(&id).copied()
    }
}

/// Loads with lopdf and maps its errors. The password never reaches an error text.
fn load(bytes: &[u8], options: LoadOptions, password: Option<&str>) -> Result<Document, OpenError> {
    Document::load_mem_with_options(bytes, options).map_err(|e| {
        use lopdf::Error;
        match e {
            Error::InvalidPassword => OpenError::WrongPassword,
            Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { limit }) => {
                OpenError::Limit(format!("a stream unpacks to more than {limit} bytes"))
            }
            Error::UnsupportedSecurityHandler(name) => OpenError::UnsupportedEncryption(format!(
                "security handler {}",
                String::from_utf8_lossy(&name)
            )),
            Error::Decryption(d) => {
                OpenError::UnsupportedEncryption(redact(d.to_string(), password))
            }
            Error::Parse(lopdf::ParseError::InvalidFileHeader) => OpenError::NotPdf,
            other => OpenError::Damaged(redact(other.to_string(), password)),
        }
    })
}

/// lopdf drops an object stream it cannot read without saying so. The cross-reference table
/// still lists its members, so the loss is reported here.
fn missing_members(doc: &Document, issues: &mut Vec<Issue>) {
    let mut lost: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for (&n, entry) in &doc.reference_table.entries {
        if let lopdf::xref::XrefEntry::Compressed { container, .. } = entry
            && !doc.objects.contains_key(&(n, 0))
        {
            *lost.entry(*container).or_default() += 1;
        }
    }
    for (container, count) in lost {
        issues.push(Issue::new(
            &format!("object {container} 0"),
            IssueKind::Damaged,
            format!("the object stream could not be read: {count} objects in it are missing"),
        ));
    }
}

fn redact(text: String, password: Option<&str>) -> String {
    match password {
        Some(p) if !p.is_empty() && text.contains(p) => text.replace(p, "***"),
        _ => text,
    }
}

/// Refuses encryption this crate cannot open, before asking for a password.
fn check_handler(doc: &Document) -> Result<(), OpenError> {
    let Some(dict) = doc
        .trailer
        .get(b"Encrypt")
        .ok()
        .and_then(|e| objects::dict(doc, e))
    else {
        return Err(OpenError::Damaged(
            "the encryption dictionary is missing".into(),
        ));
    };
    let filter = objects::get_name(doc, dict, b"Filter").unwrap_or(b"");
    if filter != b"Standard" {
        return Err(OpenError::UnsupportedEncryption(format!(
            "security handler {}",
            String::from_utf8_lossy(filter)
        )));
    }
    let v = objects::get_int(doc, dict, b"V").unwrap_or(0);
    let r = objects::get_int(doc, dict, b"R").unwrap_or(0);
    if !matches!(v, 1 | 2 | 4 | 5) || !(2..=6).contains(&r) {
        return Err(OpenError::UnsupportedEncryption(format!(
            "standard security handler version {v} revision {r}"
        )));
    }
    Ok(())
}

fn method(state: &lopdf::EncryptionState) -> String {
    let bits = state.key_length().unwrap_or(40);
    match state.version() {
        1 => "RC4-40".to_string(),
        2 | 3 => format!("RC4-{bits}"),
        4 | 5 => {
            let name = state.default_stream_filter();
            let cfm = state
                .crypt_filters()
                .get(name)
                .map(|f| f.method().to_vec())
                .unwrap_or_else(|| name.to_vec());
            match cfm.as_slice() {
                b"AESV2" => "AES-128".to_string(),
                b"AESV3" => "AES-256".to_string(),
                b"V2" => format!("RC4-{}", if bits > 40 { bits } else { 128 }),
                b"Identity" => "none".to_string(),
                other => String::from_utf8_lossy(other).into_owned(),
            }
        }
        v => format!("unknown (version {v})"),
    }
}
