# zipguard

zipguard opens a ZIP archive that you do not trust and keeps it inside limits.

- Limits for the number of entries, the size of one entry, the size of all entries together and the compression ratio
- The limits count the bytes that are really unpacked. A size written in the archive header is never believed
- Names with `..`, absolute paths or a NUL byte are refused
- An archive that can be read in two ways is refused: two entries with the same name, a local header that names another file than the central directory, or entries that overlap
- Encrypted entries are reported, never read
- `measure` unpacks an entry into a counter and keeps nothing, to learn its real size

```rust
use zipguard::{Archive, Limits};

# fn demo(bytes: &[u8]) -> Result<(), zipguard::ZipError> {
let mut archive = Archive::open(bytes, Limits::default())?;
if archive.contains("xl/workbook.xml") {
    let xml = archive.read("xl/workbook.xml")?;
    println!("{} bytes", xml.len());
}
# Ok(())
# }
```

The archive stays in memory as a byte slice. Nothing is written to disk and no entry is ever run.

License: MPL-2.0
