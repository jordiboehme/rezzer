// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writes codes as module grids, for tests that need a code to read (feature `encode`).

use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};

use crate::Symbology;

/// The modules of a code: `dark[y * width + x]`. A linear code has one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modules {
    pub width: u32,
    pub height: u32,
    pub dark: Vec<bool>,
}

/// How to write a code.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The error correction level: `L`, `M`, `Q` or `H` for QR codes, a percentage for Aztec,
    /// `0` to `8` for PDF417.
    pub error_correction: Option<String>,
}

fn format(s: Symbology) -> BarcodeFormat {
    match s {
        Symbology::QrCode => BarcodeFormat::QR_CODE,
        Symbology::MicroQrCode => BarcodeFormat::MICRO_QR_CODE,
        Symbology::DataMatrix => BarcodeFormat::DATA_MATRIX,
        Symbology::Aztec => BarcodeFormat::AZTEC,
        Symbology::Pdf417 => BarcodeFormat::PDF_417,
        Symbology::Code128 => BarcodeFormat::CODE_128,
        Symbology::Code39 => BarcodeFormat::CODE_39,
        Symbology::Ean13 => BarcodeFormat::EAN_13,
        Symbology::UpcA => BarcodeFormat::UPC_A,
        Symbology::Itf => BarcodeFormat::ITF,
    }
}

/// `text` written as a code without a quiet zone.
pub fn text(symbology: Symbology, text: &str, options: &Options) -> Result<Modules, String> {
    write(symbology, text, None, options)
}

/// `bytes` written as a code in byte mode (ISO-8859-1), without a quiet zone.
pub fn bytes(symbology: Symbology, bytes: &[u8], options: &Options) -> Result<Modules, String> {
    let text: String = bytes.iter().map(|&b| char::from(b)).collect();
    write(symbology, &text, Some("ISO-8859-1"), options)
}

fn write(
    symbology: Symbology,
    contents: &str,
    charset: Option<&str>,
    options: &Options,
) -> Result<Modules, String> {
    let hints = EncodeHints {
        Margin: Some("0".into()),
        ErrorCorrection: options.error_correction.clone(),
        CharacterSet: charset.map(str::to_string),
        ..Default::default()
    };
    let m = MultiFormatWriter
        .encode_with_hints(contents, &format(symbology), 0, 0, &hints)
        .map_err(|e| e.to_string())?;
    let (w, h) = (m.getWidth(), m.getHeight());
    // A linear code comes as one row or as rows that repeat it.
    let rows = if symbology.two_d() { h } else { 1 };
    let mut dark = Vec::with_capacity(w as usize * rows as usize);
    for y in 0..rows {
        for x in 0..w {
            dark.push(m.get(x, y));
        }
    }
    Ok(Modules {
        width: w,
        height: rows,
        dark,
    })
}

impl Modules {
    /// The code as gray pixels, `px` pixels per module, with a quiet zone of `quiet` modules;
    /// a linear code is `bar` modules high. Returns the pixels, width and height.
    pub fn gray(&self, px: u32, quiet: u32, bar: u32) -> (Vec<u8>, u32, u32) {
        let rows = if self.height == 1 {
            bar.max(1)
        } else {
            self.height
        };
        let w = (self.width + 2 * quiet) * px;
        let h = (rows + 2 * quiet) * px;
        let mut out = vec![255u8; w as usize * h as usize];
        for my in 0..rows {
            let sy = if self.height == 1 { 0 } else { my };
            for mx in 0..self.width {
                if !self.dark[(sy * self.width + mx) as usize] {
                    continue;
                }
                for dy in 0..px {
                    let y = (my + quiet) * px + dy;
                    let at = (y * w + (mx + quiet) * px) as usize;
                    out[at..at + px as usize].fill(0);
                }
            }
        }
        (out, w, h)
    }
}
