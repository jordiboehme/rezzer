// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes and QR codes through the binary: a generated invoice with a QR code picture and
//! a Code 128 drawn as bars, read with and without an output directory.
#![cfg(feature = "fmt-pdf")]

#[path = "../../readers/rezzer-read-pdf/tests/common/mod.rs"]
mod pdfkit;

use std::io::Write;

use assert_cmd::cargo::cargo_bin_cmd;
use barcodes::Symbology;
use barcodes::encode::{self, Options};
use rezzer_model::Document;

const URL: &str = "https://example.org/t/dNuKThV1";

fn invoice() -> Vec<u8> {
    let qr = encode::text(Symbology::QrCode, URL, &Options::default()).expect("encode");
    let (gray, w, h) = qr.gray(4, 4, 0);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&gray).expect("deflate");
    let packed = z.finish().expect("deflate");
    let bar = encode::text(Symbology::Code128, "RZ-2026-114", &Options::default()).expect("encode");
    let mut content = String::from(
        "BT /F1 16 Tf 72 780 Td (Invoice 2026-114) Tj ET\n\
         BT /F1 11 Tf 72 740 Td (Two chairs, 240.00 EUR) Tj ET\nq 0 g\n",
    );
    for col in 0..bar.width {
        if bar.dark[col as usize] {
            content += &format!("{:.2} 600 0.8 40 re\n", 72.0 + col as f32 * 0.8);
        }
    }
    content += "f Q\nq 90 0 0 90 480 40 cm /P0 Do Q\n";
    let mut b = pdfkit::Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray \
             /BitsPerComponent 8 /Filter /FlateDecode"
        ),
        &packed,
    );
    let cs = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> /XObject << /P0 {image} 0 R >> >> \
         /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

fn codes(doc: &Document) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = doc
        .assets
        .iter()
        .flat_map(|a| {
            a.codes.iter().map(|c| {
                (
                    a.id.clone(),
                    c.symbology.clone(),
                    c.text.clone().unwrap_or_default(),
                )
            })
        })
        .collect();
    out.sort();
    out
}

#[test]
fn codes_are_read_with_and_without_an_output_directory() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("invoice.pdf");
    std::fs::write(&file, invoice()).expect("write");
    let want = vec![
        (
            "page-1-code-1".to_string(),
            "Code 128".to_string(),
            "RZ-2026-114".to_string(),
        ),
        (
            "page-1-pic-1".to_string(),
            "QR Code".to_string(),
            URL.to_string(),
        ),
    ];
    // Without an output directory, as an MCP read: the JSON on stdout.
    let out = cargo_bin_cmd!("rezzer")
        .args(["read", "--json", "--no-cache"])
        .arg(&file)
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Document = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(codes(&doc), want);
    let facts = doc.overview.expect("overview").facts;
    assert!(
        facts.contains(&("Codes".to_string(), "2 (Code 128 1, QR Code 1)".to_string())),
        "{facts:?}"
    );
    assert!(doc.assets.iter().all(|a| !a.needs_vision));
    // With one: the crops are written, and the Markdown links them with the codes below.
    let o = dir.path().join("out");
    let run = cargo_bin_cmd!("rezzer")
        .args(["read", "--no-cache", "-o"])
        .arg(&o)
        .arg(&file)
        .output()
        .expect("run");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let md = std::fs::read_to_string(o.join("invoice.pdf.md")).expect("markdown");
    assert!(
        md.contains(
            "![QR Code, box 480, 40 to 570, 130](invoice.pdf.assets/page-1-pic-1.png)\n\n\
             QR Code: [https://example.org/t/dNuKThV1](https://example.org/t/dNuKThV1)"
        ),
        "{md}"
    );
    assert!(md.contains("Code 128: RZ-2026-114"), "{md}");
    assert!(o.join("invoice.pdf.assets/page-1-code-1.png").is_file());
    assert!(!md.contains("Needs vision"), "{md}");
}

/// `n` A4 pages, each a text-like scan at 300 dpi of its own.
fn scans(n: usize) -> Vec<u8> {
    let (w, h) = (2481u32, 3507u32);
    let mut b = pdfkit::Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let mut kids = String::new();
    for p in 0..n {
        let mut state = 7u32 + p as u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let white = vec![245u8; w as usize];
        let mut ink = white.clone();
        for y in 0..h {
            if y % 50 == 0 {
                for (x, v) in ink.iter_mut().enumerate() {
                    let glyph = x > 200 && x < w as usize - 200 && x % 5 < 3;
                    *v = if glyph && next() % 3 != 0 { 30 } else { 245 };
                }
            }
            let line = y % 50 < 28 && y > 200 && y < h - 200;
            z.write_all(if line { &ink } else { &white })
                .expect("deflate");
        }
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray \
                 /BitsPerComponent 8 /Filter /FlateDecode"
            ),
            &z.finish().expect("deflate"),
        );
        let cs = b.stream("", b"q 595 0 0 842 0 0 cm /Im1 Do Q\n");
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// Runs the binary with `args` under a config with a memory limit of 256 MiB, and returns
/// its exit status and the most memory it held.
// The child is reaped with wait4, which gives its resource use; `Child::wait` does not.
#[allow(clippy::zombie_processes)]
fn run_at_256_mib(dir: &std::path::Path, args: &[&std::ffi::OsStr]) -> (i32, u64) {
    let config = dir.join("config/rezzer");
    std::fs::create_dir_all(&config).expect("config dir");
    std::fs::write(
        config.join("config.toml"),
        "cache = false\n\n[limits]\nmax_memory_bytes = 268435456\n",
    )
    .expect("config");
    let child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("rezzer"))
        .args(args)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("HOME", dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let mut status = 0;
    // SAFETY: wait4 fills `status` and `usage` for the child we started.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let pid = child.id() as libc::pid_t;
    unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    let rss = usage.ru_maxrss as u64;
    let rss = if cfg!(target_os = "macos") {
        rss
    } else {
        rss * 1024
    };
    (status, rss)
}

#[test]
fn sixty_scanned_pages_stay_under_the_memory_limit_in_the_binary() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("scans.pdf");
    std::fs::write(&file, scans(60)).expect("write");
    let out = dir.path().join("out");
    for with_files in [false, true] {
        let mut args: Vec<&std::ffi::OsStr> = vec!["read".as_ref(), "--no-cache".as_ref()];
        if with_files {
            args.push("-o".as_ref());
            args.push(out.as_os_str());
        }
        args.push(file.as_os_str());
        let (status, rss) = run_at_256_mib(dir.path(), &args);
        eprintln!(
            "60 scanned pages, files {with_files}: status {status}, max RSS {} MiB",
            rss >> 20
        );
        assert_eq!(status, 0);
        assert!(
            rss < 256 << 20,
            "files {with_files}: max RSS {} MiB",
            rss >> 20
        );
    }
}
