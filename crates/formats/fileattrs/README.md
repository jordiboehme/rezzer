# fileattrs

fileattrs reads where a file came from. It reads the metadata a file system keeps beside a file, not the file content.

What it reads:

- Binary property lists (`bplist00`) whose top object is an array of strings. macOS keeps the download addresses of a file in `com.apple.metadata:kMDItemWhereFroms` and its Finder tags in `com.apple.metadata:_kMDItemUserTags` in this shape. ASCII, UTF-16 and UTF-8 strings are read. Other entries are skipped.
- Tag names. `finder_tags` reads the macOS Finder tags and drops the colour number after each name. `xdg_tags` splits the comma-separated `user.xdg.tags` value of Linux desktops. Both give at most 64 names (`MAX_TAGS`).
- The Windows `Zone.Identifier` stream. `zone_text` decodes UTF-8 or UTF-16LE with a byte order mark and `zone_host_url` returns the `HostUrl` of the `[ZoneTransfer]` section.
- Extended attributes of an open file on macOS and Linux. `read_xattr` asks for the size first and never reads more than the cap you give.

```rust
let text = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/f.zip\r\n";
assert_eq!(
    fileattrs::zone_host_url(text).as_deref(),
    Some("https://example.com/f.zip")
);
```

Limits:

- At most 1,024 strings and 1 MiB of text per property list
- Containers nest at most 8 deep and at most 16,384 objects are checked
- A cycle, an offset or reference outside the data or a length past the end is an error, never a panic
- `MAX_ATTR_BYTES` is 64 KiB. A larger attribute gives `None`, as does a missing one or a file system without extended attributes

License: MPL-2.0
