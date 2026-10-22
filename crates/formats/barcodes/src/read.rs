// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The calls into rxing: one prepared gray picture, both passes.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use rxing::common::HybridBinarizer;
use rxing::multi::{GenericMultipleBarcodeReader, MultipleBarcodeReader};
use rxing::{
    BarcodeFormat, BinaryBitmap, DecodeHints, Luma8LuminanceSource, MultiUseMultiFormatReader,
    RXingResult, RXingResultMetadataType, RXingResultMetadataValue,
};

use crate::{Code, Payload, Runner, Scan, Stop, Symbology};

/// One gray picture to read, quiet zone included.
pub(crate) struct Job {
    pub gray: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Only Aztec: a window of a larger picture, around a place its finder may be.
    pub aztec_only: bool,
}

/// One code as rxing reads it, in the job's pixels.
#[derive(Debug, Clone)]
pub(crate) struct Hit {
    pub symbology: Symbology,
    pub text: String,
    /// The payload's bytes when rxing gives them: the byte segments of a Data Matrix or a QR
    /// code read by the older reader, the content of a QR code read by the newer one.
    pub bytes: Option<Vec<u8>>,
    pub points: Vec<(f32, f32)>,
}

/// Runs each call in the calling thread.
pub(crate) struct Inline;

impl Runner for Inline {
    fn run(&mut self, job: Job, deadline: Instant) -> Result<Vec<Hit>, Stop> {
        if Instant::now() >= deadline {
            return Err(Stop::Time(Scan::default()));
        }
        run(job)
    }
}

/// The formats of the first pass. MaxiCode is never asked for.
const TWO_D: [BarcodeFormat; 5] = [
    BarcodeFormat::QR_CODE,
    BarcodeFormat::MICRO_QR_CODE,
    BarcodeFormat::DATA_MATRIX,
    BarcodeFormat::AZTEC,
    BarcodeFormat::PDF_417,
];

/// The formats of the second pass. EAN-8 and UPC-E are left out: they give false hits.
const ONE_D: [BarcodeFormat; 5] = [
    BarcodeFormat::CODE_128,
    BarcodeFormat::CODE_39,
    BarcodeFormat::EAN_13,
    BarcodeFormat::UPC_A,
    BarcodeFormat::ITF,
];

fn symbology(f: &BarcodeFormat) -> Option<Symbology> {
    Some(match f {
        BarcodeFormat::QR_CODE => Symbology::QrCode,
        BarcodeFormat::MICRO_QR_CODE => Symbology::MicroQrCode,
        BarcodeFormat::DATA_MATRIX => Symbology::DataMatrix,
        BarcodeFormat::AZTEC => Symbology::Aztec,
        BarcodeFormat::PDF_417 => Symbology::Pdf417,
        BarcodeFormat::CODE_128 => Symbology::Code128,
        BarcodeFormat::CODE_39 => Symbology::Code39,
        BarcodeFormat::EAN_13 => Symbology::Ean13,
        BarcodeFormat::UPC_A => Symbology::UpcA,
        BarcodeFormat::ITF => Symbology::Itf,
        _ => return None,
    })
}

/// Reads `job` in both passes, with every panic caught.
pub(crate) fn run(job: Job) -> Result<Vec<Hit>, Stop> {
    panic::catch_unwind(AssertUnwindSafe(|| read(job))).unwrap_or(Err(Stop::Internal))
}

fn hints(formats: &[BarcodeFormat]) -> DecodeHints {
    DecodeHints {
        TryHarder: Some(true),
        // The inverted picture is an attempt of its own.
        AlsoInverted: Some(false),
        PossibleFormats: Some(formats.iter().copied().collect::<HashSet<_>>()),
        ..Default::default()
    }
}

fn read(job: Job) -> Result<Vec<Hit>, Stop> {
    let aztec_only = job.aztec_only;
    let Ok(source) = Luma8LuminanceSource::new(job.gray, job.width, job.height) else {
        return Err(Stop::BadSize);
    };
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(source));
    let mut out = Vec::new();
    let passes: &[&[BarcodeFormat]] = if aztec_only {
        &[&[BarcodeFormat::AZTEC]]
    } else {
        &[&TWO_D, &ONE_D]
    };
    for &formats in passes {
        let hints = hints(formats);
        let mut reader = GenericMultipleBarcodeReader::new(MultiUseMultiFormatReader::default());
        // A picture without a code is an error to rxing.
        let found = reader
            .decode_multiple_with_hints(&mut bitmap, &hints)
            .unwrap_or_default();
        out.extend(found.iter().filter_map(hit));
    }
    Ok(out)
}

fn hit(r: &RXingResult) -> Option<Hit> {
    let symbology = symbology(r.getBarcodeFormat())?;
    let bytes = match r
        .getRXingResultMetadata()
        .get(&RXingResultMetadataType::BYTE_SEGMENTS)
    {
        // Byte segments may be only a part of the payload.
        Some(RXingResultMetadataValue::ByteSegments(s)) => {
            let all = s.concat();
            (latin1(r.getText()).as_ref() == Some(&all)
                || std::str::from_utf8(&all).ok() == Some(r.getText()))
            .then_some(all)
        }
        // The newer QR reader keeps the payload as its raw bytes; the older one keeps the
        // code words there, but it always gives byte segments for a byte payload.
        _ if matches!(symbology, Symbology::QrCode | Symbology::MicroQrCode)
            && !r.getRawBytes().is_empty() =>
        {
            Some(r.getRawBytes().to_vec())
        }
        _ => None,
    };
    Some(Hit {
        symbology,
        text: r.getText().to_string(),
        bytes,
        points: r.getPoints().iter().map(|p| (p.x, p.y)).collect(),
    })
}

/// `text` as ISO-8859-1 bytes, when every character fits.
fn latin1(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect()
}

/// Whether `c` may stand in a text payload: tab, line breaks, and the separators of GS1 and
/// ISO/IEC 15434 (EOT, FS, GS, RS). Every other control character makes the payload bytes.
fn allowed(c: char) -> bool {
    !c.is_control()
        || matches!(
            c,
            '\t' | '\n' | '\r' | '\u{4}' | '\u{1c}' | '\u{1d}' | '\u{1e}'
        )
}

/// The payload of `hit`, judged on its bytes: valid UTF-8 is text; else bytes that are all
/// printable ISO-8859-1 (the default character set of QR codes and PDF417) are text in that
/// set; else they are bytes. rxing turns every payload into a string (unknown bytes as
/// ISO-8859-1), so the string alone cannot tell text from bytes. A payload rxing read as
/// characters outside ISO-8859-1 (through an ECI) is text as it is.
/// Also the payload's size in bytes as the code holds it.
pub(crate) fn payload(hit: &Hit, max: usize) -> (Payload, usize) {
    let bytes = match hit.bytes.clone().or_else(|| latin1(&hit.text)) {
        Some(b) => b,
        None => {
            let t = hit.text.clone();
            let n = t.len();
            if n > max {
                return (Payload::TooLarge(n), n);
            }
            return if t.chars().all(allowed) {
                (Payload::Text(t), n)
            } else {
                (Payload::Bytes(t.into_bytes()), n)
            };
        }
    };
    let n = bytes.len();
    if n > max {
        return (Payload::TooLarge(n), n);
    }
    if let Ok(t) = std::str::from_utf8(&bytes)
        && t.chars().all(allowed)
    {
        return (Payload::Text(t.to_string()), n);
    }
    let printable = |b: u8| b >= 0xa0 || (0x20..0x7f).contains(&b) || allowed(char::from(b));
    if bytes.iter().all(|&b| printable(b)) {
        return (
            Payload::Text(bytes.iter().map(|&b| char::from(b)).collect()),
            n,
        );
    }
    (Payload::Bytes(bytes), n)
}

/// The code of `hit`, its points moved into the picture by `map` and clamped to its `w` by
/// `h` pixels. A hit without points (rxing gives none for some small codes) lies somewhere in
/// `area`, the part of the picture that was read.
pub(crate) fn code_of(
    hit: &Hit,
    map: &dyn Fn(f32, f32) -> (f32, f32),
    (w, h): (u32, u32),
    area: [u32; 4],
    max_payload: usize,
) -> Option<Code> {
    let points: Vec<(f32, f32)> = hit
        .points
        .iter()
        .map(|&(x, y)| map(x, y))
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    let Some(first) = points.first() else {
        let (payload, bytes) = payload(hit, max_payload);
        return Some(Code {
            symbology: hit.symbology,
            payload,
            bytes,
            bounds: [
                area[0].min(w),
                area[1].min(h),
                area[2].min(w),
                area[3].min(h),
            ],
        });
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for &(x, y) in &points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    // QR finder points are module centres inside the code, and a linear code gives the line
    // it was read on: both are widened to about the code's box.
    let (gx, gy) = if hit.symbology.two_d() {
        let m = (x1 - x0).max(y1 - y0) * 0.1;
        (m, m)
    } else {
        (0.0, ((x1 - x0) / 8.0).max(2.0))
    };
    let clamp = |v: f32, max: u32| v.clamp(0.0, max as f32).round() as u32;
    let (payload, bytes) = payload(hit, max_payload);
    Some(Code {
        symbology: hit.symbology,
        payload,
        bytes,
        bounds: [
            clamp(x0 - gx, w),
            clamp(y0 - gy, h),
            clamp(x1 + gx, w),
            clamp(y1 + gy, h),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(text: &str, bytes: Option<&[u8]>) -> Hit {
        Hit {
            symbology: Symbology::Aztec,
            text: text.into(),
            bytes: bytes.map(<[u8]>::to_vec),
            points: vec![],
        }
    }

    #[test]
    fn bytes_read_as_latin1_stay_bytes() {
        let raw = [0x00u8, 0xff, 0x10, 0x80];
        let text: String = raw.iter().map(|&b| char::from(b)).collect();
        assert_eq!(
            payload(&hit(&text, Some(&raw)), 100).0,
            Payload::Bytes(raw.to_vec())
        );
    }

    #[test]
    fn utf8_bytes_are_text() {
        let raw = "Grüße".as_bytes();
        // rxing may read UTF-8 bytes as ISO-8859-1.
        let misread: String = raw.iter().map(|&b| char::from(b)).collect();
        assert_eq!(
            payload(&hit(&misread, Some(raw)), 100).0,
            Payload::Text("Grüße".into())
        );
    }

    #[test]
    fn gs1_separators_stay_text_and_other_controls_do_not() {
        assert_eq!(
            payload(&hit("01\u{1d}10", None), 100).0,
            Payload::Text("01\u{1d}10".into())
        );
        assert_eq!(
            payload(&hit("a\u{7}b", None), 100).0,
            Payload::Bytes(b"a\x07b".to_vec())
        );
    }

    #[test]
    fn printable_latin1_is_text() {
        assert_eq!(
            payload(&hit("Gr\u{fc}\u{df}e", None), 100).0,
            Payload::Text("Grüße".into())
        );
    }

    #[test]
    fn a_payload_over_the_cap_is_only_counted() {
        assert_eq!(payload(&hit("abcdef", None), 4).0, Payload::TooLarge(6));
    }
}
