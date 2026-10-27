# fileattrs: format facts

Clean-room note. Facts only, taken from public descriptions listed under Sources. It contains
no code and no pseudocode, and nothing was copied from any plist library or from Apple's
CFBinaryPList.c. Statements marked "Rezzer policy" are decisions for a safe reader, not
claims about the format. Statements marked "unconfirmed" were not found in a public
non-code source.

## 1. Binary property list (bplist00)

Used for the macOS extended attributes in section 3. Apple calls the binary format opaque and
does not publish a formal specification. The layout below is the one described by the
sources.

### 1.1 File layout

A file has four parts in this order: an 8-byte header, the object table, the offset table and
a 32-byte trailer. Objects have no separators. A reader finds everything by reading the
trailer first, from the end of the file.

All multi-byte integers are big-endian (most significant byte first). UTF-16 code units are
big-endian too, so UTF-16BE, without a byte order mark.

### 1.2 Header

Eight bytes: the ASCII text `bplist` followed by a two-digit ASCII version, `00`
(hex 62 70 6C 69 73 74 30 30). Only version `00` is described publicly. Other versions
exist (for example a newer one with extra object types) but are not documented; Rezzer policy:
reject anything but `bplist00` as "not a readable property list".

### 1.3 Trailer (last 32 bytes)

| Offset from end of file | Size | Field |
|---|---|---|
| -32 | 5 | unused |
| -27 | 1 | sort version (ordering of the object table; not needed for reading) |
| -26 | 1 | offset int size: width in bytes of each offset table entry |
| -25 | 1 | object ref size: width in bytes of each reference inside arrays, sets and dictionaries |
| -24 | 8 | number of objects (equals the number of offset table entries) |
| -16 | 8 | top object: index of the root object (usually 0, must not be assumed) |
| -8 | 8 | offset table start: absolute byte position of the offset table |

The sort version, the unused bytes and the exact numeric limits are not needed to read a
file.

### 1.4 Offset table

Starts at the offset table start from the trailer. It holds one entry per object, each entry
exactly offset-int-size bytes, big-endian, no separators. Entry n is the absolute byte
position (from the start of the file) of object n. Array, set and dictionary members refer to
objects by index (a reference, object-ref-size bytes wide), not by position, so one object can
be shared by several containers.

### 1.5 Object marker

Each object starts with one marker byte. The high nibble is the type, the low nibble is
usually a length or count.

| Marker (high nibble) | Type |
|---|---|
| 0x0 | null (0x00), false (0x08), true (0x09), fill (0x0F) |
| 0x1 | integer: low nibble n means the value takes 2^n bytes (1, 2, 4, 8; 0x14 = 16 bytes exists in newer files, unconfirmed) |
| 0x2 | real: 4 or 8 bytes (low nibble 2 or 3) |
| 0x3 | date: marker 0x33, 8-byte float, seconds since 2001-01-01 00:00:00 UTC |
| 0x4 | data (raw bytes), low nibble is the byte count |
| 0x5 | ASCII string, low nibble is the character count (= byte count) |
| 0x6 | UTF-16 string, low nibble is the count of 16-bit code units (not bytes), big-endian |
| 0x7 | UTF-8 string, low nibble is the byte count |
| 0x8 | UID, low nibble is the byte count minus 1 |
| 0xA | array, low nibble is the element count |
| 0xC | set, low nibble is the element count |
| 0xD | dictionary, low nibble is the entry count |

Rezzer only needs array (0xA), ASCII string (0x5) and UTF-16 string (0x6), plus the integer
marker for lengths. UTF-8 strings (0x7) are a described type; a reader may accept them.
Every other type inside the two attributes is "not read".

### 1.6 Length encoding

- Low nibble 0x0 to 0xE: the nibble is the length or count.
- Low nibble 0xF: the length does not fit. Immediately after the marker byte comes a complete
  integer object (its own marker 0x1n, then 2^n bytes big-endian) holding the real length or
  count. That integer object is not in the offset table.

### 1.7 Array content

An array object is the marker (and the optional length integer), followed by count references,
each object-ref-size bytes, big-endian. Each reference is an index into the offset table. For
a string object the characters follow the marker (and the optional length integer) directly:
ASCII bytes, or count x 2 bytes of UTF-16BE.

### 1.8 Invalid files (Rezzer policy, derived from the layout)

A reader must treat the file as invalid, without panicking or allocating from untrusted
numbers, when:

- the file is shorter than 8 + 32 bytes, or the header is not `bplist00`
- offset int size or object ref size is 0 or greater than 8
- the offset table start plus (object count x offset int size) is beyond the start of the
  trailer, or the multiplication overflows
- the object count is larger than the data allows (it can be at most the number of bytes
  before the trailer, since each object takes at least one byte), or is 0
- the top object index is not smaller than the object count
- any offset is not inside the object area (header end up to the offset table start)
- a reference index is not smaller than the object count
- a length makes an object or its elements run past the offset table start
- references form a cycle (an array that contains itself directly or through other containers)
  or nest deeper than a fixed limit
- a string length integer uses a marker that is not an integer, or is negative

Sizes: a reference count of an array cannot exceed (offset table start - object start) /
object-ref-size. A reader should also cap the total number of objects visited and the nesting
depth, so a small file cannot expand.

## 2. Attribute shapes

### 2.1 macOS

Extended attributes on files, names exactly as given:

- `com.apple.metadata:kMDItemWhereFroms`: a binary plist holding an array of strings. The
  entries are URLs, usually the download address first and the referring page second. The
  array can have one entry, and an entry can be an empty string (all of this is from common
  forensic practice, not from a formal specification: unconfirmed). Strings may be ASCII or
  UTF-16 objects.
- `com.apple.metadata:_kMDItemUserTags`: a binary plist holding an array of strings (the array
  can be empty). Each string is the tag name, then a line feed (0x0A), then a colour number as
  a decimal digit. Example: `Red` + LF + `6`. A tag with no colour can be written with no
  line feed and no number, or with number 0 (reports differ: handle both).

Finder tag colour numbers: 0 none, 1 grey, 2 green, 3 purple, 4 blue, 5 yellow, 6 red,
7 orange. Tag names may contain any Unicode text, so they are UTF-16 strings in the plist
when they contain non-ASCII characters. Separate from the attribute, a Finder tag colour
label is also kept in `com.apple.FinderInfo`; Rezzer does not read it.

The attribute can remain on a file after the last tag is removed, as an empty array.

### 2.2 Linux / freedesktop

Namespace `user.` extended attributes. Names are written top level first, with periods:
`user.<namespace>....<name>`. Attribute strings should be UTF-8.

- `user.xdg.origin.url`: set on a file downloaded from a URL; the value is the URL it came
  from. Described as a proposed attribute. The value is plain UTF-8 text, not a plist.
- `user.xdg.tags`: a comma-separated list of tags, UTF-8, as written by KDE Dolphin and
  indexed by Baloo. De facto, not an official freedesktop standard. Hierarchical tags have
  been seen written as slash-separated paths (for example `parent/child`). The attribute can
  be left present and empty after tags are removed. The exact escaping of a comma inside a tag
  name is not documented: unconfirmed.
- Other freedesktop attributes in the same document (not read by Rezzer): `user.xdg.comment`,
  `user.xdg.origin.email.subject`, `.from`, `.message-id`, `user.xdg.language`,
  `user.xdg.publisher`, `user.xdg.robots.index`, `user.xdg.robots.backup`.

### 2.3 Windows Mark of the Web

An NTFS alternate data stream named `Zone.Identifier` on the file (`file.ext:Zone.Identifier`).
It is lost when the file moves to a file system without streams (for example FAT32) and is
removed when the user unblocks the file. Chromium-based browsers, Edge and Explorer write
it; it is also set on files extracted from a downloaded archive, with `ReferrerUrl` pointing
at the archive.

Content is plain text in INI layout: a group header `[ZoneTransfer]` on its own line, then
`Key=Value` lines. Line ends are CR LF in files written by Windows. Keys seen:

- `ZoneId`: the security zone number (URLZONE): -1 invalid, 0 local machine, 1 intranet,
  2 trusted sites, 3 internet, 4 restricted (untrusted). Values 1000 to 10000 are reserved for
  user-defined zones. Older streams contain only `ZoneId`.
- `HostUrl`: the address the file was downloaded from.
- `ReferrerUrl`: the page that linked to the download.
- `HostIpAddress`, `AppZoneId`, `LastWriterPackageFamilyName`: named in forensic descriptions,
  meaning not documented in the sources. Rezzer ignores them.

`HostUrl` can be the literal text `about:internet` for some downloads (reported by forensic
practice: unconfirmed).

Text encoding: the sources do not state it. Streams are commonly seen as ANSI/UTF-8 text, and
a stream can also be UTF-16LE with a byte order mark (FF FE). Rezzer policy: if the stream
starts with FF FE decode as UTF-16LE (and drop the BOM), if it starts with EF BB BF decode as
UTF-8 and drop the BOM, otherwise decode as UTF-8 with lossy replacement. Match the group
header and key names case-insensitively; split each line at the first `=`; ignore unknown
keys and a missing `[ZoneTransfer]` header is "not read".

## Sources

- Wikipedia, Property list (binary format, markers, nibble rules, big-endian):
  https://en.wikipedia.org/wiki/Property_list
- Belkasoft, Plist Files: Structure, Forensic Value, and How to Analyze Them (32-byte
  trailer layout, offset table, 0xF length-follows case, marker table):
  https://belkasoft.com/plist-files-in-dfir
- Eclectic Light Company, xattr com.apple.metadata:_kMDItemUserTags, Finder tags (tag
  string shape and colour numbers):
  https://eclecticlight.co/2017/12/27/xattr-com-apple-metadata_kmditemusertags-finder-tags/
- freedesktop.org, Guidelines for extended attributes (user.xdg.* names, UTF-8, origin.url):
  https://wiki.archlinux.org/title/Xattrs is the source for user.xdg.tags (Arch Wiki,
  comma-separated); the freedesktop page is https://wiki.freedesktop.org/www/CommonExtendedAttributes/
- KDE Bugzilla 376117, Baloo leaves empty user.xdg.tags xattr (empty attribute case):
  https://bugs.kde.org/show_bug.cgi?id=376117
- Digital Detective, Forensic Analysis of the Zone.Identifier Stream (INI shape, keys, zone
  values): https://www.digital-detective.net/?p=10165
- Microsoft, URLZONE enumeration (zone numbers, background for Mark of the Web):
  https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/platform-apis/ms537175(v=vs.85)
  (named as the origin of the enum; its text was not fetched for this note)
