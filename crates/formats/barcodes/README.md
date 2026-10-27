# barcodes

Reads barcodes and QR codes in untrusted pictures, with bounds.

- Input is 8-bit gray or RGBA pixels. Transparent pixels count as white.
- QR Code, Data Matrix, Aztec and PDF417, then Code 128, Code 39, EAN-13, UPC-A and ITF. Micro QR Code is looked for but not tested.
- MaxiCode, EAN-8 and UPC-E are not read.
- A large picture is read at its own resolution in overlapping tiles.
- Each picture has a time limit, a pixel cap, a code cap and a payload cap.
- `Worker` runs the decoder on worker threads with one shared queue and stops waiting at the time limit. A call past its time is left to end on its own, and a new thread takes the next call. At most two such calls run at once. Past that, calls wait for them.

The decoder is [rxing](https://github.com/rxing-core/rxing) (Apache-2.0).

```rust
use barcodes::{decode, Limits, Picture, Pixels};

let gray = vec![255u8; 100 * 100];
let picture = Picture { pixels: Pixels::Gray(&gray), width: 100, height: 100 };
let scan = decode(&picture, &Limits::default()).unwrap();
assert!(scan.codes.is_empty());
```

License: MPL-2.0.
