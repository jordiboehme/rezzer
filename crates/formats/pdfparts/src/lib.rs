// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The object layer of a PDF file, read with a stream guard.
//!
//! A PDF library gives you objects. This crate reads what they mean: document metadata (Info
//! and XMP), language, pages with their labels, sizes and images, the outline, named
//! destinations, links, annotations, the structure tree, embedded files, layers and article
//! threads.
//!
//! [`Pdf::open`] parses the file with its own bounded parser, keeps only the objects the
//! document reaches, and writes them as canonical bytes ([`Pdf::canonical`]): a plain PDF with a
//! classic cross-reference table, no object streams and no encryption. Other parsers get only
//! those bytes. Every stream is measured once before open returns: a stream that unpacks past
//! the size or ratio caps is refused, and so is every page that uses it. An object stream bomb
//! ends the open with [`OpenError::Limit`]. A problem never panics. It becomes an [`Issue`],
//! and the other parts are still read.
//!
//! ```
//! use pdfparts::objects::{pdf_date, text_string};
//! assert_eq!(text_string(b"Is\xEDs"), "Is\u{ed}s");
//! assert_eq!(pdf_date("D:20151230152248+01'00'").as_deref(), Some("2015-12-30T15:22:48+01:00"));
//! ```

mod annots;
mod extras;
mod guard;
mod meta;
mod nav;
pub mod objects;
mod pagetree;
mod parse;
mod structure;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, OnceLock};

use lopdf::{Document, ObjectId};

pub use annots::Annot;
pub use extras::{EmbeddedFile, Layer, Thread};
pub use meta::{Info, PdfDate};
pub use nav::{ImageInfo, Link, LinkTarget, OutlineItem, PageInfo};
pub use structure::{StructElem, StructTree};

/// Bounds for reading one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Deepest nesting of a tree (page tree, outline, number and name trees, XML).
    pub max_depth: usize,
    /// Most items in one structure: one walk (tree entries, XML events, objects visited), one
    /// array or dictionary, or the members of one object stream.
    pub max_items: usize,
    /// Most bytes one stream may unpack to.
    pub max_stream_bytes: u64,
    /// Most bytes all streams together may unpack to.
    pub max_total_bytes: u64,
    /// Most times a stream may grow when it is unpacked.
    pub max_ratio: u64,
    /// Most revisions (incremental updates) that are read.
    pub max_revisions: usize,
    /// Most bytes the parsed objects and the canonical copy may take: every value counts a
    /// fixed 128 bytes plus its strings, names and stream data.
    pub max_object_bytes: u64,
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
            max_object_bytes: 1 << 30,
        }
    }
}

/// Why a part, or a piece of it, could not be read in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// The part is broken or points at something that is not there.
    Damaged,
    /// A limit was hit or the caller asked to stop.
    Limit,
    /// The part uses something that is not read, on purpose.
    Unsupported,
}

/// One thing that went wrong while reading, with the place it concerns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    canonical: Vec<u8>,
    original: Vec<u8>,
    /// Values parsed and the bytes charged for them and the canonical copy.
    values: usize,
    object_bytes: u64,
    limits: Limits,
    encryption: Option<Encryption>,
    /// Page objects in page order; page number = index + 1.
    pages: Vec<pagetree::PageNode>,
    page_numbers: HashMap<ObjectId, u32>,
    verdict: guard::Verdict,
    unsafe_tree: bool,
    issues: Mutex<IssueLog>,
    /// The structure tree, read on first use.
    structure: OnceLock<Option<structure::Built>>,
}

/// Issues in the order found, each once.
#[derive(Default)]
struct IssueLog {
    list: Vec<Issue>,
    seen: HashSet<Issue>,
}

impl IssueLog {
    fn add(&mut self, issue: Issue) {
        if self.seen.insert(issue.clone()) {
            self.list.push(issue);
        }
    }
}

impl fmt::Debug for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pdf")
            .field("pages", &self.pages.len())
            .field("encryption", &self.encryption)
            .field("refused_pages", &self.verdict.refused_pages)
            .field("unpacked", &self.verdict.unpacked)
            .field("values", &self.values)
            .field("object_bytes", &self.object_bytes)
            .finish_non_exhaustive()
    }
}

impl Pdf {
    /// Opens a PDF file from memory. `password` is tried as user and as owner password when the
    /// file does not open with an empty one. Every stream is measured before this returns.
    pub fn open(
        bytes: &[u8],
        password: Option<&str>,
        limits: &Limits,
        stop: &dyn Fn() -> bool,
    ) -> Result<Pdf, OpenError> {
        let head = &bytes[..bytes.len().min(parse::HEADER_WINDOW)];
        if !head.windows(5).any(|w| w == b"%PDF-") {
            return Err(OpenError::NotPdf);
        }
        if stop() {
            return Err(OpenError::Limit("stopped by the caller".into()));
        }
        let built = parse::build(bytes, password, limits, stop)?;
        let doc = built.doc;
        let mut issues = built.issues;
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
        let tree = pagetree::page_tree(&doc, limits, &mut issues);
        let pages = tree.pages;
        let verdict = guard::measure(&doc, &pages, built.unpacked, limits, stop, &mut issues)?;
        // A page listed twice keeps its first number.
        let mut page_numbers = HashMap::new();
        for (i, p) in pages.iter().enumerate() {
            page_numbers.entry(p.id).or_insert(i as u32 + 1);
        }
        let mut log = IssueLog::default();
        for issue in issues {
            log.add(issue);
        }
        Ok(Pdf {
            doc,
            canonical: built.canonical,
            original: bytes.to_vec(),
            values: built.items,
            object_bytes: built.charged,
            limits: *limits,
            encryption: built.encryption,
            pages,
            page_numbers,
            verdict,
            unsafe_tree: tree.unsafe_tree,
            issues: Mutex::new(log),
            structure: OnceLock::new(),
        })
    }

    /// How the file is encrypted, or `None` for a plain file.
    pub fn encryption(&self) -> Option<Encryption> {
        self.encryption.clone()
    }

    /// The document as canonical bytes: the objects it reaches, decrypted, with a classic
    /// cross-reference table and no object streams. Other parsers must read these bytes and
    /// never the original file.
    pub fn canonical(&self) -> &[u8] {
        &self.canonical
    }

    /// The bytes the file was opened from, for checks that need them as they are (signature
    /// byte ranges, revisions). They must not be handed to another parser.
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// Pages (1-based) whose streams broke a limit. They must not be handed to other parsers.
    pub fn refused_pages(&self) -> &[u32] {
        &self.verdict.refused_pages
    }

    /// The page tree has a cycle, a shared or repeated node, an untyped node or more depth
    /// than the limits allow. Page numbers here follow lopdf's page walk, which other parsers
    /// built on lopdf use too; a parser with a different walk may number such a file
    /// differently, so it must not get the file.
    pub fn unsafe_page_tree(&self) -> bool {
        self.unsafe_tree
    }

    /// Bytes all measured streams unpack to.
    pub fn unpacked(&self) -> u64 {
        self.verdict.unpacked
    }

    /// Every issue found so far: while opening, and by the readers called since.
    pub fn issues(&self) -> Vec<Issue> {
        self.issues
            .lock()
            .map(|i| i.list.clone())
            .unwrap_or_default()
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
        if let Ok(mut log) = self.issues.lock() {
            for issue in found {
                log.add(issue);
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
