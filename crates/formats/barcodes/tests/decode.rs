// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Codes are written in the test with rxing's own writer and drawn into pictures.

use std::time::{Duration, Instant};

use barcodes::encode::{self, Modules, Options};
use barcodes::{Limits, Payload, Picture, Pixels, Scan, Stop, Symbology, Worker, decode};

fn modules(s: Symbology, text: &str, ec: Option<&str>) -> Modules {
    encode::text(
        s,
        text,
        &Options {
            error_correction: ec.map(str::to_string),
        },
    )
    .expect("encode")
}

/// The default limits with 60 s a picture, a safety stop with room for any machine (the
/// slowest scan here took 8.25 s under load): what a scan finds must not depend on the
/// machine's speed or load (the default 10 s ran out under load). Work bounds are checked by
/// counted decoder calls ([`barcodes::work_steps`]), not by time.
fn unhurried() -> Limits {
    Limits {
        time: Duration::from_secs(60),
        ..Limits::default()
    }
}

fn scan_gray(gray: &[u8], w: u32, h: u32) -> Scan {
    decode(
        &Picture {
            pixels: Pixels::Gray(gray),
            width: w,
            height: h,
        },
        &unhurried(),
    )
    .expect("scan")
}

fn scan_rgba(rgba: &[u8], w: u32, h: u32) -> Scan {
    decode(
        &Picture {
            pixels: Pixels::Rgba(rgba),
            width: w,
            height: h,
        },
        &unhurried(),
    )
    .expect("scan")
}

fn texts(scan: &Scan) -> Vec<(Symbology, String)> {
    let mut out: Vec<(Symbology, String)> = scan
        .codes
        .iter()
        .map(|c| match &c.payload {
            Payload::Text(t) => (c.symbology, t.clone()),
            other => (c.symbology, format!("{other:?}")),
        })
        .collect();
    out.sort();
    out
}

/// Gray pixels as RGBA, dark pixels in `ink`, light ones in `paper` (with alpha).
fn colored(gray: &[u8], ink: [u8; 4], paper: [u8; 4]) -> Vec<u8> {
    gray.iter()
        .flat_map(|&v| if v < 128 { ink } else { paper })
        .collect()
}

const URL: &str = "https://example.org/ticket/A1B2C3";

#[test]
fn qr_with_a_colored_logo_over_15_percent_is_read_at_q_and_h() {
    for ec in ["Q", "H"] {
        let m = modules(Symbology::QrCode, URL, Some(ec));
        let (gray, w, h) = m.gray(4, 4, 0);
        let mut rgba = colored(&gray, [0, 0, 0, 255], [255, 255, 255, 255]);
        // A red logo over 15 % of the code's area, in the middle.
        let code = (m.width * 4) as f64;
        let side = (code * code * 0.15).sqrt() as u32;
        let (x0, y0) = ((w - side) / 2, (h - side) / 2);
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                let at = ((y * w + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[220, 40, 40, 255]);
            }
        }
        let scan = scan_rgba(&rgba, w, h);
        assert_eq!(
            texts(&scan),
            vec![(Symbology::QrCode, URL.to_string())],
            "level {ec}"
        );
    }
}

#[test]
fn colored_on_colored_is_read() {
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    // Dark blue on yellow, and red on green: luma alone splits the second one badly.
    for (ink, paper) in [
        ([20, 30, 120, 255], [250, 220, 60, 255]),
        ([200, 30, 30, 255], [40, 160, 60, 255]),
    ] {
        let scan = scan_rgba(&colored(&gray, ink, paper), w, h);
        assert_eq!(texts(&scan), vec![(Symbology::QrCode, URL.to_string())]);
    }
}

#[test]
fn inverted_code_is_read() {
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    let inverted: Vec<u8> = gray.iter().map(|v| 255 - v).collect();
    assert_eq!(
        texts(&scan_gray(&inverted, w, h)),
        vec![(Symbology::QrCode, URL.to_string())]
    );
}

#[test]
fn code_on_a_transparent_background_is_read() {
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    // Black modules on fully transparent black: read as on white.
    let rgba = colored(&gray, [0, 0, 0, 255], [0, 0, 0, 0]);
    assert_eq!(
        texts(&scan_rgba(&rgba, w, h)),
        vec![(Symbology::QrCode, URL.to_string())]
    );
}

#[test]
fn code_turned_a_quarter_is_read() {
    let m = modules(Symbology::DataMatrix, "ROT-90 4711", None);
    let (gray, w, h) = m.gray(4, 4, 0);
    let mut turned = vec![0u8; gray.len()];
    for y in 0..h {
        for x in 0..w {
            // (x, y) goes to (h - 1 - y, x) in a picture h wide.
            turned[(x * h + (h - 1 - y)) as usize] = gray[(y * w + x) as usize];
        }
    }
    assert_eq!(
        texts(&scan_gray(&turned, h, w)),
        vec![(Symbology::DataMatrix, "ROT-90 4711".to_string())]
    );
    let q = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = q.gray(4, 4, 0);
    let mut turned = vec![0u8; gray.len()];
    for y in 0..h {
        for x in 0..w {
            turned[(x * h + (h - 1 - y)) as usize] = gray[(y * w + x) as usize];
        }
    }
    assert_eq!(
        texts(&scan_gray(&turned, h, w)),
        vec![(Symbology::QrCode, URL.to_string())]
    );
}

#[test]
fn two_pixel_modules_are_read() {
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(2, 4, 0);
    assert_eq!(
        texts(&scan_gray(&gray, w, h)),
        vec![(Symbology::QrCode, URL.to_string())]
    );
}

#[test]
fn data_matrix_is_read() {
    let m = modules(Symbology::DataMatrix, "DM 0123456789 Rezzer", None);
    let (gray, w, h) = m.gray(3, 2, 0);
    assert_eq!(
        texts(&scan_gray(&gray, w, h)),
        vec![(Symbology::DataMatrix, "DM 0123456789 Rezzer".to_string())]
    );
}

/// An IATA boarding pass payload (Bar Coded Boarding Pass, one leg), made up.
const BCBP: &str = "M1DOE/JOHN            EABC123 FRAJFKLH 0400 290Y012A0012 100";

#[test]
fn pdf417_boarding_pass_is_read() {
    let m = modules(Symbology::Pdf417, BCBP, Some("2"));
    let (gray, w, h) = m.gray(2, 2, 0);
    assert_eq!(
        texts(&scan_gray(&gray, w, h)),
        vec![(Symbology::Pdf417, BCBP.to_string())]
    );
}

#[test]
fn code_128_is_read() {
    let m = modules(Symbology::Code128, "RZ-2026-1021", None);
    let (gray, w, h) = m.gray(3, 10, 40);
    assert_eq!(
        texts(&scan_gray(&gray, w, h)),
        vec![(Symbology::Code128, "RZ-2026-1021".to_string())]
    );
}

#[test]
fn binary_payload_comes_back_as_its_bytes() {
    let raw: Vec<u8> = (0u8..=255)
        .rev()
        .step_by(3)
        .chain([0, 0x1b, 0x7f])
        .collect();
    let m = encode::bytes(Symbology::QrCode, &raw, &Options::default()).expect("encode");
    let (gray, w, h) = m.gray(3, 4, 0);
    let scan = scan_gray(&gray, w, h);
    assert_eq!(scan.codes.len(), 1, "{scan:?}");
    assert_eq!(scan.codes[0].payload, Payload::Bytes(raw));
}

/// A text-like page: lines of made-up 5 by 7 glyphs at 2 pixels per dot, as an A4 page at
/// 150 dpi.
fn text_page() -> (Vec<u8>, u32, u32) {
    let (w, h) = (1240u32, 1754u32);
    let mut out = vec![255u8; (w * h) as usize];
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut y = 100;
    while y + 14 < h - 100 {
        let mut x = 100;
        while x + 10 < w - 100 {
            let glyph = next();
            if glyph % 7 == 0 {
                // A space.
                x += 12;
                continue;
            }
            for dy in 0..7 {
                let bits = next();
                for dx in 0..5 {
                    if bits >> dx & 1 == 1 {
                        for py in 0..2 {
                            for px in 0..2 {
                                let at = (y + dy * 2 + py) * w + x + dx * 2 + px;
                                out[at as usize] = 0;
                            }
                        }
                    }
                }
            }
            x += 12;
        }
        y += 24;
    }
    (out, w, h)
}

#[test]
fn a_text_page_gives_no_code() {
    let (gray, w, h) = text_page();
    let scan = scan_gray(&gray, w, h);
    assert!(scan.codes.is_empty(), "{scan:?}");
}

/// A picture of 11 by 28 pixels that rxing's MaxiCode reader loops on for minutes (found by
/// mutation fuzzing in the decoder spike): the same row of stripes 28 times.
fn maxicode_trap() -> Vec<u8> {
    let row = [255u8, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0];
    row.iter().copied().cycle().take(11 * 28).collect()
}

#[test]
fn maxicode_is_off_so_the_trap_returns_at_once() {
    let gray = maxicode_trap();
    let start = Instant::now();
    let scan = scan_gray(&gray, 11, 28);
    assert!(scan.codes.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn four_codes_on_one_page_are_all_found() {
    let (w, h) = (1400u32, 1000u32);
    let mut page = vec![255u8; (w * h) as usize];
    let mut put = |gray: &[u8], gw: u32, gh: u32, x: u32, y: u32| {
        for row in 0..gh {
            let to = ((y + row) * w + x) as usize;
            let from = (row * gw) as usize;
            page[to..to + gw as usize].copy_from_slice(&gray[from..from + gw as usize]);
        }
    };
    let qr = modules(Symbology::QrCode, URL, None).gray(4, 4, 0);
    put(&qr.0, qr.1, qr.2, 60, 60);
    let dm = modules(Symbology::DataMatrix, "DM-PAGE-2", None).gray(5, 2, 0);
    put(&dm.0, dm.1, dm.2, 800, 80);
    let az = modules(Symbology::Aztec, "AZTEC TICKET 77", None).gray(4, 2, 0);
    put(&az.0, az.1, az.2, 120, 600);
    let c128 = modules(Symbology::Code128, "C128-0042", None).gray(3, 10, 40);
    put(&c128.0, c128.1, c128.2, 700, 650);
    let scan = scan_gray(&page, w, h);
    assert_eq!(
        texts(&scan),
        vec![
            (Symbology::QrCode, URL.to_string()),
            (Symbology::DataMatrix, "DM-PAGE-2".to_string()),
            (Symbology::Aztec, "AZTEC TICKET 77".to_string()),
            (Symbology::Code128, "C128-0042".to_string()),
        ]
    );
}

#[test]
fn a_picture_over_the_pixel_cap_is_refused_before_any_decode() {
    let start = Instant::now();
    let got = decode(
        &Picture {
            pixels: Pixels::Gray(&[]),
            width: 20_000,
            height: 20_000,
        },
        &Limits::default(),
    );
    assert_eq!(
        got,
        Err(Stop::TooLarge {
            pixels: 400_000_000,
            max: 100_000_000
        })
    );
    assert!(start.elapsed() < Duration::from_millis(100));
}

/// Many finder-like squares: rxing tries each triple, which takes seconds.
fn finders(n: u32) -> (Vec<u8>, u32, u32) {
    let per = (f64::from(n).sqrt().ceil()) as u32;
    let side = per * 36;
    let mut out = vec![255u8; (side * side) as usize];
    for k in 0..n {
        let (cx, cy) = ((k % per) * 36 + 4, (k / per) * 36 + 4);
        for y in 0..28 {
            for x in 0..28 {
                let ring = x.min(y).min(27 - x).min(27 - y);
                // A dark ring of one module, a light one, a dark centre of three.
                if !(4..8).contains(&ring) {
                    out[((cy + y) * side + cx + x) as usize] = 0;
                }
            }
        }
    }
    (out, side, side)
}

#[test]
fn the_worker_stops_waiting_at_the_time_limit() {
    let (gray, w, h) = finders(1_000);
    let limits = Limits {
        time: Duration::from_millis(500),
        ..Limits::default()
    };
    let start = Instant::now();
    let got = Worker::global().decode(
        &Picture {
            pixels: Pixels::Gray(&gray),
            width: w,
            height: h,
        },
        &limits,
    );
    let took = start.elapsed();
    assert!(matches!(got, Err(Stop::Time(_))), "{got:?} after {took:?}");
    assert!(took < Duration::from_secs(2), "{took:?}");
    // The abandoned call holds its own thread only: a new thread reads the next small code.
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    let mut read = None;
    for _ in 0..60 {
        if let Ok(scan) = Worker::global().decode(
            &Picture {
                pixels: Pixels::Gray(&gray),
                width: w,
                height: h,
            },
            &Limits {
                time: Duration::from_secs(1),
                ..Limits::default()
            },
        ) {
            read = Some(scan);
            break;
        }
    }
    assert_eq!(
        read.map(|s| texts(&s)),
        Some(vec![(Symbology::QrCode, URL.to_string())])
    );
}

#[test]
fn the_code_cap_keeps_the_first_codes() {
    let m = modules(Symbology::QrCode, URL, None);
    let (one, ow, oh) = m.gray(3, 4, 0);
    // Six different codes in a row.
    let w = ow * 6;
    let mut page = vec![255u8; (w * oh) as usize];
    for k in 0..6u32 {
        let m = modules(Symbology::QrCode, &format!("{URL}/{k}"), None);
        let (g, gw, gh) = m.gray(3, 4, 0);
        assert_eq!((gw, gh), (ow, oh));
        for row in 0..gh {
            let to = (row * w + k * ow) as usize;
            page[to..to + gw as usize]
                .copy_from_slice(&g[(row * gw) as usize..((row + 1) * gw) as usize]);
        }
    }
    let _ = one;
    let scan = decode(
        &Picture {
            pixels: Pixels::Gray(&page),
            width: w,
            height: oh,
        },
        &Limits {
            max_codes: 4,
            ..Limits::default()
        },
    )
    .expect("scan");
    assert_eq!(scan.codes.len(), 4);
    assert!(scan.capped);
}

#[test]
fn aztec_is_read() {
    for px in [3, 4] {
        let m = modules(Symbology::Aztec, "AZTEC TICKET 77", None);
        let (gray, w, h) = m.gray(px, 2, 0);
        assert_eq!(
            texts(&scan_gray(&gray, w, h)),
            vec![(Symbology::Aztec, "AZTEC TICKET 77".to_string())],
            "{px} px"
        );
    }
}

/// rxing looks for an Aztec finder near the middle of a picture only.
#[test]
fn aztec_away_from_the_middle_is_read() {
    let m = modules(Symbology::Aztec, "AZTEC TICKET 77", None);
    let (g, gw, gh) = m.gray(4, 2, 0);
    for (w, h, x, y) in [
        (600u32, 600u32, 100u32, 100u32),
        (600, 600, 450, 250),
        (1400, 1000, 120, 600),
    ] {
        let mut page = vec![255u8; (w * h) as usize];
        for row in 0..gh {
            let to = ((y + row) * w + x) as usize;
            page[to..to + gw as usize]
                .copy_from_slice(&g[(row * gw) as usize..((row + 1) * gw) as usize]);
        }
        assert_eq!(
            texts(&scan_gray(&page, w, h)),
            vec![(Symbology::Aztec, "AZTEC TICKET 77".to_string())],
            "{w}x{h} at {x}, {y}"
        );
    }
}

#[test]
fn a_code_in_a_large_picture_is_found_in_its_tile() {
    // 24 MP: six tiles and more, the code in the bottom right corner at its own resolution.
    let (w, h) = (6000u32, 4000u32);
    let mut page = vec![255u8; (w * h) as usize];
    let m = modules(Symbology::QrCode, URL, None);
    let (g, gw, gh) = m.gray(3, 4, 0);
    let (x, y) = (w - gw - 50, h - gh - 50);
    for row in 0..gh {
        let to = ((y + row) * w + x) as usize;
        page[to..to + gw as usize]
            .copy_from_slice(&g[(row * gw) as usize..((row + 1) * gw) as usize]);
    }
    // The work is bounded by the decoder calls, not by the machine: the default 10 s per
    // picture ran out under load (`Time(Scan { codes: [QrCode ..] })`, the code found but the
    // other tiles not done), so the scan has the 60 s safety stop.
    // STEPS was measured once (2026-10-27): the decoder calls of all tiles and attempts. At
    // least STEPS: every tile was read, not only the one with the code.
    const STEPS: u64 = 33;
    let before = barcodes::work_steps();
    let scan = scan_gray(&page, w, h);
    let steps = before.zip(barcodes::work_steps()).map(|(b, a)| a - b);
    eprintln!("a code in a large picture: {steps:?} decoder calls");
    assert_eq!(texts(&scan), vec![(Symbology::QrCode, URL.to_string())]);
    // Its box lies at the code.
    let b = scan.codes[0].bounds;
    assert!(
        b[0] + 20 >= x && b[2] <= x + gw + 20 && b[1] + 20 >= y && b[3] <= y + gh + 20,
        "{b:?}"
    );
    if cfg!(debug_assertions) {
        assert!(
            steps.is_some_and(|n| (STEPS..=STEPS + STEPS / 2).contains(&n)),
            "{steps:?} decoder calls, measured {STEPS}"
        );
    } else {
        assert_eq!(steps, None);
    }
}

#[test]
fn the_calling_thread_scan_stops_between_calls_at_the_time_limit() {
    let (gray, w, h) = finders(400);
    let got = decode(
        &Picture {
            pixels: Pixels::Gray(&gray),
            width: w,
            height: h,
        },
        &Limits {
            time: Duration::ZERO,
            ..Limits::default()
        },
    );
    assert_eq!(got, Err(Stop::Time(Scan::default())));
}

#[test]
fn block_noise_gives_no_linear_code() {
    // Noise of 4 pixel blocks reads as an EAN-13 along one row; its rows differ, so it has no
    // bars and is not a code.
    let (w, h) = (2000u32, 2000u32);
    let mut state = 0x9e37_79b9u32;
    let cols = w.div_ceil(4);
    let mut blocks = vec![0u8; cols as usize];
    let mut noise = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        if y % 4 == 0 {
            for v in &mut blocks {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *v = if state & 1 == 0 { 0 } else { 255 };
            }
        }
        for x in 0..w {
            noise.push(blocks[(x / 4) as usize]);
        }
    }
    let got = decode(
        &Picture {
            pixels: Pixels::Gray(&noise),
            width: w,
            height: h,
        },
        &Limits::default(),
    );
    // A busy machine may stop it at the time limit; what it found until then counts too.
    match got {
        Ok(scan) | Err(Stop::Time(scan)) => assert!(scan.codes.is_empty(), "{scan:?}"),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------------------
// Fix round 1

/// `gray` of `w` by `h` turned by `deg` degrees around its middle on a white picture large
/// enough to hold it (nearest pixel).
fn turned(gray: &[u8], w: u32, h: u32, deg: f64) -> (Vec<u8>, u32, u32) {
    let (s, c) = deg.to_radians().sin_cos();
    let side = ((w * w + h * h) as f64).sqrt().ceil() as u32 + 20;
    let mut out = vec![255u8; (side * side) as usize];
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let m = f64::from(side) / 2.0;
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (f64::from(x) - m, f64::from(y) - m);
            let (sx, sy) = (c * dx + s * dy + cx, -s * dx + c * dy + cy);
            if sx >= 0.0 && sy >= 0.0 && (sx as u32) < w && (sy as u32) < h {
                out[(y * side + x) as usize] = gray[(sy as u32 * w + sx as u32) as usize];
            }
        }
    }
    (out, side, side)
}

#[test]
fn linear_codes_turned_a_quarter_or_a_little_are_read() {
    let m = modules(Symbology::Code128, "RZ-2026-1021", None);
    let (gray, w, h) = m.gray(3, 10, 40);
    for deg in [90.0, 7.0, -7.0] {
        let (t, tw, th) = turned(&gray, w, h, deg);
        assert_eq!(
            texts(&scan_gray(&t, tw, th)),
            vec![(Symbology::Code128, "RZ-2026-1021".to_string())],
            "{deg} degrees"
        );
    }
}

#[test]
fn codes_cut_tight_are_read() {
    for (s, text) in [
        (Symbology::QrCode, URL),
        (Symbology::DataMatrix, "DM TIGHT 4711"),
        (Symbology::Code128, "TIGHT-0042"),
    ] {
        let (gray, w, h) = modules(s, text, None).gray(3, 0, 30);
        assert_eq!(
            texts(&scan_gray(&gray, w, h)),
            vec![(s, text.to_string())],
            "{s:?}"
        );
    }
}

#[test]
fn binary_aztec_and_pdf417_payloads_come_back_as_their_bytes() {
    let raw: Vec<u8> = vec![0, 1, 0xff, 0x41, 0x42, 0x80, 0x9f, 0x10];
    for s in [Symbology::Aztec, Symbology::Pdf417] {
        let m = encode::bytes(s, &raw, &Options::default()).expect("encode");
        let (gray, w, h) = m.gray(3, 4, 0);
        let scan = scan_gray(&gray, w, h);
        assert_eq!(scan.codes.len(), 1, "{s:?}: {scan:?}");
        assert_eq!(scan.codes[0].payload, Payload::Bytes(raw.clone()), "{s:?}");
        assert_eq!(scan.codes[0].bytes, raw.len(), "{s:?}");
    }
}

#[test]
fn the_size_is_the_payload_bytes() {
    // 20 bytes of ISO-8859-1 letters: text of 40 UTF-8 bytes, a payload of 20.
    let raw: Vec<u8> = (0xc0u8..0xd4).collect();
    let m = encode::bytes(Symbology::QrCode, &raw, &Options::default()).expect("encode");
    let (gray, w, h) = m.gray(3, 4, 0);
    let scan = scan_gray(&gray, w, h);
    assert!(
        matches!(scan.codes[0].payload, Payload::Text(_)),
        "{scan:?}"
    );
    assert_eq!(scan.codes[0].bytes, 20);
}

#[test]
fn the_quick_attempts_are_gray_and_inverted_only() {
    // Pure red on a green of the same luma splits only in a color channel: the full scan
    // reads it, the quick one does not. An inverted code the quick scan reads.
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    let rgba = colored(&gray, [255, 0, 0, 255], [0, 76, 0, 255]);
    let pic = Picture {
        pixels: Pixels::Rgba(&rgba),
        width: w,
        height: h,
    };
    let quick = Limits {
        quick: true,
        ..Limits::default()
    };
    assert_eq!(
        decode(&pic, &Limits::default()).expect("scan").codes.len(),
        1
    );
    assert!(decode(&pic, &quick).expect("scan").codes.is_empty());
    let inverted: Vec<u8> = gray.iter().map(|v| 255 - v).collect();
    let pic = Picture {
        pixels: Pixels::Gray(&inverted),
        width: w,
        height: h,
    };
    assert_eq!(decode(&pic, &quick).expect("scan").codes.len(), 1);
}

#[test]
fn small_codes_of_one_pixel_a_module_are_read() {
    // Data Matrix gives no points at this size; Aztec needs the doubled attempt.
    for (s, text) in [
        (Symbology::DataMatrix, "PARCEL 00340434161094042016"),
        (Symbology::Aztec, "PARCEL 00340434161094042016"),
    ] {
        let (gray, w, h) = modules(s, text, None).gray(1, 4, 0);
        assert_eq!(
            texts(&scan_gray(&gray, w, h)),
            vec![(s, text.to_string())],
            "{s:?}"
        );
    }
}

#[test]
fn work_steps_count_decoder_calls() {
    let m = modules(Symbology::QrCode, URL, None);
    let (gray, w, h) = m.gray(4, 4, 0);
    let picture = Picture {
        pixels: Pixels::Gray(&gray),
        width: w,
        height: h,
    };
    let steps = || {
        let before = barcodes::work_steps();
        decode(&picture, &Limits::default()).expect("scan");
        let after = barcodes::work_steps();
        before.zip(after).map(|(b, a)| a - b)
    };
    let (first, second) = (steps(), steps());
    if cfg!(debug_assertions) {
        assert!(first.is_some_and(|n| n > 0), "{first:?}");
        assert_eq!(first, second);
    } else {
        assert_eq!((first, second), (None, None));
    }
}
