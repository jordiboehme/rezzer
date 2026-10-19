# DWG File Format

Scope: byte and bit level description of AutoCAD DWG files R13 (AC1012) to R2018 (AC1032), focused on reading structure, metadata and text for knowledge extraction.

Status: draft

---

## 1. Conventions

- All multi-byte values are little-endian unless a field says otherwise.
- Offsets are given in hexadecimal with a `0x` prefix.
- "Section" means a logical data stream (for example `AcDb:Header`). In R2004 and later a section is stored in one or more "pages".
- "Handle" is the 64-bit object identifier used for every object in the database.
- Type codes used in the tables are defined in chapter 3. Version ranges are inclusive: "R2000+" means R2000 and every later version, "R13-R14" means R13 and R14 only.
- Field names are the public AutoCAD names where they exist (DXF group codes, header variable names such as `$INSUNITS`), otherwise a short descriptive name.
- DXF group codes are given in a "DXF" column where useful, because the DXF Reference explains the meaning of most values.

---

## 2. Version identification

The first 6 bytes of every DWG file are an ASCII version string.

| Magic | Release name | AutoCAD releases that write it natively | Notes |
|---|---|---|---|
| AC1012 | R13 | AutoCAD R13 | |
| AC1013 | R13C3 | AutoCAD R13 c3 and later c-updates | Rare, mostly same layout as AC1012 with the C3 changes |
| AC1014 | R14 | AutoCAD R14, R14.01 | |
| AC1015 | R2000 | AutoCAD 2000, 2000i, 2002 | Some tools also report AC1016/AC1017 for 2000i/2002; real files use AC1015 |
| AC1018 | R2004 | AutoCAD 2004, 2005, 2006 | New paged container |
| AC1021 | R2007 | AutoCAD 2007, 2008, 2009 | Unique container (Reed-Solomon, other compression), Unicode strings |
| AC1024 | R2010 | AutoCAD 2010, 2011, 2012 | R2004 container again, R2007 bit coding |
| AC1027 | R2013 | AutoCAD 2013 to 2017 | |
| AC1032 | R2018 | AutoCAD 2018 and later | Still used by current releases |

Older strings (AC1001 to AC1009, AC1.x, AC2.x, MC0.0) belong to pre-R13 formats that have a completely different structure; they are out of scope.

Byte `0x0B` holds the maintenance release number (`$ACADMAINTVER`). Together with the version string it identifies the exact writer version. Several layout details depend on it (see 8.1 and 9.1).

The R2004+ auxiliary header and several objects use a numeric version code:

| Code | Version |
|---|---|
| 17 | AC1010 |
| 18 | AC1011 |
| 19 | AC1012 (R13) |
| 20 | AC1013 |
| 21 | AC1014 (R14) |
| 22 | AC1015 beta |
| 23 | AC1015 (R2000) |
| 24 | AC1018 beta |
| 25 | AC1018 (R2004) |
| 26 | AC1021 beta |
| 27 | AC1021 (R2007) |
| 28 | AC1024 beta |
| 29 | AC1024 (R2010) |
| 30 | AC1027 beta |
| 31 | AC1027 (R2013) |
| 32 | AC1032 beta |
| 33 | AC1032 (R2018) |

### 2.1 Version families and support order

| Family | Versions | Container | Bit coding | Strings |
|---|---|---|---|---|
| A | R13, R14, R2000 | Flat file, section locator table at `0x15` | R13 style (R2000 adds compact forms) | 8-bit in drawing code page |
| B | R2004 | Paged, LZ77 variant compression, masked page headers | as R2000 | 8-bit in drawing code page |
| C | R2007 | Paged, Reed-Solomon coding, second LZ77 variant | R2007 style with separate string stream | UTF-16LE |
| D | R2010, R2013, R2018 | As family B | As family C plus new object type coding and handle stream size | UTF-16LE |

Recommended implementation order: R2000 (family A) first, then families B and D (they share the container), then C (R2007, the most complex container, but rare in the wild). R13 and R14 differ from R2000 only in many small field-level details (marked "R13-R14" in this document); they can be added late.

---

## 3. Bit-level primitives

### 3.1 Bit order

Most data in the header, classes and objects sections is a bit stream. Bits are consumed from the most significant bit of each byte to the least significant bit, then the next byte. A "raw" multi-byte value inside a bit stream (RC, RS, RL, RD) is read as consecutive 8-bit groups starting at the current bit position (not necessarily byte aligned); each group is one byte, and the bytes are assembled little-endian.

### 3.2 Type codes

| Code | Meaning | Size |
|---|---|---|
| B | one bit | 1 bit |
| BB | two bits as an unsigned number 0-3 | 2 bits |
| 3B | 1 to 3 bits (see 3.4) | |
| BS | bit short (16-bit) | 2-18 bits |
| BL | bit long (32-bit) | 2-34 bits |
| BLL | bit long long (64-bit) | 3 bits + 0-7 bytes |
| BD | bit double (IEEE 754 64-bit) | 2-66 bits |
| DD | bit double with default | 2-66 bits |
| 2BD, 3BD | 2 or 3 consecutive BD | |
| 2DD, 3DD | 2 or 3 consecutive DD, each with its own default | |
| RC | raw byte | 8 bits |
| RS | raw 16-bit integer | 16 bits |
| RL | raw 32-bit integer | 32 bits |
| RLL | raw 64-bit integer | 64 bits |
| RD | raw double | 64 bits |
| 2RD, 3RD | 2 or 3 raw doubles | |
| MC | modular char (signed or unsigned variable length) | 1-5 bytes (8 for unsigned handles) |
| MS | modular short | 2 or 4 bytes |
| H | handle reference | 8 bits + 0-8 bytes |
| OT | object type | BS before R2010, special form R2010+ |
| T | text, 8-bit (R13-R2004) | |
| TU | text, UTF-16LE (R2007+) | |
| TV | "variable text": T in R13-R2004, TU in R2007+ (taken from the string stream, see 9.3) | |
| T16, TU16 | RS length + text, used in byte-oriented sections | |
| T32 | RL byte size + text, used in byte-oriented sections | |
| BE | bit extrusion | |
| BT | bit thickness | |
| CMC | color value | |
| ENC | entity color with flags (R2004+) | |
| TIMEBLL | date or duration as two BL | |
| TIMERLL | date or duration as two RL | |
| SN | 16-byte sentinel | 16 bytes |

### 3.3 BS, BL, BD

Each starts with a 2-bit code:

| Code | BS | BL | BD |
|---|---|---|---|
| 00 | RS follows | RL follows | RD follows |
| 01 | RC follows (unsigned 0-255) | RC follows (unsigned 0-255) | value 1.0 |
| 10 | value 0 | value 0 | value 0.0 |
| 11 | value 256 | not used | not used |

Negative numbers always use the full-width form.

### 3.4 3B and BLL

3B: read one bit; if it is 0 stop. Otherwise read another bit; if it is 0 stop; otherwise read a third bit. Each newly read bit is appended as the new least significant bit of the result. Results are 0, 2, 6 or 7.

BLL is used for `$REQUIREDVERSIONS` (R2013+) and for the size of entity graphics (R2010+). Rule that works on real files: read 3 bits as an unsigned number n (first bit is the most significant), then n raw bytes, least significant first; n = 0 means the value 0. (One source describes the length prefix as 3B instead; see open question Q5.)

### 3.5 DD (bit double with default)

A 2-bit code, then data. A default value d is always known from context (usually a previously read value).

| Code | Result |
|---|---|
| 00 | d itself, nothing follows |
| 01 | 4 bytes follow; they replace the 4 least significant bytes of d (d taken as its 8-byte little-endian IEEE image) |
| 10 | 6 bytes follow; the first 2 replace bytes 4 and 5 of d (counting from 0, least significant first), the next 4 replace bytes 0 to 3 |
| 11 | a full RD follows |

### 3.6 BE and BT

- BE (extrusion), R13-R14: 3BD. R2000+: one bit; 1 means extrusion (0,0,1) and nothing follows; 0 means 3BD follows.
- BT (thickness), R13-R14: BD. R2000+: one bit; 1 means 0.0; 0 means BD follows.

### 3.7 Modular char (MC) and modular short (MS)

MC: a sequence of bytes. In each byte bit 7 (0x80) means "another byte follows"; bits 0-6 carry data. The first byte carries the least significant 7 bits, the next byte the next 7 bits, and so on. In the signed variant, bit 6 (0x40) of the last byte (the one with bit 7 clear) is a sign flag: clear it, assemble the value, then negate. The unsigned variant (used for handle offsets in the object map and the R2010+ handle stream size) has no sign bit and may run to 8 bytes.

Example: bytes `82 24` give 2 + (0x24 << 7) = 4610. Bytes `85 4B`: last byte 0x4B has 0x40 set, so the value is -(5 + (0x0B << 7)) = -1413.

MS: like MC but with 16-bit little-endian words. In each word bit 15 means "another word follows"; bits 0-14 are data, first word least significant. At most 2 words are used in practice (object sizes up to 2^30).

### 3.8 Object type (OT)

- R13-R2007: BS.
- R2010+: a BB code, then:
  - 0: one RC; type = that byte.
  - 1: one RC; type = that byte + 0x1F0.
  - 2 or 3: one RS; type = that value.

### 3.9 Handle references (H)

```
first byte: high nibble = code (0-15), low nibble = counter (number of following bytes, 0-8)
then counter bytes: the handle value, most significant byte first
```

Example: `52 05 E7` is code 5, length 2, value 0x05E7.

Codes 2 to 5 carry an absolute handle and say what kind of reference it is:

| Code | Meaning |
|---|---|
| 2 | soft owner |
| 3 | hard owner |
| 4 | soft pointer |
| 5 | hard pointer |

Codes 6, 8, 0xA, 0xC are relative to the handle of the object that contains the reference (they are always soft pointers):

| Code | Absolute handle |
|---|---|
| 6 | reference + 1 (counter is 0) |
| 8 | reference - 1 (counter is 0) |
| 0xA | reference + value |
| 0xC | reference - value |

Code 0 with a counter is used for an object's own handle and for a few "data handles" (for example `$HANDSEED`). A null reference is a handle with value 0. Counters above 4 are rare; files with counter 5 exist. A reader must reject a counter above 8.

### 3.10 Text

- T (R13-R2004): BS length, then that many bytes in the drawing code page (chapter 3.13). Writers since R2004 often include a terminating zero byte in the length; R13-R2000 normally do not. Strip trailing zero bytes.
- TU (R2007+): BS count of UTF-16 code units, then that many RS values. The count usually includes a terminating zero unit. Strip trailing zeros. Lone surrogates occur in damaged files and must be replaced, not passed through.
- In R2007+ objects, header and classes, every TV field is read from a separate string stream (9.3), not from the main data stream.
- T16: RS length, then length bytes (R2004 and earlier) or length UTF-16 units (R2007+).
- T32: RL size in bytes, then the bytes; in R2007+ files the bytes are UTF-16LE (size / 2 units).

Strings may contain the escape `\U+XXXX` (4 hex digits) for a Unicode character outside the code page, and `\M+NXXXX` for a multibyte character (N = code page digit, XXXX = two bytes in hex). Both should be decoded.

### 3.11 Colors

CMC (R13-R2000): BS color index (ACI). 0 = BYBLOCK, 256 = BYLAYER, 1-255 palette, negative for an "off" layer.

CMC (R2004+):

| Field | Type | Meaning |
|---|---|---|
| index | BS | ACI index, may be 0 and should be ignored if the method byte says "RGB" |
| rgb | BL | high byte = color method, low 24 bits = RGB |
| flags | RC | bit 0: color name follows, bit 1: book name follows |
| name | TV | if flags bit 0 |
| book name | TV | if flags bit 1 |

Color method (high byte of rgb): 0xC0 BYLAYER, 0xC1 BYBLOCK, 0xC2 true color (RGB in low 24 bits), 0xC3 ACI (index in low byte), 0xC5 foreground, 0xC8 none.

ENC (entity color, R2004+): BS raw value. Color index = raw & 0x1FF; flags = raw >> 8. Then, depending on flags:

| Flag bit | What follows |
|---|---|
| 0x20 | BL transparency: high byte is the type (0 BYLAYER, 1 BYBLOCK, 3 explicit), low byte is the alpha 0-255 |
| 0x40 | the color is a reference to a DBCOLOR object; the handle is in the handle stream (see 10.2) |
| 0x80 (and 0x40 clear) | BL with RGB in the low 24 bits (the high byte is the color method) |
| 0x41 both set | TV color name |
| 0x42 both set | TV color book name |

The fields are read in the order shown. See open question Q2 about the order of transparency and RGB.

### 3.12 Dates

TIMEBLL: BL Julian day number, then BL milliseconds since midnight. TIMERLL: the same as two RL. Convert to a date with: Julian day 2440588 is 1970-01-01. Durations (`$TDINDWG`, `$TDUSRTIMER`, summary "total editing time") use the same encoding with a day count instead of a Julian day.

### 3.13 Drawing code page

The RS at file offset `0x13` is the drawing code page. Values and their meaning:

| Value | Code page | Value | Code page |
|---|---|---|---|
| 0 | undefined (treat as UTF-8 or ANSI_1252) | 23 | Macintosh Roman |
| 1 | US ASCII | 24 | Big5 |
| 2 | ISO-8859-1 | 25 | CP949 (Korean) |
| 3 | ISO-8859-2 | 26 | Johab |
| 4 | ISO-8859-3 | 27 | CP866 |
| 5 | ISO-8859-4 | 28 | ANSI_1250 |
| 6 | ISO-8859-5 | 29 | ANSI_1251 |
| 7 | ISO-8859-6 | 30 | ANSI_1252 |
| 8 | ISO-8859-7 | 31 | GB2312 |
| 9 | ISO-8859-8 | 32 | ANSI_1253 |
| 10 | ISO-8859-9 | 33 | ANSI_1254 |
| 11 | CP437 | 34 | ANSI_1255 |
| 12 | CP850 | 35 | ANSI_1256 |
| 13 | CP852 | 36 | ANSI_1257 |
| 14 | CP855 | 37 | ANSI_874 |
| 15 | CP857 | 38 | ANSI_932 (Shift-JIS) |
| 16 | CP860 | 39 | ANSI_936 (GBK) |
| 17 | CP861 | 40 | ANSI_949 |
| 18 | CP863 | 41 | ANSI_950 |
| 19 | CP864 | 42 | ANSI_1361 |
| 20 | CP865 | 43 | UTF-16 (R2007+) |
| 21 | CP869 | 44 | ANSI_1258 |
| 22 | CP932 | | |

For R2007+ all strings are UTF-16LE regardless of this value; the value still records the original code page. In R13-R2004 files, 30 (ANSI_1252) is by far the most common.

### 3.14 Sentinels

16-byte constants that mark section boundaries in R13-R2000 and at the start of several R2004+ sections:

| Use | Bytes |
|---|---|
| Header variables start | CF 7B 1F 23 FD DE 38 A9 5F 7C 68 B8 4E 6D 33 5F |
| Header variables end | 30 84 E0 DC 02 21 C7 56 A0 83 97 47 B1 92 CC A0 |
| Classes start | 8D A1 C4 B8 C4 A9 F8 C5 C0 DC F4 5F E7 CF B6 8A |
| Classes end | 72 5E 3B 47 3B 56 07 3A 3F 23 0B A0 18 30 49 75 |
| File header (section locators) end | 95 A0 4E 28 99 82 1A E5 5E 41 E0 5F 9D 3A 4D 00 |
| Preview start | 1F 25 6D 07 D4 36 28 28 9D 57 CA 3F 9D 44 10 2B |
| Preview end | E0 DA 92 F8 2B C9 D7 D7 62 A8 35 C0 62 BB EF D4 |
| Second header start (R13-R2000) | D4 7B 21 CE 28 93 9F BF 53 24 40 09 12 3C AA 01 |
| Second header end (R13-R2000) | 2B 84 DE 31 D7 6C 60 40 AC DB BF F6 ED C3 55 FE |

### 3.15 Checksums

CRC-16: the reflected CRC-16 with polynomial 0x8005 (lookup table built from 0xA001 by the usual right-shifting method; the first table entries are 0x0000, 0xC0C1, 0xC181, 0x0140). For each byte b: `idx = (crc XOR b) AND 0xFF; crc = (crc >> 8) XOR table[idx]`. There is no final XOR. The start value ("seed") is 0xC0C1 for every CRC that a reader needs (file header locators, header variables, classes, object map chunks, objects). The CRC value is stored as RS (little-endian), except in the object map where it is big-endian.

CRC-32 (R2004+ file header): the standard CRC-32 (reflected polynomial 0xEDB88320, initial value all ones, final complement), computed with seed 0.

Page checksum (R2004+ page headers): an Adler-32 style sum with modulus 0xFFF1. Start with `s1 = seed AND 0xFFFF`, `s2 = seed >> 16`. Process the data in chunks of at most 0x15B0 bytes; for every byte `s1 += byte; s2 += s1`; after each chunk `s1 %= 0xFFF1; s2 %= 0xFFF1`. Result `(s2 << 16) OR s1`.

R2007 uses two 64-bit CRC variants and a 32-bit data checksum on the container level. They only verify integrity; a reader can skip them (see 6.8).

---

## 4. Family A: R13, R14, R2000 file layout

### 4.1 Overall order

```
file header with section locators, CRC, sentinel
[R13 before C3: preview image]
header variables section (sentinels)
classes section (sentinels)
[R13C3+: padding, 0x200 bytes, may contain the template data]
objects (each object stands alone, addressed by the object map)
object map ("AcDb:Handles")
[optional: object free space]
[R14-R2000 optional: template]
second header (sentinels)
[R13C3+: preview image (sentinels)]
[auxiliary header, R2000]
```

Never rely on this physical order; always use the addresses from the locator records.

### 4.2 File header

| Offset | Type | Meaning |
|---|---|---|
| 0x00 | 6 bytes | version string |
| 0x06 | 5 bytes | zero (R14: 4 zero bytes and ...) |
| 0x0B | RC | maintenance release version |
| 0x0C | RC | 0, 1 or 3 |
| 0x0D | RL | address of the preview image start sentinel |
| 0x11 | RC | version byte of the writing application |
| 0x12 | RC | maintenance byte of the writing application |
| 0x13 | RS | drawing code page (3.13) |
| 0x15 | RL | number of section locator records (3 to 6) |
| 0x19 | records | number x { RC record number, RL address, RL size } |
| ... | RS | CRC-16 seed 0xC0C1 over bytes 0x00 up to (not including) this CRC |
| ... | SN | file header end sentinel |

One source describes the CRC as "seed 0, then XOR with a constant that depends on the record count (3: 0xA598, 4: 0x8101, 5: 0x3CC4, 6: 0x8461)". Both descriptions produce the same value for the fixed header length; using seed 0xC0C1 is simpler.

Locator record numbers:

| Number | Section |
|---|---|
| 0 | header variables (address of the start sentinel, size including both sentinels) |
| 1 | classes (same) |
| 2 | object map |
| 3 | object free space (no sentinels); its presence means R13C3+ layout |
| 4 | template (contains `$MEASUREMENT`), optional |
| 5 | auxiliary header (no sentinels), R2000 |

Check each record: address + size must not exceed the file size.

### 4.3 Header variables section (R13-R2000)

```
SN  header variables start sentinel
RL  size of the following data in bytes
... bit-coded header variables (chapter 8)
RS  CRC-16 seed 0xC0C1 over the size field and the data
SN  header variables end sentinel
```

The CRC sits at (address + 16 + 4 + size). The data is padded to a byte boundary before the CRC.

### 4.4 Classes section (R13-R2000)

```
SN  classes start sentinel
RL  size of class data in bytes
... class records until the size is used up
RS  CRC-16 seed 0xC0C1 over size field and data
SN  classes end sentinel
```

Class record (R13-R2000):

| Field | Type | DXF | Meaning |
|---|---|---|---|
| class number | BS | | the object type value used by objects of this class (500 and up) |
| proxy flags | BS | 90 | capabilities, see 7.2 |
| application name | TV | 3 | e.g. "ObjectDBX Classes" |
| C++ class name | TV | 2 | e.g. "AcDbRasterImage" |
| DXF class name | TV | 1 | e.g. "IMAGE"; this name decides how the object is decoded |
| was a zombie | B | 280 | 1 if the class is a proxy (application not loaded when saved) |
| item class id | BS | 281 | 0x1F2 = class makes entities, 0x1F3 = class makes objects |

Section size on disk = data size + 38 (two sentinels, RL, RS).

### 4.5 Object map (R13-R2000)

The object map lists, for every object, its handle and its absolute file offset. It is a sequence of chunks:

```
RS (big-endian)  chunk size in bytes, including these 2 bytes, excluding the CRC
repeat until chunk size is used up:
    unsigned MC  handle delta (added to the previous handle in this chunk)
    signed MC    offset delta (added to the previous offset in this chunk)
RS (big-endian)  CRC-16 seed 0xC0C1 over the chunk (size field and pairs)
```

At the start of each chunk, "previous handle" and "previous offset" are 0. The last chunk has size 2 (only the size field) and ends the map. Chunks are at most about 2032 to 2040 bytes; reject anything larger than 2048 as corrupt.

In R13-R2000 the offsets are absolute file positions. Each offset points to the MS size field of an object (chapter 9).

### 4.6 Template, second header, auxiliary header

Template section (locator 4): T16 description (normally empty), RS `$MEASUREMENT` (0 English, 1 Metric). Some R13C3+ files store the measurement value in the first bytes of the 0x200 padding area instead.

Second header: a redundant copy of the locator records and of 14 important handles (handseed, the table control objects, the named objects dictionary, ...), framed by its sentinels. A reader does not need it; it can serve as a fallback when the main locators are damaged.

Auxiliary header (R2000 locator 5): same content as the R2004+ `AcDb:AuxHeader` section (11.6).

---

## 5. Families B and D: R2004, R2010, R2013, R2018 file layout

### 5.1 Overview

The file consists of a 0x100 byte file header area, followed by pages. Every page starts on a 0x20 byte boundary. Two "system" pages (the page map and the section map) describe where all "data" pages are and which logical section they belong to. A logical section (for example `AcDb:AcDbObjects`) is the concatenation of its pages after decompression.

### 5.2 File header (plain part, 0x00-0x7F)

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 6 | version string |
| 0x06 | 5 | zero |
| 0x0B | 1 | maintenance release version |
| 0x0C | 1 | 0, 1 or 3 |
| 0x0D | 4 | preview address: file offset of the preview page + 0x20 |
| 0x11 | 1 | version byte of the writing application |
| 0x12 | 1 | maintenance byte of the writing application |
| 0x13 | 2 | drawing code page (3.13) |
| 0x15 | 3 | zero (one source reads further application version bytes here) |
| 0x18 | 4 | security flags (5.10) |
| 0x1C | 4 | unknown, 0 |
| 0x20 | 4 | summary info address: page offset + 0x20 |
| 0x24 | 4 | VBA project address, 0 if none |
| 0x28 | 4 | always 0x80 |
| 0x2C | 0x54 | zero |
| 0x80 | 0x6C | encrypted file header data (5.3) |
| 0xEC | 0x14 | filler bytes |

### 5.3 Encrypted file header data (0x80, 0x6C bytes)

XOR the 0x6C bytes with a key stream produced by this generator:

1. `r = 1` (32-bit unsigned arithmetic, wrapping).
2. For each byte: `r = r * 0x343FD + 0x269EC3`; the key byte is `(r >> 16) AND 0xFF`.

The first key bytes are `29 23 BE 84 E1 6C D6 AE`. Decrypted layout:

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 12 | "AcFssFcAJMB" followed by a zero byte |
| 0x0C | 4 | 0 |
| 0x10 | 4 | 0x6C |
| 0x14 | 4 | 4 |
| 0x18 | 4 | root tree node gap |
| 0x1C | 4 | lowermost left tree node gap |
| 0x20 | 4 | lowermost right tree node gap |
| 0x24 | 4 | unknown, 1 |
| 0x28 | 4 | last page id |
| 0x2C | 8 | last page end address (relative, add 0x100) |
| 0x34 | 8 | address of the second (end of file) copy of this header data |
| 0x3C | 4 | gap count |
| 0x40 | 4 | page count |
| 0x44 | 4 | 0x20 |
| 0x48 | 4 | 0x80 |
| 0x4C | 4 | 0x40 |
| 0x50 | 4 | page map page id |
| 0x54 | 8 | page map address; file offset = value + 0x100 |
| 0x5C | 4 | section map page id |
| 0x60 | 4 | page array size (highest page id) |
| 0x64 | 4 | gap array size |
| 0x68 | 4 | CRC-32 (seed 0) over all 0x6C bytes with this field set to 0 |

A mismatch of the ID string means the file is not a valid R2004+ file (or is damaged). A CRC mismatch should be reported but reading can continue.

### 5.4 System page header

Both system pages start with a 20-byte plain header:

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 4 | page type: 0x41630E3B page map, 0x4163003B section map |
| 0x04 | 4 | decompressed size |
| 0x08 | 4 | compressed size |
| 0x0C | 4 | compression type, 2 |
| 0x10 | 4 | page checksum |

The compressed data follows directly. Checksum: page checksum (3.15) over the 20 header bytes with the checksum field zeroed, seed 0; then page checksum over the compressed data, seeded with the first result.

### 5.5 Page map

Found at the page map address (5.3). Decompress (5.8) to get a list of records:

```
RL page number (signed)
RL page size in bytes (the extent of the page in the file)
if page number < 0 (a gap / free block), 16 more bytes follow:
    RL parent, RL left, RL right, RL 0
```

The file offset of each page is computed by summing: the first page starts at 0x100, each next page starts at the previous start + the previous size. The map also contains the system pages themselves. Build a lookup "page number to file offset and size".

If the page map address points past the end of the file, the page can be found by scanning backwards from the end of the file in 0x20 steps for a 4-byte value 0x41630E3B.

### 5.6 Section map

Look up the section map page id in the page map. The page starts with a system page header of type 0x4163003B. Decompress. Layout:

```
RL number of section descriptions
RL 2 (compressed)
RL 0x7400 (max page size)
RL 0 (encrypted)
RL number of section descriptions (again)
repeat number times:
    RLL  section size (logical, decompressed)
    RL   page count (pages present in the file)
    RL   max decompressed page size (normally 0x7400)
    RL   unknown
    RL   compressed (1 = no, 2 = yes)
    RL   section id
    RL   encrypted (0 no, 1 yes, 2 unknown)
    64 bytes section name, ASCII, zero padded
    repeat page count times:
        RL   page number (look up in the page map)
        RL   data size (stored bytes of this page)
        RLL  start offset of this page inside the logical section
```

Sections a reader cares about:

| Name | Usually compressed | Page size | Content |
|---|---|---|---|
| (empty name) | yes | 0x7400 | empty section, id 0 |
| AcDb:Security | no | 0x7400 | password information (11.7) |
| AcDb:FileDepList | no | 0x80 | external file dependencies (11.4) |
| AcDb:VBAProject | no | | embedded VBA project |
| AcDb:AppInfo | no | 0x80 | writing application (11.3) |
| AcDb:Preview | no | 0x400 | thumbnail (11.5) |
| AcDb:SummaryInfo | no | 0x100 | title, author, ... (11.2) |
| AcDb:RevHistory | yes | 0x7400 | revision history (content mostly unknown) |
| AcDb:AcDbObjects | yes | 0x7400 | all objects |
| AcDb:ObjFreeSpace | yes | 0x7400 | free space bookkeeping |
| AcDb:Template | yes | 0x7400 | `$MEASUREMENT` |
| AcDb:Handles | yes | 0x7400 | object map |
| AcDb:Classes | yes | 0x7400 | class definitions |
| AcDb:AuxHeader | yes | 0x7400 | auxiliary header |
| AcDb:Header | yes | 0x7400 | header variables |
| AcDb:AcDsPrototype_1b | yes | | R2013+ data store (ACIS solids); not needed for text knowledge |
| AcDb:Signature | | | digital signature, rarely present |

Always take the page size from the section description; do not assume 0x7400.

### 5.7 Data page header and assembling a section

Each data page starts with a 32-byte header that is XOR-masked. Let `a` be the absolute file offset of the page (from the page map), taken as 32-bit. Mask = `0x4164536B XOR a`. XOR each of the eight 32-bit little-endian words of the header with the mask. Decoded:

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 4 | page type, always 0x4163043B |
| 0x04 | 4 | section id |
| 0x08 | 4 | data size: stored bytes after the header (compressed size, or full page size if not compressed) |
| 0x0C | 4 | page size (extent in the file, multiple of 0x20) |
| 0x10 | 8 | start offset inside the logical section (some readers treat the upper 4 bytes as unknown, always 0) |
| 0x18 | 4 | page header checksum (page checksum over the decoded header with this field zeroed, seeded with the data checksum) |
| 0x1C | 4 | data checksum (page checksum over the stored data, seed 0) |

Procedure to build a logical section:

1. Allocate `page count x max page size` bytes (zeroed); reject if this exceeds a sane limit (14).
2. For each page record in the order given by the section map: read and unmask the page header, check the page type, then either decompress `data size` bytes (compressed sections) or copy the stored bytes (uncompressed sections) to the start offset.
3. Pages that contain only zeros may be omitted from the file. If a page's start offset is beyond the end of the data produced so far, the gap is zero bytes.
4. The logical size is the section size from the section map; ignore bytes beyond it (the last page is padded).

### 5.8 R2004 compression (LZ77 variant)

Input: compressed bytes. Output: a buffer of known maximum size. Back references copy from earlier output, byte by byte (source and destination may overlap; overlapping copies repeat earlier output, like a run).

Definitions:

- Literal length L(op): let `v = op AND 0x0F`. If v is not 0, the length is `v + 3`. If v is 0: read bytes; each 0x00 byte adds 0xFF; the first non-zero byte b ends the sequence and adds b; then add 0x0F and finally 3. (Example: `00 02` gives 0x0F + 2 + 3 = 0x14.)
- Long count C(op, mask): let `v = op AND mask`. If v is not 0, the count is `v + 2`. If v is 0: read bytes; each 0x00 adds 0xFF; the first non-zero byte b adds b and ends; then add mask and 2.
- Two-byte offset: read bytes `b1`, `b2`; `offset = (b1 >> 2) OR (b2 << 6)`; the literal count that follows the copy is `b1 AND 3`.

Procedure:

1. Read the first byte `op`. If `(op AND 0xF0) == 0`, it is a literal length: copy L(op) bytes from input to output, then read the next byte as `op`.
2. Loop until `op == 0x11` (end marker), or input or output is exhausted:
   - If `op >= 0x40` (or `op < 0x10`, which does not occur in valid data): copy count = `(op >> 4) - 1`; read `b2`; distance = `((op >> 2) AND 3) OR (b2 << 2)` plus 1; literal count after the copy = `op AND 3`.
   - If `0x12 <= op <= 0x1F` or `op == 0x10`: copy count = C(op, 7); then a two-byte offset; distance = two-byte offset + 0x4000, plus another 0x4000 if bit 3 of `op` is set; literal count from the two-byte offset.
   - If `0x20 <= op <= 0x3F`: copy count = C(op, 0x1F); then a two-byte offset; distance = two-byte offset + 1; literal count from the two-byte offset.
   - Copy `count` bytes from (output position - distance) to the output position.
   - If the literal count is 0: read the next byte `op`; if `(op AND 0xF0) == 0` it is a literal length L(op): copy that many literal bytes and read the next `op`; otherwise `op` is already the next opcode.
   - If the literal count is 1 to 3: copy that many literal bytes, then read the next `op`.
3. Validate every copy: distance must not exceed the output position, and the copy must not run past the end of the output.

### 5.9 Families B/D: where things are

| Data | Section |
|---|---|
| Header variables | AcDb:Header (chapter 8) |
| Classes | AcDb:Classes (7.1) |
| Object map | AcDb:Handles (same format as 4.5; offsets are relative to the start of AcDb:AcDbObjects) |
| Objects | AcDb:AcDbObjects; the section starts with RL 0x0DCA; offsets in the map already account for it |
| Summary info | AcDb:SummaryInfo (11.2) |
| Preview | AcDb:Preview (11.5) |
| Measurement | AcDb:Template (4.6 format) |

### 5.10 Security flags and encryption

The RL at 0x18:

| Bit | Meaning |
|---|---|
| 0x01 | data sections are encrypted (all except Preview and SummaryInfo) |
| 0x02 | properties are encrypted (Preview and SummaryInfo) |
| 0x10 | file is digitally signed |
| 0x20 | signature has a time stamp |

Password-protected drawings set bit 0x01 (and often 0x02), have an `AcDb:Security` section, and mark sections as encrypted in the section map (encrypted = 1). The content is encrypted with a key derived from the password; without the password the content cannot be read. A reader must detect this and report "password protected, content not readable". Signatures (0x10) do not hinder reading.

---

## 6. Family C: R2007 file layout

### 6.1 Overview

| Area | Offset | Size |
|---|---|---|
| plain file header | 0x00 | 0x80 |
| Reed-Solomon coded file header | 0x80 | 0x400 |
| pages | 0x480 | rest |

Pages are addressed by id through a page map. A section map lists sections and their pages. Both maps are "system pages" (6.4). Section data pages are "data pages" (6.6).

### 6.2 Plain header (0x00-0x7F)

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 6 | "AC1021" |
| 0x06 | 5 | zero |
| 0x0B | 1 | maintenance release version |
| 0x0C | 1 | 0, 1 or 3 |
| 0x0D | 4 | preview address |
| 0x11 | 1 | writer version byte |
| 0x12 | 1 | writer maintenance byte |
| 0x13 | 2 | drawing code page |
| 0x15 | 3 | zero |
| 0x18 | 4 | security flags (5.10) |
| 0x1C | 4 | unknown |
| 0x20 | 4 | summary info address |
| 0x24 | 4 | VBA project address, 0 if none |
| 0x28 | 4 | 0x80 |
| 0x2C | 4 | application info address |

### 6.3 Coded file header (0x80, 0x400 bytes)

1. Read 0x3D8 bytes at 0x80. They contain 3 Reed-Solomon (255,239) code words, interleaved: byte j of code word i is at position `i + 3 * j`. Collect the 239 data bytes of each code word (the 16 parity bytes follow the data bytes in each code word). Optional: correct errors with the parity bytes (6.7).
2. The 3 x 239 = 717 data bytes start with:

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 8 | CRC |
| 0x08 | 8 | key |
| 0x10 | 8 | CRC of the compressed data |
| 0x18 | 4 | compressed length (signed); if negative, the data is stored uncompressed and the length is the absolute value |
| 0x1C | 4 | second length |
| 0x20 | n | data |

3. Decompress the data with the R2007 algorithm (6.5) to 0x110 bytes (or copy if stored uncompressed). The result is the file header:

| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 8 | header size (0x70) |
| 0x08 | 8 | file size |
| 0x10 | 8 | page map CRC (compressed) |
| 0x18 | 8 | page map correction factor |
| 0x20 | 8 | page map CRC seed |
| 0x28 | 8 | page map 2 offset (relative to 0x480) |
| 0x30 | 8 | page map 2 id |
| 0x38 | 8 | page map offset (relative to 0x480) |
| 0x40 | 8 | page map id |
| 0x48 | 8 | header 2 offset (relative to 0x480) |
| 0x50 | 8 | page map size, compressed |
| 0x58 | 8 | page map size, uncompressed |
| 0x60 | 8 | page count |
| 0x68 | 8 | highest page id |
| 0x70 | 8 | unknown (0x20) |
| 0x78 | 8 | unknown (0x40) |
| 0x80 | 8 | page map CRC (uncompressed) |
| 0x88 | 8 | unknown (0xF800) |
| 0x90 | 8 | unknown (4) |
| 0x98 | 8 | unknown (1) |
| 0xA0 | 8 | section count + 1 |
| 0xA8 | 8 | section map CRC (uncompressed) |
| 0xB0 | 8 | section map size, compressed |
| 0xB8 | 8 | section map 2 id |
| 0xC0 | 8 | section map id |
| 0xC8 | 8 | section map size, uncompressed |
| 0xD0 | 8 | section map CRC (compressed) |
| 0xD8 | 8 | section map correction factor |
| 0xE0 | 8 | section map CRC seed |
| 0xE8 | 8 | stream version (0x60100) |
| 0xF0 | 8 | CRC seed |
| 0xF8 | 8 | encoded CRC seed |
| 0x100 | 8 | random seed |
| 0x108 | 8 | header CRC-64 |

The last 0x28 bytes of the 0x400 area are check data for the header; they can be ignored by a reader. A copy of the coded header is at the end of the file.

### 6.4 System pages and the two maps

A system page is described by (compressed size c, uncompressed size u, correction factor f) from the file header.

1. Pre-coded size `p = (c rounded up to a multiple of 8) * f`.
2. Code word count `n = ceil(p / 239)`.
3. Bytes on disk: `n * 255` rounded up to a multiple of 8.
4. De-interleave: byte j of code word i is at `i + n * j`; take the first 239 bytes of each code word.
5. Take the first c bytes. If c < u, decompress (6.5) to u bytes; else copy u bytes.

The data repeats f times inside the page (redundancy); only the first copy is needed.

Page map: at file offset `0x480 + page map offset`. The decoded content is a list of records `RLL size, RLL id` (signed id). Page offsets are cumulative: the first page starts at 0x480, each next at previous start + previous size. Records with a negative id describe free space. Keep a lookup "id to file offset and size".

Section map: look up the section map id in the page map; decode the system page at that offset with the section map sizes and correction factor. The content is a list of section records until the data is used up:

```
RLL data size (logical section size)
RLL max page size
RLL encryption (0 none, 1 encrypted)
RLL hash code
RLL name length (see below)
RLL unknown
RLL encoding (1 = not interleaved, 4 = interleaved)
RLL page count
name: UTF-16LE, name length bytes
repeat page count times:
    RLL offset inside the logical section
    RLL page size (on disk)
    RLL page id (look up in the page map)
    RLL uncompressed size
    RLL compressed size
    RLL checksum
    RLL CRC
```

The name length is the byte length of the name including a two-byte terminator, as observed in practice; see open question Q7.

Known sections and hash codes (useful to recognize sections when names are damaged):

| Name | Hash | Page size | Encoding |
|---|---|---|---|
| AcDb:Security | 0x4A0204EA | 0xF800 | 1 |
| AcDb:FileDepList | 0x6C4205CA | varies | 1 |
| AcDb:VBAProject | 0x586E0544 | varies | 1 |
| AcDb:AppInfo | 0x3FA0043E | 0x300 | 1 |
| AcDb:Preview | 0x40AA0473 | varies | 1 |
| AcDb:SummaryInfo | 0x717A060F | 0x80 | 1 |
| AcDb:RevHistory | 0x60A205B3 | 0x1000 | 4 |
| AcDb:AcDbObjects | 0x674C05A9 | 0xF800 | 4 |
| AcDb:ObjFreeSpace | 0x77E2061F | 0xF800 | 4 |
| AcDb:Template | 0x4A1404CE | 0x400 | 4 |
| AcDb:Handles | 0x3F6E0450 | 0xF800 | 4 |
| AcDb:Classes | 0x3F54045F | 0xF800 | 4 |
| AcDb:AuxHeader | 0x54F0050A | 0x800 | 4 |
| AcDb:Header | 0x32B803D9 | 0x800 | 4 |

### 6.5 R2007 compression

Back references copy byte by byte from earlier output (overlap allowed). Literal runs are copied from the input but in a permuted byte order (see the table below).

Definitions:

- Literal length from an opcode `op` (only used when `op` is below 0x10): `len = op + 8`. If `len == 0x17`: read a byte n and add it; if n was 0xFF, then repeatedly read a 16-bit little-endian value m and add it, while m == 0xFFFF.
- Instruction decoding from an opcode `op` (updates `op`, yields a copy length and a distance), by the high nibble `h = op >> 4`:
  - h = 0: `length = (op AND 0x0F) + 0x13`; read `b`; read new `op`; `length += (op >> 3) AND 0x10`; `distance = ((op AND 0x78) << 5) + 1 + b`.
  - h = 1: `length = (op AND 0x0F) + 3`; read `b`; read new `op`; `distance = ((op AND 0xF8) << 5) + 1 + b`.
  - h = 2: read `b1`, `b2`; `distance = b1 OR (b2 << 8)`; `length = op AND 7`; if `(op AND 8) == 0`: read new `op`; `length += op AND 0xF8`. Otherwise: `distance += 1`; read `b`; `length += b << 3`; read new `op`; `length += ((op AND 0xF8) << 8) + 0x100`.
  - h >= 3: `length = op >> 4`; `distance = op AND 0x0F`; read new `op`; `distance = ((op AND 0xF8) << 1) + distance + 1`.

Procedure:

1. Read `op`. If `(op AND 0xF0) == 0x20`: skip 2 bytes, read a byte, the first literal length is `that byte AND 7` (must not be 0). Otherwise the first literal length is 0 (computed in step 2 from `op`).
2. While input remains:
   1. If the literal length is 0, compute it from `op` (literal length rule above).
   2. Copy the literal run (permuted order, see below). If the input is exhausted, stop.
   3. Read `op`, decode an instruction.
   4. Repeat: copy `length` bytes from `distance` bytes back. Set the literal length to `op AND 7`. If it is not 0, or the input is exhausted, leave this inner loop (continue at 2.2 via 2.1). Otherwise read `op`. If `op >> 4 == 0`, leave the inner loop (the next literal length is computed from this `op`). If `op >> 4 == 0x0F`, set `op = op AND 0x0F`. Decode an instruction and repeat.

Literal byte order: a literal run is processed in blocks of 32 input bytes. Within a 32-byte block the output is input bytes 24-31, then 16-23, then 8-15, then 0-7 (8-byte groups in reverse order, bytes inside a group in order). The remaining 1 to 31 bytes are written in this order (numbers are positions inside the remainder):

| Rest | Output order of input positions |
|---|---|
| 1 | 0 |
| 2 | 1, 0 |
| 3 | 2, 1, 0 |
| 4 | 0, 1, 2, 3 |
| 5 | 4, 0, 1, 2, 3 |
| 6 | 5, 1, 2, 3, 4, 0 |
| 7 | 6, 5, 1, 2, 3, 4, 0 |
| 8 | 0, 1, 2, 3, 4, 5, 6, 7 |
| 9 | 8, 0-7 |
| 10 | 9, 1-8, 0 |
| 11 | 10, 9, 1-8, 0 |
| 12 | 8-11, 0-7 |
| 13 | 12, 8-11, 0-7 |
| 14 | 13, 9-12, 1-8, 0 |
| 15 | 14, 13, 9-12, 1-8, 0 |
| 16 | 8-15, 0-7 |
| 17 | 9-16, 8, 0-7 |
| 18 | 17, 9-16, 1-8, 0 |
| 19 | 18, 17, 16, 8-15, 0-7 |
| 20 | 16-19, 8-15, 0-7 |
| 21 | 20, 16-19, 8-15, 0-7 |
| 22 | 21, 20, 16-19, 8-15, 0-7 |
| 23 | 22, 21, 20, 16-19, 8-15, 0-7 |
| 24 | 16-23, 8-15, 0-7 |
| 25 | 17-24, 16, 8-15, 0-7 |
| 26 | 25, 17-24, 16, 8-15, 0-7 |
| 27 | 26, 25, 17-24, 16, 8-15, 0-7 |
| 28 | 24-27, 16-23, 8-15, 0-7 |
| 29 | 28, 24-27, 16-23, 8-15, 0-7 |
| 30 | 29, 28, 24-27, 16-23, 8-15, 0-7 |
| 31 | 30, 26-29, 18-25, 10-17, 2-9, 1, 0 |

("a-b" means positions a, a+1, ..., b in ascending order.) The 32-byte blocks come first, then the remainder; positions in the remainder are relative to its own start.

### 6.6 Data pages

For each page record of a section:

1. Find the page id in the page map; seek to its file offset; read "page size" bytes.
2. The page is Reed-Solomon coded if compressed size differs from uncompressed size, or if the page size equals `ceil(round8(compressed size) / 251) * 255` rounded up to a multiple of 32. Pages stored verbatim are only padded to 32.
3. For a coded page: code word count `n = ceil(round8(compressed size) / 251)`; byte j of code word i is at `i + n * j`; take the first 251 bytes of each code word; then take the first "compressed size" bytes.
4. If compressed size < uncompressed size, decompress (6.5) to the uncompressed size; else copy.
5. Place the result at "offset" in the section buffer (allocate the logical "data size" of the section).

A section with encryption 1 cannot be read without the password (5.10).

### 6.7 Reed-Solomon parameters

- Header and system pages: (255,239), corrects up to 8 byte errors per code word.
- Data pages: (255,251), corrects up to 2 byte errors per code word.
- Field generator polynomials (as published): x^8 + x^6 + x^5 + x^3 + 1 for (255,239) and x^8 + x^4 + x^3 + x^2 + 1 for (255,251); see open question Q21.

Error correction is optional for a reader: undamaged files decode correctly by de-interleaving and dropping the parity bytes.

### 6.8 R2007 checksums

The container uses 64-bit CRCs (one ECMA-182 based "normal" variant and one "mirrored" variant with its own table), a 32-bit data page checksum derived from the R2004 page checksum with a modified seed, and a pseudo-random generator for padding. None of these are needed to read content. Validating them is a later addition.

### 6.9 Sections in R2007

After the container is decoded, the logical sections have the same content as in families B/D, with R2007 bit coding (separate string stream for header, classes and objects).

---

## 7. Classes

### 7.1 Classes section R2004+ (and R2007)

```
SN   classes start sentinel
RL   size of class data in bytes
RL   R2010+ with maintenance version > 3, and all R2018: unknown (high part of a 64-bit size)
RL   R2007+: size of the class data in bits (end of data stream, used to find the string stream, 9.3)
BS   highest class number
RC   0
RC   0
B    1
repeat (highest class number - 499) times:
    BS  class number
    BS  proxy flags
    TV  application name        (R2007+: from the string stream)
    TV  C++ class name          (R2007+: from the string stream)
    TV  DXF class name          (R2007+: from the string stream)
    B   was a zombie
    BS  item class id (0x1F2 entity, 0x1F3 object)
    BL  number of instances of this class in the drawing (DXF 91)
    BS  DWG version of the class
    BS  maintenance version of the class
    BL  unknown (0)
    BL  unknown (0)
RS   CRC
SN   classes end sentinel
(R2004+: 8 unknown bytes)
```

A class count outside 0..5000 indicates corruption.

### 7.2 Proxy flags

| Bit | Meaning |
|---|---|
| 1 | erase allowed |
| 2 | transform allowed |
| 4 | color change allowed |
| 8 | layer change allowed |
| 16 | linetype change allowed |
| 32 | linetype scale change allowed |
| 64 | visibility change allowed |
| 128 | cloning allowed |
| 256 | lineweight change allowed |
| 512 | plot style name change allowed |
| 1024 | disables proxy warning dialog |
| 32768 | is an R13 format proxy |

### 7.3 Resolving object types

- Types below 500 are fixed (table in 9.5).
- Types 498 (0x1F2) and 499 (0x1F3) are proxy entities and proxy objects.
- Types 500 and above refer to the class whose "class number" equals the type. Use the class's DXF name to choose the decoder (for example "LWPOLYLINE", "HATCH", "IMAGE", "LAYOUT", "MULTILEADER", "ACAD_TABLE", "XRECORD", "DICTIONARYVAR", "GROUP", "SCALE", "FIELD", "MTEXT" is never a class). In practice class numbers are 500, 501, ... in list order; matching by number is safer than matching by position.
- Several types that have fixed numbers in the table (LWPOLYLINE 0x4D, HATCH 0x4E, XRECORD 0x4F, PLACEHOLDER 0x50, LAYOUT 0x52) normally appear as class-based types in real files. Accept both.
- A class type whose DXF name is unknown to the reader must be skipped using the object size (9.6). If the class item id is 0x1F2 the object is an entity and still has the common entity data (layer, owner) which can be read.

---

## 8. Header variables

### 8.1 Section framing

R13-R2000: see 4.3.

R2004: `AcDb:Header` section: SN start sentinel, RL size, data, RS CRC, SN end sentinel. Strings and handles are inline.

R2007+: the data, string and handle streams are separate:

```
SN   header start sentinel                         (bytes 0-15)
RL   size of data in bytes                         (bytes 16-19)
RL   R2010+ with maintenance version > 3, and all R2018: high part of size (bytes 20-23)
RL   size of the data stream in bits: B             (next 4 bytes)
...  data stream
```

Let `S` be the bit position of the start of the field that holds B (160 normally, 192 when the extra RL is present). Then:

- the handle stream starts at bit `S + B`;
- the string stream ends at bit `S + B - 1`, where a single "has strings" bit sits; the string stream itself is located with the rule of 9.3 (the size fields sit just before that bit).

All H fields in the list below are read from the handle stream in R2007+ (in order), except `$HANDSEED`, which is in the data stream. All TV fields are read from the string stream in R2007+ (in order, see the end of 8.2).

### 8.2 Field order

The header is one bit stream; every field must be read in order. Fields marked as unknown must still be read. Columns: version range, type, name. "Knowledge" fields are in bold.

| Versions | Type | Field |
|---|---|---|
| R2013+ | BLL | `$REQUIREDVERSIONS` |
| all | BD | unknown, 412148564080.0 |
| all | BD | unknown, 1.0 (R13-R2004 often 6.162483e-14) |
| all | BD | unknown, 1.0 |
| all | BD | unknown, 1.0 |
| R13-R2004 | TV x4 | unknown strings (unit names such as "m", "meter", "inch") |
| all | BL | unknown, 24 |
| all | BL | unknown, 0 |
| R13-R14 | BS | unknown, 0 |
| R13-R2000 | H | current viewport entity header (hard pointer) |
| all | B | `$DIMASO` |
| all | B | `$DIMSHO` |
| R13-R14 | B | `$DIMSAV` |
| all | B | `$PLINEGEN`, `$ORTHOMODE`, `$REGENMODE`, `$FILLMODE`, `$QTEXTMODE`, `$PSLTSCALE`, `$LIMCHECK` (7 bits in this order) |
| R13-R14 | B | `$BLIPMODE` |
| R2004+ | B | unknown |
| all | B | `$USRTIMER`, `$SKPOLY`, `$ANGDIR`, `$SPLFRAME` |
| R13-R14 | B | `$ATTREQ`, `$ATTDIA` |
| all | B | `$MIRRTEXT`, `$WORLDVIEW` |
| R13-R14 | B | `$WIREFRAME` |
| all | B | `$TILEMODE`, `$PLIMCHECK`, `$VISRETAIN` |
| R13-R14 | B | `$DELOBJ` |
| all | B | `$DISPSILH`, `$PELLIPSE` |
| all | BS | `$PROXYGRAPHICS` |
| R13-R14 | BS | `$DRAGMODE` |
| all | BS | `$TREEDEPTH` |
| all | BS | **`$LUNITS`** |
| all | BS | `$LUPREC` |
| all | BS | **`$AUNITS`** |
| all | BS | `$AUPREC` |
| R13-R14 | BS | `$OSMODE` |
| all | BS | `$ATTMODE` |
| R13-R14 | BS | `$COORDS` |
| all | BS | `$PDMODE` |
| R13-R14 | BS | `$PICKSTYLE` |
| R2004+ | BL x3 | unknown |
| all | BS | `$USERI1` to `$USERI5`, `$SPLINESEGS`, `$SURFU`, `$SURFV`, `$SURFTYPE`, `$SURFTAB1`, `$SURFTAB2`, `$SPLINETYPE`, `$SHADEDGE`, `$SHADEDIF`, `$UNITMODE`, `$MAXACTVP`, `$ISOLINES`, `$CMLJUST`, `$TEXTQLTY` (19 values in this order) |
| all | BD | `$LTSCALE`, `$TEXTSIZE`, `$TRACEWID`, `$SKETCHINC`, `$FILLETRAD`, `$THICKNESS`, `$ANGBASE`, `$PDSIZE`, `$PLINEWID`, `$USERR1` to `$USERR5`, `$CHAMFERA`, `$CHAMFERB`, `$CHAMFERC`, `$CHAMFERD`, `$FACETRES`, `$CMLSCALE`, `$CELTSCALE` (23 values) |
| R13-R2004 | TV | `$MENUNAME` |
| all | TIMEBLL | **`$TDCREATE`** (creation date, 3.12; see Q4) |
| all | TIMEBLL | **`$TDUPDATE`** (last save date) |
| R2004+ | BL x3 | unknown |
| all | TIMEBLL | **`$TDINDWG`** (total editing time, as days and ms) |
| all | TIMEBLL | `$TDUSRTIMER` |
| all | CMC | `$CECOLOR` |
| all | H (code 0, data stream) | **`$HANDSEED`** (next free handle; an upper bound for all handles) |
| all | H | **`$CLAYER`** (current layer) |
| all | H | `$TEXTSTYLE` |
| all | H | `$CELTYPE` |
| R2007+ | H | `$CMATERIAL` |
| all | H | `$DIMSTYLE` |
| all | H | `$CMLSTYLE` |
| R2000+ | BD | `$PSVPSCALE` |
| all | 3BD | `$PINSBASE` |
| all | 3BD | **`$PEXTMIN`** (paper space extents) |
| all | 3BD | **`$PEXTMAX`** |
| all | 2RD | `$PLIMMIN` |
| all | 2RD | `$PLIMMAX` |
| all | BD | `$PELEVATION` |
| all | 3BD | `$PUCSORG`, `$PUCSXDIR`, `$PUCSYDIR` |
| all | H | `$PUCSNAME` |
| R2000+ | H | `$PUCSORTHOREF` |
| R2000+ | BS | `$PUCSORTHOVIEW` |
| R2000+ | H | `$PUCSBASE` |
| R2000+ | 3BD x6 | `$PUCSORGTOP`, `$PUCSORGBOTTOM`, `$PUCSORGLEFT`, `$PUCSORGRIGHT`, `$PUCSORGFRONT`, `$PUCSORGBACK` |
| all | 3BD | `$INSBASE` |
| all | 3BD | **`$EXTMIN`** (model space extents) |
| all | 3BD | **`$EXTMAX`** |
| all | 2RD | **`$LIMMIN`** |
| all | 2RD | **`$LIMMAX`** |
| all | BD | `$ELEVATION` |
| all | 3BD | `$UCSORG`, `$UCSXDIR`, `$UCSYDIR` |
| all | H | `$UCSNAME` |
| R2000+ | H | `$UCSORTHOREF` |
| R2000+ | BS | `$UCSORTHOVIEW` |
| R2000+ | H | `$UCSBASE` |
| R2000+ | 3BD x6 | `$UCSORGTOP` ... `$UCSORGBACK` (same order as paper space) |
| R2000-R2004 | TV | `$DIMPOST` |
| R2000-R2004 | TV | `$DIMAPOST` |
| R13-R14 | B | `$DIMTOL`, `$DIMLIM`, `$DIMTIH`, `$DIMTOH`, `$DIMSE1`, `$DIMSE2`, `$DIMALT`, `$DIMTOFL`, `$DIMSAH`, `$DIMTIX`, `$DIMSOXD` |
| R13-R14 | RC | `$DIMALTD`, `$DIMZIN` |
| R13-R14 | B | `$DIMSD1`, `$DIMSD2` |
| R13-R14 | RC | `$DIMTOLJ`, `$DIMJUST`, `$DIMFIT` |
| R13-R14 | B | `$DIMUPT` |
| R13-R14 | RC | `$DIMTZIN`, `$DIMALTZ`, `$DIMALTTZ`, `$DIMTAD` |
| R13-R14 | BS | `$DIMUNIT`, `$DIMAUNIT`, `$DIMDEC`, `$DIMTDEC`, `$DIMALTU`, `$DIMALTTD` |
| R13-R14 | H | `$DIMTXSTY` |
| all | BD | `$DIMSCALE`, `$DIMASZ`, `$DIMEXO`, `$DIMDLI`, `$DIMEXE`, `$DIMRND`, `$DIMDLE`, `$DIMTP`, `$DIMTM` |
| R2007+ | BD | `$DIMFXL`, `$DIMJOGANG` |
| R2007+ | BS | `$DIMTFILL` |
| R2007+ | CMC | `$DIMTFILLCLR` |
| R2000+ | B | `$DIMTOL`, `$DIMLIM`, `$DIMTIH`, `$DIMTOH`, `$DIMSE1`, `$DIMSE2` |
| R2000+ | BS | `$DIMTAD`, `$DIMZIN`, `$DIMAZIN` |
| R2007+ | BS | `$DIMARCSYM` |
| all | BD | `$DIMTXT`, `$DIMCEN`, `$DIMTSZ`, `$DIMALTF`, `$DIMLFAC`, `$DIMTVP`, `$DIMTFAC`, `$DIMGAP` |
| R13-R14 | T | `$DIMPOST`, `$DIMAPOST`, `$DIMBLK`, `$DIMBLK1`, `$DIMBLK2` |
| R2000+ | BD | `$DIMALTRND` |
| R2000+ | B | `$DIMALT` |
| R2000+ | BS | `$DIMALTD` |
| R2000+ | B | `$DIMTOFL`, `$DIMSAH`, `$DIMTIX`, `$DIMSOXD` |
| all | CMC | `$DIMCLRD`, `$DIMCLRE`, `$DIMCLRT` |
| R2000+ | BS | `$DIMADEC`, `$DIMDEC`, `$DIMTDEC`, `$DIMALTU`, `$DIMALTTD`, `$DIMAUNIT`, `$DIMFRAC`, `$DIMLUNIT`, `$DIMDSEP`, `$DIMTMOVE`, `$DIMJUST` |
| R2000+ | B | `$DIMSD1`, `$DIMSD2` |
| R2000+ | BS | `$DIMTOLJ`, `$DIMTZIN`, `$DIMALTZ`, `$DIMALTTZ` |
| R2000+ | B | `$DIMUPT` |
| R2000+ | BS | `$DIMATFIT` |
| R2007+ | B | `$DIMFXLON` |
| R2010+ | B | `$DIMTXTDIRECTION` |
| R2010+ | BD | `$DIMALTMZF` |
| R2010+ | BD | `$DIMMZF` |
| R2000+ | H | `$DIMTXSTY`, `$DIMLDRBLK`, `$DIMBLK`, `$DIMBLK1`, `$DIMBLK2` |
| R2007+ | H | `$DIMLTYPE`, `$DIMLTEX1`, `$DIMLTEX2` |
| R2000+ | BS | `$DIMLWD`, `$DIMLWE` |
| all | H | **BLOCK control object** (hard owner) |
| all | H | **LAYER control object** |
| all | H | **STYLE control object** |
| all | H | **LTYPE control object** |
| all | H | VIEW control object |
| all | H | UCS control object |
| all | H | VPORT control object |
| all | H | **APPID control object** |
| all | H | **DIMSTYLE control object** |
| R13-R2000 | H | viewport entity header control object |
| all | H | **dictionary ACAD_GROUP** |
| all | H | dictionary ACAD_MLINESTYLE |
| all | H | **named objects dictionary (root dictionary)** |
| R2000+ | BS | `$TSTACKALIGN` (default 1) |
| R2000+ | BS | `$TSTACKSIZE` (default 70) |
| R2000-R2004 | TV | **`$HYPERLINKBASE`** |
| R2000-R2004 | TV | `$STYLESHEET` |
| R2000+ | H | **dictionary ACAD_LAYOUT** |
| R2000+ | H | dictionary ACAD_PLOTSETTINGS |
| R2000+ | H | dictionary ACAD_PLOTSTYLENAME |
| R2004+ | H | dictionary ACAD_MATERIAL |
| R2004+ | H | dictionary ACAD_COLOR |
| R2007+ | H | dictionary ACAD_VISUALSTYLE |
| R2013+ | H | unknown dictionary |
| R2000+ | BL | flags, see below |
| R2000+ | BS | **`$INSUNITS`** |
| R2000+ | BS | `$CEPSNTYPE` |
| R2000+ | H | `$CPSNID`, only if `$CEPSNTYPE` == 3 |
| R2000-R2004 | TV | **`$FINGERPRINTGUID`** |
| R2000-R2004 | TV | **`$VERSIONGUID`** |
| R2004+ | RC | `$SORTENTS`, `$INDEXCTL`, `$HIDETEXT`, `$XCLIPFRAME`, `$DIMASSOC`, `$HALOGAP` |
| R2004+ | BS | `$OBSCOLOR`, `$INTERSECTIONCOLOR` |
| R2004+ | RC | `$OBSLTYPE`, `$INTERSECTIONDISPLAY` |
| R2004 | TV | **`$PROJECTNAME`** |
| all | H | block record *PAPER_SPACE |
| all | H | **block record *MODEL_SPACE** |
| all | H | linetype BYLAYER, BYBLOCK, CONTINUOUS (3 handles) |
| R2007+ | B | `$CAMERADISPLAY` |
| R2007+ | BL x2 | unknown |
| R2007+ | BD | unknown |
| R2007+ | BD | `$STEPSPERSEC`, `$STEPSIZE`, `$3DDWFPREC`, `$LENSLENGTH`, `$CAMERAHEIGHT` |
| R2007+ | RC | `$SOLIDHIST`, `$SHOWHIST` |
| R2007+ | BD | `$PSOLWIDTH`, `$PSOLHEIGHT`, `$LOFTANG1`, `$LOFTANG2`, `$LOFTMAG1`, `$LOFTMAG2` |
| R2007+ | BS | `$LOFTPARAM` |
| R2007+ | RC | `$LOFTNORMALS` |
| R2007+ | BD | **`$LATITUDE`**, **`$LONGITUDE`**, `$NORTHDIRECTION` |
| R2007+ | BL | `$TIMEZONE` |
| R2007+ | RC | `$LIGHTGLYPHDISPLAY`, `$TILEMODELIGHTSYNCH`, `$DWFFRAME`, `$DGNFRAME` |
| R2007+ | B | `$REALWORLDSCALE` |
| R2007+ | CMC | `$INTERFERECOLOR` |
| R2007+ | H | `$INTERFEREOBJVS`, `$INTERFEREVPVS`, `$DRAGVS` |
| R2007+ | RC | `$CSHADOW` |
| R2007+ | BD | `$SHADOWPLANELOCATION` |
| R14+ | BS x4 | unknown |
| R2007+ | string stream | (see below) |
| all | RS | CRC |

R2000+ flags BL (`$CELWEIGHT` and friends):

| Mask | Variable |
|---|---|
| 0x001F | `$CELWEIGHT` as a lineweight index (8.3) |
| 0x0060 | `$ENDCAPS` (non-zero = set) |
| 0x0180 | `$JOINSTYLE` |
| 0x0200 | `$LWDISPLAY` is the inverse of this bit |
| 0x0400 | `$XEDIT` is the inverse of this bit |
| 0x0800 | `$EXTNAMES` |
| 0x2000 | `$PSTYLEMODE` |
| 0x4000 | `$OLESTARTUP` |

R2007+ string stream order (each one TU): the 4 unknown unit strings, `$MENUNAME`, `$DIMPOST`, `$DIMAPOST`, then R2010+ only `$DIMALTMZS`, `$DIMMZS`, then `$HYPERLINKBASE`, `$STYLESHEET`, `$FINGERPRINTGUID`, `$VERSIONGUID`, `$PROJECTNAME`. See open question Q3.

`$MEASUREMENT` is not in this section; it is in the template section (4.6, 5.9).

### 8.3 Meaning of the knowledge variables

| Variable | Values |
|---|---|
| `$INSUNITS` | 0 unitless, 1 inches, 2 feet, 3 miles, 4 millimeters, 5 centimeters, 6 meters, 7 kilometers, 8 microinches, 9 mils, 10 yards, 11 angstroms, 12 nanometers, 13 microns, 14 decimeters, 15 decameters, 16 hectometers, 17 gigameters, 18 astronomical units, 19 light years, 20 parsecs, 21 US survey feet, 22 US survey inch, 23 US survey yard, 24 US survey mile |
| `$LUNITS` | 1 scientific, 2 decimal, 3 engineering, 4 architectural, 5 fractional (6 = Windows desktop, only for dimension styles) |
| `$AUNITS` | 0 decimal degrees, 1 degrees/minutes/seconds, 2 gradians, 3 radians, 4 surveyor's units |
| `$ANGDIR` | 0 counterclockwise, 1 clockwise |
| `$MEASUREMENT` | 0 English (imperial), 1 Metric |
| `$EXTMIN`/`$EXTMAX` | model space extents in WCS; a value of +1e20 / -1e20 means "not computed" (empty or never regenerated drawing) |
| `$TDCREATE`, `$TDUPDATE` | Julian day + milliseconds (3.12) |
| `$TDINDWG` | days + milliseconds of editing time |
| `$HANDSEED` | next handle that will be given out; every object handle must be smaller |
| `$FINGERPRINTGUID` | GUID set at creation, identifies the drawing across copies |
| `$VERSIONGUID` | GUID that changes with every save |
| `$PROJECTNAME` | project name used to find xrefs and images |
| `$HYPERLINKBASE` | base path for relative hyperlinks |

Lineweight index (used in layer flags and entity lineweight bytes): 0 to 23 map to 0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200, 211 hundredths of a millimeter; 29 = BYLAYER, 30 = BYBLOCK, 31 = DEFAULT. In the entity common data the lineweight is an RC that already holds this index.

---

## 9. Objects: common structure

### 9.1 Object prefix

Every object (table entry, entity, dictionary, ...) starts at the offset given by the object map:

| Versions | Type | Meaning |
|---|---|---|
| all | MS | object size in bytes, counted from the first byte after this field (and after the next MC in R2010+) up to the CRC |
| R2010+ | unsigned MC | size of the handle stream in bits (includes the padding bits at the end) |
| all | OT | object type |
| R2000-R2007 | RL | size of the data stream in bits ("end bit"), counted from the start of the OT field |
| all | H | the object's own handle (code 0) |
| all | EED | extended entity data, see 9.4 |

The bit position "start of the OT field" (the first bit after MS, and after MC in R2010+) is position 0 for all positions inside the object.

Location of the handle stream (bit position H0):
- R13-R14: the RL "size in bits" field appears later (see 10.1/10.3), with the same meaning.
- R2000-R2007: H0 = the RL value.
- R2010+: H0 = object size x 8 - handle stream size.

The CRC (RS, seed 0xC0C1) is at byte (start of OT field) + object size, and covers all bytes from the first byte of the MS field to just before the CRC.

### 9.2 Three streams per object

- Data stream: from the type field up to H0 (R2007+: up to the string stream).
- Handle stream: from H0 to the end of the object data. All H fields of the object (except the object's own handle and the few "data handles" marked in this document) are read from this stream, in the order they appear in the object descriptions. In the descriptions below they are listed in a "Handles" block for clarity.
- String stream (R2007+ only): all TV fields of the object are read from it, in the order they appear in the description.

A reader keeps three independent read positions. Reading the data stream must stop at its end (do not read handles from the data stream).

### 9.3 String stream (R2007+)

For an object, let `E = H0 - 1` (the last bit of the data area). For the header and classes sections, `E` is the bit computed in 8.1.

1. Read the bit at E. If it is 0, the object has no strings.
2. Otherwise read an RS (16 raw bits) starting at bit `E - 16`. This is `L`.
3. If `L AND 0x8000` is set: read another RS starting at bit `E - 32`, call it `Hi`; `L = (L AND 0x7FFF) OR (Hi << 15)`; the size fields now start at `E - 32`. Otherwise they start at `E - 16`.
4. `L` is the size of the string stream in bits. The string stream starts at bit `(start of the size fields) - L`.
5. If `L` is larger than the object, treat the object as having no strings (corruption).

Object types that never contain strings may skip this step. Type codes 500 and up always check.

### 9.4 Extended entity data (EED)

Directly after the object's handle, in the data stream:

```
repeat:
    BS  size of this application's data in bytes (0 ends the EED)
    H   handle of the APPID table entry (registered application)
    size bytes of data items
```

The data bytes are read as raw bytes starting at the current bit position (not byte aligned in general). Each data item is one byte code (DXF group code minus 1000) followed by its value:

| Code | DXF | Value |
|---|---|---|
| 0 | 1000 | string. R13-R2004: RC length n, RS code page (stored most significant byte first, see Q9), n bytes. R2007+: RS length n, n UTF-16 units |
| 1 | 1001 | not used inside data (the application is given by the handle) |
| 2 | 1002 | RC: 0 = "{" open list, 1 = "}" close list |
| 3 | 1003 | layer reference, 8-byte handle (see Q8) |
| 4 | 1004 | binary chunk: RC length n, n bytes |
| 5 | 1005 | database handle, 8 bytes, most significant byte first |
| 10-15 | 1010-1015 | point: 3 RD (1010 point, 1011 world position, 1012 displacement, 1013 direction) |
| 40-42 | 1040-1042 | RD (1040 real, 1041 distance, 1042 scale factor) |
| 70 | 1070 | RS 16-bit integer |
| 71 | 1071 | RL 32-bit integer |

Any other code means the EED is damaged: skip to the end of this application's block (size is known). Resolve the application handle to an APPID name to report EED by application (for example "ACAD", "AcadAnnotative", "AcDbBlockRepBTag", vendor names).

### 9.5 Fixed object types

| Type | Name | Type | Name |
|---|---|---|---|
| 0x01 | TEXT | 0x2A | DICTIONARY |
| 0x02 | ATTRIB | 0x2B | OLEFRAME |
| 0x03 | ATTDEF | 0x2C | MTEXT |
| 0x04 | BLOCK | 0x2D | LEADER |
| 0x05 | ENDBLK | 0x2E | TOLERANCE |
| 0x06 | SEQEND | 0x2F | MLINE |
| 0x07 | INSERT | 0x30 | BLOCK control |
| 0x08 | MINSERT | 0x31 | BLOCK_HEADER (block record) |
| 0x0A | VERTEX (2D) | 0x32 | LAYER control |
| 0x0B | VERTEX (3D) | 0x33 | LAYER |
| 0x0C | VERTEX (mesh) | 0x34 | STYLE control |
| 0x0D | VERTEX (pface) | 0x35 | STYLE |
| 0x0E | VERTEX (pface face) | 0x38 | LTYPE control |
| 0x0F | POLYLINE (2D) | 0x39 | LTYPE |
| 0x10 | POLYLINE (3D) | 0x3C | VIEW control |
| 0x11 | ARC | 0x3D | VIEW |
| 0x12 | CIRCLE | 0x3E | UCS control |
| 0x13 | LINE | 0x3F | UCS |
| 0x14 | DIMENSION (ordinate) | 0x40 | VPORT control |
| 0x15 | DIMENSION (linear) | 0x41 | VPORT |
| 0x16 | DIMENSION (aligned) | 0x42 | APPID control |
| 0x17 | DIMENSION (angular 3 point) | 0x43 | APPID |
| 0x18 | DIMENSION (angular 2 line) | 0x44 | DIMSTYLE control |
| 0x19 | DIMENSION (radius) | 0x45 | DIMSTYLE |
| 0x1A | DIMENSION (diameter) | 0x46 | viewport entity header control (R13-R2000) |
| 0x1B | POINT | 0x47 | viewport entity header (R13-R2000) |
| 0x1C | 3DFACE | 0x48 | GROUP |
| 0x1D | POLYLINE (pface) | 0x49 | MLINESTYLE |
| 0x1E | POLYLINE (mesh) | 0x4A | OLE2FRAME |
| 0x1F | SOLID | 0x4B | dummy |
| 0x20 | TRACE | 0x4C | LONG_TRANSACTION |
| 0x21 | SHAPE | 0x4D | LWPOLYLINE |
| 0x22 | VIEWPORT | 0x4E | HATCH |
| 0x23 | ELLIPSE | 0x4F | XRECORD |
| 0x24 | SPLINE | 0x50 | ACDBPLACEHOLDER |
| 0x25 | REGION | 0x51 | VBA_PROJECT |
| 0x26 | 3DSOLID | 0x52 | LAYOUT |
| 0x27 | BODY | 0x1F2 | ACAD_PROXY_ENTITY |
| 0x28 | RAY | 0x1F3 | ACAD_PROXY_OBJECT |
| 0x29 | XLINE | 500+ | class based (7.3) |

### 9.6 Skipping objects

Because the size of every object is known (MS) and the handle stream position is known, a reader can:
- skip unknown types entirely;
- stop reading an object after the fields it needs (it does not have to parse the rest of the data stream);
- read the handle stream of an object even when it does not understand all data fields, as long as it knows how many handles to expect (or reads handles until the end of the stream).

### 9.7 Walking all objects

1. Read the object map; it gives every (handle, offset) pair. Duplicated handles can occur; keep the last one.
2. Decode each object's prefix and type. Build a lookup handle to object.
3. Start from the header handles (table control objects, root dictionary, model and paper space block records) to find the "live" objects. Objects in the map that are not reachable from the header (orphans) exist in real files and are usually stale copies; a knowledge reader should mark them or ignore them.
4. Ownership: each object (except control objects) has an owner handle; entities in model space or paper space may omit it (entity mode, 10.1).

---

## 10. Entity and object descriptions

In the tables, "Handles" lists the handle stream content in order. "Common entity handles" and "common object handles" are defined in 10.2 and 10.3. The DXF column gives the DXF group code with the same meaning.

### 10.1 Common entity data

After the prefix (9.1, including EED), every entity has:

| Versions | Type | Field |
|---|---|---|
| all | B | graphics present flag |
| R13-R2007 | RL | graphics size in bytes (if flag) |
| R2010+ | BLL | graphics size in bytes (if flag) |
| all | bytes | graphics data (if flag); proxy entity graphics format, see 10.24 |
| R13-R14 | RL | size of data stream in bits (handle stream position) |
| all | BB | entity mode: 0 = owner handle present (entity inside a block definition, VERTEX, ATTRIB, SEQEND), 1 = paper space, 2 = model space |
| all | BL | number of persistent reactors |
| R13-R14 | B | 1 = linetype is BYLAYER (no linetype handle) |
| R2004+ | B | 1 = no extension dictionary handle |
| R13-R2000 | B | "no links": 1 = previous/next entity links are not stored |
| R2013+ | B | has data in the data store section (ACIS) |
| R13-R2000 | CMC | color (BS index) |
| R2004+ | ENC | color (3.11) |
| all | BD | linetype scale (DXF 48) |
| R2000+ | BB | linetype flags: 0 BYLAYER, 1 BYBLOCK, 2 CONTINUOUS, 3 handle follows in handle stream |
| R2000-R2004 | BB | plot style flags: same coding |
| R2007+ | BB | material flags: 0 BYLAYER, 1 BYBLOCK, 2 global, 3 handle follows |
| R2007+ | BB | plot style flags |
| R2007+ | RC | shadow flags (DXF 284): 0 casts and receives, 1 casts, 2 receives, 3 ignores |
| R2010+ | B | has full visual style |
| R2010+ | B | has face visual style |
| R2010+ | B | has edge visual style |
| all | BS | invisibility (bit 0 set = invisible, DXF 60) |
| R2000+ | RC | lineweight index (DXF 370, 8.3) |

The order of material and plot style flags in R2007+ is uncertain, see Q1.

### 10.2 Common entity handles

At the start of the handle stream of every entity:

| Versions | Handle | Condition |
|---|---|---|
| R2004+ | DBCOLOR reference | only if the ENC flags have 0x40 (see Q2) |
| all | owner (soft pointer, DXF 330) | only if entity mode is 0 |
| all | reactors (soft pointers) | number of reactors |
| all | extension dictionary (hard owner, DXF 360) | R13-R2000 always; R2004+ only if the "no xdictionary" bit is 0 |
| R13-R14 | layer (hard pointer, DXF 8) | |
| R13-R14 | linetype (DXF 6) | if not BYLAYER |
| R13-R2000 | previous entity, next entity | only if "no links" is 0 |
| R2000+ | layer (hard pointer, DXF 8) | |
| R2000+ | linetype (DXF 6) | if linetype flags = 3 |
| R2007+ | material (DXF 347) | if material flags = 3 |
| R2007+ | shadow | if shadow flags = 3 (rare) |
| R2000+ | plot style (DXF 390) | if plot style flags = 3 |
| R2010+ | full, face, edge visual style (DXF 348) | one each if the matching bit is set |

Entity-specific handles follow.

When entity mode is 1 or 2 the owner is the paper space or model space block record (header handles).

### 10.3 Common object data and handles (non-entities)

After the prefix and EED:

| Versions | Type | Field |
|---|---|---|
| R13-R14 | RL | size of data stream in bits |
| all | BL | number of reactors |
| R2004+ | B | 1 = no extension dictionary |
| R2013+ | B | has data store data |

Handle stream start:

| Handle | Condition |
|---|---|
| owner (soft pointer, DXF 330) | |
| reactors | number of reactors |
| extension dictionary (hard owner) | unless "no extension dictionary" |

Table entries (LAYER, STYLE, LTYPE, VIEW, UCS, VPORT, APPID, DIMSTYLE, BLOCK_HEADER) continue with the common table entry fields:

| Versions | Type | Field |
|---|---|---|
| all | TV | entry name (DXF 2) |
| R13-R2004 | B | "referenced" bit (DXF 70 bit 64) |
| all | BS | xref index + 1 (subtract 1; -1 = not from an xref). R2007+: the value 256 marks an xref-dependent entry |
| R13-R2004 | B | xref dependent (DXF 70 bit 16) |
| handles | H | external reference block record (hard pointer); null unless xref dependent |

Xref-dependent entries have names of the form `XREFNAME|ENTRYNAME`.

Control objects (BLOCK control, LAYER control, ...) have this layout instead:

| Type | Field |
|---|---|
| BL | number of entries (DXF 70) |
| Handles | owner (null), reactors, extension dictionary, then the entry handles (soft owner); BLOCK control adds *MODEL_SPACE and *PAPER_SPACE (hard owner) after the entries; LTYPE control adds BYLAYER and BYBLOCK |

Entry counts in control objects do not always include special entries (model and paper space, BYLAYER, BYBLOCK). Duplicate entry names occur in damaged files.

### 10.4 LAYER (0x33)

| Versions | Type | DXF | Field |
|---|---|---|---|
| | | | common table entry fields |
| R13-R14 | B | 70 bit 1 | frozen |
| R13-R14 | B | | off (also shown by a negative color) |
| R13-R14 | B | 70 bit 2 | frozen in new viewports |
| R13-R14 | B | 70 bit 4 | locked |
| R2000+ | BS | | flags: 1 frozen, 2 off, 4 frozen in new viewports, 8 locked, 16 plottable (DXF 290), bits 5-9 (mask 0x3E0) lineweight index (DXF 370) |
| all | CMC | 62 | color |
| Handles | | | xref block record (common), then R2000+ plot style (390), R2007+ material (347), all: linetype (6), R2013+ visual style (348) |

Layer state for knowledge: name, on/off, frozen, locked, plot, color (index, RGB, color book name), linetype name (resolve handle), lineweight. Layer "0" always exists; layer "Defpoints" is never plotted.

### 10.5 BLOCK_HEADER (block record, 0x31)

| Versions | Type | DXF | Field |
|---|---|---|---|
| | | | common table entry fields (name, DXF 2) |
| all | B | 70 bit 1 | anonymous |
| all | B | 70 bit 2 | has attribute definitions |
| all | B | 70 bit 4 | is an external reference (xref) |
| all | B | 70 bit 8 | xref is an overlay |
| R2000+ | B | 70 bit 32 | xref is unloaded (1 = not loaded) |
| R2004+ | BL | | number of owned entities; only present if not xref and not overlay |
| all | 3BD | 10 | base point |
| all | TV | 1 | xref path name (empty for normal blocks) |
| R2000+ | RC... | | insert count: a sequence of non-zero bytes ended by a zero byte; the number of non-zero bytes is the number of INSERT handles listed below |
| R2000+ | TV | 4 | block description |
| R2000+ | BL | | preview data size |
| R2000+ | bytes | 310 | preview data (a small binary image, usually BMP or PNG) |
| R2007+ | BS | 70 | insert units (same values as `$INSUNITS`) |
| R2007+ | B | 280 | explodable |
| R2007+ | RC | 281 | block scaling (0 any, 1 uniform) |
| Handles | | | xref block record (common, usually null), BLOCK entity (hard owner); R13-R2000: first entity and last entity (only if not xref/overlay); R2004+: owned entities (one per owned count); ENDBLK entity (hard owner); R2000+: INSERT handles (count from above, soft pointers); R2000+: LAYOUT (hard pointer, DXF 340) |

Notes:
- Names starting with `*` are anonymous blocks: `*Model_Space`, `*Paper_Space`, `*Paper_SpaceN` (layouts), `*D` (dimension graphics), `*U` (anonymous, often dynamic block variants or groups), `*T` (table graphics), `*X` (hatch), `*E` (and others). They are not parts in a parts list.
- The insert handle list exists only from R2000 on and lists INSERTs that reference this block. A parts-list count is more reliable when computed by scanning INSERT entities (10.8).
- Xrefs: "is xref" set and the xref path name gives the referenced file (absolute or relative path, may use the project search paths `$PROJECTNAME`). The `AcDb:FileDepList` section (11.4) also lists xrefs.

### 10.6 BLOCK (0x04), ENDBLK (0x05), SEQEND (0x06)

BLOCK: common entity data, TV block name (DXF 2), common entity handles. The block record holds the real information; the BLOCK entity name may be empty or `*`; use the block record name.

ENDBLK and SEQEND: common entity data and handles only. SEQEND closes the attribute list of an INSERT and the vertex list of a POLYLINE.

### 10.7 TEXT (0x01)

R13-R14:

| Type | DXF | Field |
|---|---|---|
| BD | 30 | elevation |
| 2RD | 10 | insertion point |
| 2RD | 11 | alignment point |
| 3BD | 210 | extrusion |
| BD | 39 | thickness |
| BD | 51 | oblique angle |
| BD | 50 | rotation |
| BD | 40 | height |
| BD | 41 | width factor |
| TV | 1 | **text value** |
| BS | 71 | generation flags (2 backwards, 4 upside down) |
| BS | 72 | horizontal alignment |
| BS | 73 | vertical alignment |

R2000+:

| Type | DXF | Field |
|---|---|---|
| RC | | data flags (a set bit means "field absent, use default") |
| RD | 30 | elevation, if not flags & 0x01 (default 0) |
| 2RD | 10 | insertion point |
| 2DD | 11 | alignment point, if not flags & 0x02; defaults are the insertion point x and y |
| BE | 210 | extrusion |
| BT | 39 | thickness |
| RD | 51 | oblique angle, if not flags & 0x04 (default 0) |
| RD | 50 | rotation, if not flags & 0x08 (default 0) |
| RD | 40 | height |
| RD | 41 | width factor, if not flags & 0x10 (default 1) |
| TV | 1 | **text value** |
| BS | 71 | generation, if not flags & 0x20 (default 0) |
| BS | 72 | horizontal alignment, if not flags & 0x40 (default 0) |
| BS | 73 | vertical alignment, if not flags & 0x80 (default 0) |

Handles: common entity handles, then text STYLE (hard pointer, DXF 7).

Text values may contain the control sequences of chapter 12.

### 10.8 INSERT (0x07) and MINSERT (0x08)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | 3BD | 10 | insertion point |
| R13-R14 | 3BD | 41/42/43 | scale x, y, z |
| R2000+ | BB | | scale flag, then: 3 = (1,1,1), nothing stored; 1 = x is 1.0, then DD y and DD z with default 1.0; 2 = RD x, y and z equal x; 0 = RD x, DD y (default x), DD z (default x) |
| all | BD | 50 | rotation (radians) |
| all | 3BD | 210 | extrusion |
| all | B | 66 | has attributes |
| R2004+ | BL | | number of owned attributes (present if has attributes; see Q12) |
| MINSERT only | BS | 70 | column count |
| MINSERT only | BS | 71 | row count |
| MINSERT only | BD | 44 | column spacing |
| MINSERT only | BD | 45 | row spacing |
| Handles | | | common entity handles; **block record** (hard pointer, DXF 2); if has attributes: R13-R2000 first and last ATTRIB; R2004+ one handle per owned attribute; then the SEQEND (hard owner) |

Notes for knowledge:
- The block name comes from the referenced block record.
- MINSERT represents rows x columns instances.
- Dynamic block references point to an anonymous `*U` block record; the original block is found through that block record's extended data (application "AcDbBlockRepBTag" with a 1005 handle to the original block record) or its extension dictionary; see Q14.

### 10.9 ATTRIB (0x02) and ATTDEF (0x03)

ATTRIB starts with exactly the TEXT fields (10.7, the text value is the **attribute value**). Then:

| Versions | Type | DXF | Field |
|---|---|---|---|
| R2010+ | RC | 280 | version (0) |
| R2018+ | RC | 70 (ATTDEF) | attribute type: 1 single line, 2 multi-line attribute, 4 multi-line attribute definition |
| R2018+ | ... | | if type > 1: embedded MTEXT data (see Q15) |
| all | TV | 2 | **tag** |
| all | BS | 73 | field length (unused) |
| all | RC | 70 | flags: 1 invisible, 2 constant, 4 verification required, 8 preset (not bit-pair coded) |
| R2007+ | B | 280 | lock position |
| R2010+ | RC | | keep duplicate records (XRECORD subclass flag) |
| Handles | | | common entity handles, text STYLE (hard pointer) |

ATTRIB entities are owned by an INSERT (entity mode 0, owner handle = the INSERT). They form the title block data of a drawing.

ATTDEF: the same as ATTRIB (its TEXT value is the **default value**), plus at the end of the data stream: TV **prompt** (DXF 3). In R2010+ the version byte is also present; in R2018+ the attribute type byte. ATTDEFs live inside block definitions and define the tags and prompts.

### 10.10 MTEXT (0x2C)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | 3BD | 10 | insertion point |
| all | 3BD | 210 | extrusion |
| all | 3BD | 11 | x axis direction |
| all | BD | 41 | reference rectangle width |
| R2007+ | BD | 46 | reference rectangle height |
| all | BD | 40 | text height |
| all | BS | 71 | attachment point: 1 top left, 2 top center, 3 top right, 4 middle left, 5 middle center, 6 middle right, 7 bottom left, 8 bottom center, 9 bottom right |
| all | BS | 72 | drawing direction: 1 left to right, 3 top to bottom, 5 by style |
| all | BD | | extents height |
| all | BD | | extents width |
| all | TV | 1 and 3 | **text** (the full contents with formatting codes, chapter 12) |
| R2000+ | BS | 73 | line spacing style |
| R2000+ | BD | 44 | line spacing factor |
| R2000+ | B | | unknown |
| R2004+ | BL | 90 | background flags: 1 fill, 2 fill with drawing background color, 0x10 text frame (R2018+) |
| R2004+ | BD | 45 | background scale (if flags & 1 in R2004-R2018) |
| R2004+ | CMC | 63 | background color (same condition) |
| R2004+ | BL | 441 | background transparency (same condition) |
| R2018+ | B | | is NOT annotative |
| R2018+ | | | if not annotative: BS version, B default flag, BL attachment, 3BD x axis direction, 3BD insertion point, BD rectangle width, BD rectangle height, BD extents width, BD extents height, BS column type (0 none, 1 static, 2 dynamic); if column type != 0: BL count (fragments for static, column heights for dynamic), BD column width, BD gutter, B auto height, B flow reversed, and if not auto height and dynamic columns: BD x count (column heights); this block also has an application handle in the handle stream |
| Handles | | | common entity handles, text STYLE (hard pointer, DXF 7); R2018+ annotative block: registered application (hard pointer) |

### 10.11 DIMENSION family (0x14 to 0x1A)

Common dimension data (after common entity data):

| Versions | Type | DXF | Field |
|---|---|---|---|
| R2010+ | RC | 280 | version (0) |
| all | 3BD | 210 | extrusion |
| all | 2RD | 11 | text middle point |
| all | BD | 31 | elevation of the ECS points |
| all | RC | | flags 1: bit 0 set = default text position not used (DXF 70 bit 128 is its inverse), bit 1 = DXF 70 bit 32 |
| all | TV | 1 | **user text (override)**: empty = show measurement, "<>" inside the text = placeholder for the measurement, a single space = no text |
| all | BD | 53 | text rotation |
| all | BD | 51 | horizontal direction |
| all | 3BD | 41/42/43 | insertion scale of the block |
| all | BD | 54 | insertion rotation |
| R2000+ | BS | 71 | attachment point |
| R2000+ | BS | 72 | line spacing style |
| R2000+ | BD | 41 | line spacing factor |
| R2000+ | BD | 42 | **actual measurement** (in drawing units, angles in radians) |
| R2007+ | B | 73 | unknown |
| R2007+ | B | 74 | flip arrow 1 |
| R2007+ | B | 75 | flip arrow 2 |
| all | 2RD | 12 | insertion point of the clone block |

Then per type:

| Type | Fields |
|---|---|
| ORDINATE 0x14 | 3BD definition point (10), 3BD feature location (13), 3BD leader end (14), RC flags 2 (bit 0 = x ordinate, DXF 70 bit 64) |
| LINEAR 0x15 | 3BD extension line 1 origin (13), 3BD extension line 2 origin (14), 3BD dimension line point (10), BD oblique angle (52), BD rotation (50) |
| ALIGNED 0x16 | 3BD (13), 3BD (14), 3BD (10), BD oblique angle (52) |
| ANGULAR 3 POINT 0x17 | 3BD (10), 3BD (13), 3BD (14), 3BD vertex (15) |
| ANGULAR 2 LINE 0x18 | 2RD arc point (16), 3BD (13), 3BD (14), 3BD (15), 3BD (10) |
| RADIUS 0x19 | 3BD center (10), 3BD chord point (15), BD leader length (40) |
| DIAMETER 0x1A | 3BD far chord point (15), 3BD (10), BD leader length (40) |

Handles: common entity handles, DIMSTYLE (hard pointer, DXF 3), anonymous block with the dimension graphics (hard pointer, DXF 2).

Class-based dimension types: ARC_DIMENSION (common dimension data, 3BD 10, 13, 14, 15, B is partial, BD start angle, BD end angle, B has leader, 3BD leader point 1, 3BD leader point 2) and LARGE_RADIAL_DIMENSION (common data, 3BD center, 3BD chord point, BD unknown, 3BD override center, 3BD jog point). Same two trailing handles.

The displayed text of a dimension: if the user text is empty, format the actual measurement with the dimension style (`DIMLUNIT`, `DIMDEC`, `DIMPOST`, ...). The anonymous `*D` block contains the rendered text as MTEXT; it is the most reliable source of what the drawing shows.

### 10.12 LEADER (0x2D)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | B | | unknown |
| all | BS | 73 | annotation type: 0 MTEXT, 1 TOLERANCE, 2 INSERT, 3 none |
| all | BS | 72 | path type: 0 straight, 1 spline |
| all | BL | 76 | number of points |
| all | 3BD x n | 10 | points |
| all | 3BD | | origin |
| all | 3BD | 210 | extrusion |
| all | 3BD | 211 | x direction |
| all | 3BD | 212 | offset to block insertion point |
| R13C3-R2007 | 3BD | | end point projection |
| R13-R14 | BD | | dimension gap |
| all | BD | 40 | text box height |
| all | BD | 41 | text box width |
| all | B | 74 | hook line on x direction |
| all | B | 71 | arrowhead on |
| R13-R14 | BS, BD, B, B, BS, BS, B, B | | arrowhead type, arrow size, unknowns, by-block color |
| R2000+ | BS, B, B | | arrowhead type, unknown, unknown |
| Handles | | | common entity handles, associated annotation (DXF 340, the MTEXT, TOLERANCE or INSERT), DIMSTYLE (DXF 3) |

The text of a leader is in the associated annotation object.

### 10.13 MULTILEADER (class "MULTILEADER")

| Versions | Type | DXF | Field |
|---|---|---|---|
| R2010+ | BS | 270 | version (2) |
| all | BL | | number of leader roots |
| per root | B, B | 290, 291 | has last leader line point, has dog leg |
| per root | 3BD | 10 | connection point (if the first flag) |
| per root | 3BD | 11 | dog leg direction (if the second flag) |
| per root | BL, then 3BD+3BD per break | 12, 13 | break start/end points |
| per root | BL | 90 | leader index |
| per root | BD | 40 | dog leg length |
| per root | BL | | number of leader lines; per line: BL point count, 3BD points, BL break count with 3BD pairs, BL line index (91), R2010+: BS type (170), CMC color (92), handle linetype (340), BL lineweight (171), BD arrow size (40), handle arrow block (341), BL override flags (93) |
| per root, R2010+ | BS | 271 | attachment direction (0 horizontal, 1 vertical) |
| all | BD | 40 | overall scale |
| all | 3BD | 10 | content base point |
| all | BD | 41 | text height |
| all | BD | 140 | arrow size |
| all | BD | 145 | landing gap |
| all | BS x4 | 174-177 | text left attachment, text right attachment, text alignment, attachment type |
| all | B | 290 | has text content; if set: TV **text label** (304), 3BD normal, handle text style, 3BD location, 3BD direction, BD rotation, BD width, BD height, BD line spacing factor, BS line spacing style, CMC color, BS alignment, BS flow, CMC background color, BD background scale, BL background transparency, B background fill, B background mask fill, BS column type, B auto height, BD column width, BD gutter, B flow reversed, BL count and BD column sizes, B word break, B unknown |
| all | B | 296 | otherwise: has block content; if set: handle block record (341), 3BD normal, 3BD location, 3BD scale, BD rotation, CMC color, 16 BD transformation matrix |
| all | 3BD x3 | 110-112 | base point, base direction, base vertical |
| all | B | 297 | normal reversed |
| R2010+ | BS, BS | 273, 272 | top and bottom text attachment |
| all | | | then the style override part: handle MLEADERSTYLE (340), BL override flags (90), BS leader type (170), CMC line color (91), handle line type (341), BL line weight (171), B landing enabled (290), B dog leg enabled (291), BD landing distance (41), handle arrow head (342), BD arrow size (42), BS content type (172: 1 block, 2 MTEXT, 3 tolerance), handle text style (343), BS text left attachment (173), BS text right attachment (95), BS text angle type (174), BS text alignment (175), CMC text color (92), B text frame (292), handle block (344), CMC block color (93), 3BD block scale (10), BD block rotation (43), BS block attachment (176), B annotative (293), R13-R2007: BL arrow head count with B+handle each, BL block label count with handle ATTDEF (330) + TV label text (302) + BS index (177) + BD width (44) each; B text direction negative (294), BS IPE alignment (178), BS justification (179), BD scale factor (45); R2010+: BS attachment direction (271), BS top attachment (273), BS bottom attachment (272); R2013+: B leader extended to text (295) |

Handles referenced by fields marked "handle" are read from the handle stream in field order. Block label values for block content multileaders are attribute values (like ATTRIB values). See open question Q13.

### 10.14 Lines and curves

| Entity | Fields (after common entity data) |
|---|---|
| LINE 0x13 | R13-R14: 3BD start (10), 3BD end (11). R2000+: B "z values are zero", RD start x, DD end x (default start x), RD start y, DD end y (default start y), if z not zero: RD start z, DD end z (default start z). Then BT thickness, BE extrusion |
| CIRCLE 0x12 | 3BD center (10), BD radius (40), BT thickness, BE extrusion |
| ARC 0x11 | 3BD center, BD radius, BT thickness, BE extrusion, BD start angle (50), BD end angle (51); angles in radians |
| POINT 0x1B | 3BD point, BT thickness, BE extrusion, BD x axis angle (50) |
| ELLIPSE 0x23 | 3BD center (WCS), 3BD major axis vector (WCS), 3BD extrusion, BD axis ratio (40), BD start parameter (41), BD end parameter (42) |
| RAY 0x28, XLINE 0x29 | 3BD point (10), 3BD direction (11) |
| SOLID 0x1F, TRACE 0x20 | BT thickness, BD elevation, 2RD x4 corners (10-13), BE extrusion |
| 3DFACE 0x1C | R13-R14: 3BD x4, BS invisible edges. R2000+: B "no flags", B "z is zero", RD x, RD y, [RD z], 3DD corner 2 (default corner 1), 3DD corner 3 (default 2), 3DD corner 4 (default 3), [BS invisible edges if not "no flags"] |

### 10.15 LWPOLYLINE (class, or 0x4D)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | BS | | flags: 4 constant width present, 8 elevation present, 2 thickness present, 1 extrusion present, 16 bulges present, 32 widths present, 512 closed, 256 linetype generation, 1024 vertex ids present (R2010+) |
| | BD | 43 | constant width (if flag 4) |
| | BD | 38 | elevation (if flag 8) |
| | BD | 39 | thickness (if flag 2) |
| | 3BD | 210 | extrusion (if flag 1) |
| all | BL | 90 | number of vertices |
| | BL | | number of bulges (if flag 16) |
| R2010+ | BL | | number of vertex ids (if flag 1024) |
| | BL | | number of width pairs (if flag 32) |
| R13-R14 | 2RD x n | 10 | vertices |
| R2000+ | 2RD, then (n-1) x 2DD | 10 | first vertex raw, each next vertex as DD x and DD y with the previous vertex as default |
| | BD x bulges | 42 | bulges |
| R2010+ | BL x ids | 91 | vertex ids |
| | 2BD x widths | 40/41 | start and end width per vertex |

Note: the DWG closed flag is 512, not 1 as in DXF.

### 10.16 POLYLINE (2D 0x0F, 3D 0x10, pface 0x1D, mesh 0x1E) and VERTEX

| Entity | Fields |
|---|---|
| POLYLINE 2D | BS flags (70), BS curve type (75), BD start width, BD end width, BT thickness, BD elevation, BE extrusion, R2004+ BL owned vertex count |
| POLYLINE 3D | RC curve flags (1: quadratic, 2: cubic spline fit), RC flags (bit 0 closed), R2004+ BL owned count |
| POLYLINE pface | BS vertex count (71), BS face count (72), R2004+ BL owned count |
| POLYLINE mesh | BS flags, BS curve type, BS M count, BS N count, BS M density, BS N density, R2004+ BL owned count |
| Handles | common, then R13-R2000 first and last VERTEX, R2004+ one handle per owned vertex, then SEQEND |
| VERTEX 2D 0x0A | RC flags (70), 3BD point (z is unused, use polyline elevation), BD start width (if negative, abs value is start and end width and no end width follows), BD end width, BD bulge, R2010+ BL vertex id, BD tangent direction |
| VERTEX 3D/mesh/pface | RC flags, 3BD point |
| VERTEX pface face 0x0E | BS x4 vertex indices (71-74, 1-based, negative = invisible edge) |

### 10.17 SPLINE (0x24)

| Versions | Type | Field |
|---|---|---|
| all | BL | scenario: 1 control points and knots, 2 fit points |
| R2013+ | BL | spline flags: 1 method fit points, 2 control frame shown, 4 closed, 8 use knot parameter |
| R2013+ | BL | knot parameterization: 0 chord, 1 square root, 2 uniform, 15 custom. Scenario becomes 2 if flag 1 is set, 1 if knot parameter is 15 |
| all | BL | degree (71) |
| scenario 1 | B rational, B closed, B periodic, BD knot tolerance, BD control tolerance, BL knot count, BL control point count, B weighted; then BD per knot; then per control point 3BD and, if weighted, BD weight |
| scenario 2 | BD fit tolerance, 3BD start tangent, 3BD end tangent, BL fit point count, 3BD per fit point |

### 10.18 HATCH (class "HATCH", or 0x4E)

| Versions | Type | DXF | Field |
|---|---|---|---|
| R2004+ | BL | 450 | is gradient |
| R2004+ | BL | 451 | reserved |
| R2004+ | BD | 460 | gradient angle |
| R2004+ | BD | 461 | gradient shift |
| R2004+ | BL | 452 | single color gradient |
| R2004+ | BD | 462 | gradient tint |
| R2004+ | BL | 453 | gradient color count; per color: BD (463), BS unknown, BL RGB (421), RC ignored |
| R2004+ | TV | 470 | gradient name |
| all | BD | 30 | elevation |
| all | 3BD | 210 | extrusion |
| all | TV | 2 | **pattern name** (for example "ANSI31", "SOLID") |
| all | B | 70 | solid fill |
| all | B | 71 | associative |
| all | BL | 91 | boundary path count |
| | | | paths: BL path flags (92); if not (flags & 2): BL edge count, per edge RC edge type (1 line: 2RD, 2RD; 2 arc: 2RD center, BD radius, BD start, BD end, B ccw; 3 ellipse: 2RD center, 2RD major axis end, BD ratio, BD start, BD end, B ccw; 4 spline: BL degree, B rational, B periodic, BL knot count, BL control count, BD knots, 2RD control points with BD weight if rational, R2010+: BL fit count, 2RD fit points, 2RD start tangent, 2RD end tangent); if (flags & 2) polyline path: B has bulges, B closed, BL vertex count, 2RD (+BD bulge) per vertex. Then BL boundary object count (97) |
| all | BS | 75 | hatch style: 0 odd parity, 1 outermost, 2 entire area |
| all | BS | 76 | pattern type: 0 user defined, 1 predefined, 2 custom |
| if not solid | BD | 52 | pattern angle |
| if not solid | BD | 41 | pattern scale or spacing |
| if not solid | B | 77 | double hatch |
| if not solid | BS | 78 | definition line count; per line: BD angle, 2BD base point, 2BD offset, BS dash count, BD per dash |
| | BD | 47 | pixel size, only if any path flag has bit 4 |
| all | BL | 98 | seed point count, 2RD per seed point |
| Handles | | | common entity handles, then all boundary object handles (DXF 330, total of all paths) |

### 10.19 VIEWPORT entity (0x22)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | 3BD | 10 | center (paper space) |
| all | BD | 40 | width |
| all | BD | 41 | height |
| R2000+ | 3BD | 17 | view target |
| R2000+ | 3BD | 16 | view direction |
| R2000+ | BD | 51 | twist angle |
| R2000+ | BD | 45 | view height (model space) |
| R2000+ | BD | 42 | lens length |
| R2000+ | BD | 43 | front clip |
| R2000+ | BD | 44 | back clip |
| R2000+ | BD | 50 | snap angle |
| R2000+ | 2RD | 12 | view center (model space) |
| R2000+ | 2RD | 13 | snap base |
| R2000+ | 2RD | 14 | snap spacing |
| R2000+ | 2RD | 15 | grid spacing |
| R2000+ | BS | 72 | circle zoom |
| R2007+ | BS | 61 | grid major |
| R2000+ | BL | | frozen layer count |
| R2000+ | BL | 90 | status flags |
| R2000+ | TV | 1 | style sheet |
| R2000+ | RC | 281 | render mode |
| R2000+ | B, B | 74, 71 | UCS at origin, UCS per viewport |
| R2000+ | 3BD x3, BD, BS | 110-112, 146, 79 | UCS origin, x axis, y axis, elevation, orthographic type |
| R2004+ | BS | 170 | shade plot mode |
| R2007+ | B, RC, BD, BD, CMC | 292, 282, 141, 142, 63 | default lights, lighting type, brightness, contrast, ambient color |
| Handles | | | common entity handles; R13-R14 viewport entity header; R2000+ frozen layer handles (341, one per count); clip boundary (340); R2000 viewport entity header; R2000+ named UCS (345), base UCS (346); R2007+ background (332), visual style (348), shade plot (333), sun (361) |

Scale of a viewport = view height (45) / paper height (41), or paper / model as `41 / 45`. The first viewport in each layout is the paper space view itself.

### 10.20 IMAGE (class "IMAGE") and IMAGEDEF (class "IMAGEDEF")

IMAGE:

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | BL | 90 | class version (0) |
| all | 3BD | 10 | insertion point |
| all | 3BD | 11 | u vector (one pixel width) |
| all | 3BD | 12 | v vector (one pixel height) |
| all | 2RD | 13 | image size in pixels |
| all | BS | 70 | display properties |
| all | B | 280 | clipping on |
| all | RC | 281, 282, 283 | brightness, contrast, fade |
| R2010+ | B | 290 | clip mode (0 outside, 1 inside) |
| all | BS | 71 | clip boundary type: 1 rectangle (2 points), 2 polygon |
| all | BL | 91 | clip vertex count (only for polygon) |
| all | 2RD x n | 14 | clip vertices |
| Handles | | | common entity handles, IMAGEDEF (hard pointer, DXF 340), IMAGEDEF_REACTOR (hard owner, DXF 360) |

IMAGEDEF (an object):

| Type | DXF | Field |
|---|---|---|
| BL | 90 | class version |
| 2RD | 10 | image size in pixels |
| TV | 1 | **file path** of the raster image |
| B | 280 | is loaded |
| RC | 281 | resolution units: 0 none, 2 centimeters, 5 inches |
| 2RD | 11 | size of one pixel in drawing units |

Image definitions are listed in the root dictionary entry "ACAD_IMAGE_DICT" (name to IMAGEDEF). Other attached files use similar definition objects with a file path: PDFDEFINITION, DWFDEFINITION, DGNDEFINITION (underlays, DXF 1 = path, 2 = page or sheet name); see Q16.

### 10.21 DICTIONARY (0x2A) and XRECORD (class "XRECORD")

DICTIONARY:

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | BL | | entry count |
| R14 | RC | 280 | hard owner flag (only R13C3 and later R14 writers) |
| R2000+ | BS | 281 | duplicate record cloning flag |
| R2000+ | RC | 280 | hard owner flag |
| all | TV x count | 3 | entry names |
| Handles | | | common object handles, then one handle per entry (DXF 350 soft owner or 360 hard owner) |

DICTIONARYWDFLT: as DICTIONARY plus a default entry handle (hard pointer) at the end.

Important root dictionary entries (root = `named objects dictionary` from the header): ACAD_GROUP, ACAD_LAYOUT, ACAD_MLINESTYLE, ACAD_PLOTSETTINGS, ACAD_PLOTSTYLENAME, ACAD_IMAGE_DICT, ACAD_MATERIAL, ACAD_COLOR, ACAD_TABLESTYLE, ACAD_MLEADERSTYLE, ACAD_SCALELIST, ACAD_FIELDLIST, AcDbVariableDictionary (custom drawing variables as DICTIONARYVAR), and application entries (vendor data, often XRECORDs).

DICTIONARYVAR: RC schema number, TV value (DXF 1). Entries in AcDbVariableDictionary are named variables (for example "CANNOSCALE"); vendors also store custom variables there.

XRECORD:

| Versions | Type | Field |
|---|---|---|
| all | BL | data size in bytes |
| all | bytes | data items, see below (byte-oriented, read at the current bit position) |
| R2000+ | BS | duplicate record cloning flag (DXF 280) |
| Handles | | common object handles, then any number of handles until the end of the handle stream |

Each data item: RS group code, then a value whose type is given by the DXF group code ranges:

| Group codes | Value |
|---|---|
| 0-9, 100-102, 300-309, 410-419, 430-439, 470-479, 999, 1000-1009 | string: R13-R2004 RS length + RC code page + bytes; R2007+ RS length + UTF-16 units |
| 10-39, 110-139, 210-239, 1010-1039 | point: 3 RD |
| 40-59, 140-149, 460-469, 1040-1042 | RD |
| 60-79, 170-179, 270-279, 370-389, 400-409, 1070 | RS |
| 90-99, 420-429, 440-459, 1071 | RL |
| 160-169 | RLL |
| 280-289 | RC |
| 290-299 | RC as boolean |
| 310-319, 1004 | binary: RC length + bytes |
| 5, 105, 320-369, 390-399 | handle: 8 bytes |

Stop at an unknown group code or when the data size is used up.

### 10.22 LAYOUT (class "LAYOUT", or 0x52)

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | TV | 1 | page setup name |
| all | TV | 2 | printer or plot configuration file |
| all | BS | 70 | plot layout flags |
| all | BD x4 | 40-43 | left, bottom, right, top margin (mm) |
| all | BD x2 | 44, 45 | paper width and height (mm) |
| all | TV | 4 | paper size (canonical media name) |
| all | 2BD | 46, 47 | plot origin |
| all | BS | 72 | paper units: 0 inches, 1 millimeters, 2 pixels |
| all | BS | 73 | plot rotation: 0 none, 1 90 degrees, 2 180, 3 270 |
| all | BS | 74 | plot type |
| all | 2BD | 48, 49 | plot window lower left |
| all | 2BD | 140, 141 | plot window upper right |
| R13-R2000 | TV | 6 | plot view name (R2004+: a handle in the handle stream) |
| all | BD | 142 | real world units (scale numerator) |
| all | BD | 143 | drawing units (scale denominator) |
| all | TV | 7 | current style sheet (plot style table) |
| all | BS | 75 | standard scale type |
| all | BD | 147 | scale factor |
| all | 2BD | 148, 149 | paper image origin |
| R2004+ | BS x3 | 76, 77, 78 | shade plot mode, resolution level, custom DPI |
| all | TV | 1 | **layout name** (for example "Model", "Layout1", "A1 Sheet") |
| all | BS | 71 | **tab order** (0 = model) |
| all | BS | 70 | layout flags |
| all | 3BD | 12 or 13 | insertion base / UCS origin (see Q10) |
| all | 2RD | 10 | limits min |
| all | 2RD | 11 | limits max |
| all | 3BD | 13 or 12 | UCS origin / insertion base (see Q10) |
| all | 3BD | 16 | UCS x axis |
| all | 3BD | 17 | UCS y axis |
| all | BD | 146 | elevation |
| all | BS | 76 | UCS orthographic type |
| all | 3BD | 14 | extents min |
| all | 3BD | 15 | extents max |
| R2004+ | BL | | viewport count |
| Handles | | | common object handles; R2004+ plot view (6); R2007+ visual style; then **block record of this layout** (330), last active viewport (331), base UCS (346), named UCS (345); R2004+ viewport handles (one per count) |

Layouts are listed in the ACAD_LAYOUT dictionary. The block record of the layout holds its entities: `*Model_Space` for the model layout, `*Paper_Space` for the active paper layout, `*Paper_SpaceN` for the others.

### 10.23 GROUP (0x48 or class "GROUP")

| Type | DXF | Field |
|---|---|---|
| TV | 300 | description |
| BS | 70 | unnamed (1 = anonymous) |
| BS | 71 | selectable |
| BL | | member count |
| Handles | | common object handles, one hard pointer per member (DXF 340) |

The group name is the entry name in the ACAD_GROUP dictionary.

### 10.24 Proxy entities and objects

ACAD_PROXY_ENTITY (type 498) and ACAD_PROXY_OBJECT (type 499), and any class-based object whose class has "was a zombie" set, carry data of an application that was not available when the file was saved.

| Versions | Type | DXF | Field |
|---|---|---|---|
| all | BL | 90 | proxy class id (498 or 499) |
| R13-R2013 | BL | 95 | drawing format: version in the low byte, maintenance version in the next byte (see Q11) |
| R2018+ | BL, BL | 71, 97 | version, maintenance version |
| R2000+ | B | 70 | original data format: 0 DWG, 1 DXF |
| all | bits | 310 | the application's data, up to the handle stream |
| Handles | | | common handles, then any number of object references until the end |

For proxy entities, the graphics data in the common entity data (10.1) contains a drawable approximation. Its format: a sequence of chunks `RL size, RL type, data`, data padded to 4 bytes. Useful chunk types for knowledge: 1 extents (2 x 3RD), 10 text (3RD position, 3RD normal, 3RD direction, RD height, RD width factor, RD oblique, zero-terminated padded string in the code page), 11 text 2 (as 10 plus explicit length, raw flag, flags and font names), 36 Unicode text (as 10 with a UTF-16 zero-terminated padded string), 38 Unicode text 2. Other types are geometry (2 circle, 4 arc, 6 polyline, 7 polygon, 8 mesh, 9 shell, 12 xline, 13 ray, 32 polyline with normal, 33 LWPOLYLINE data) and attribute changes (14 color, 16 layer index, 18 linetype index, 22 true color, 23 lineweight, ...). A knowledge reader should extract text strings from proxy graphics and report the proxy class (application name, C++ class name, DXF name) as "content of an unknown application".

### 10.25 Other table entries

| Entry | Knowledge fields after the common table fields |
|---|---|
| STYLE 0x35 | B shape file flag (DXF 70 bit 1), B vertical (70 bit 4) (order see Q19), BD fixed height (40), BD width factor (41), BD oblique angle (50), RC generation (71), BD last height (42), TV **font file name** (3), TV big font file name (4). TrueType fonts often appear only in EED of application "ACAD": 1000 typeface name, 1071 flags (0x2000000 bold, 0x1000000 italic) |
| LTYPE 0x39 | TV **description** (3), BD pattern length (40), RC alignment (72, 'A'), RC dash count (73), then per dash BD length, BS shape number, RD x offset, RD y offset, BD scale, BD rotation, BS shape flags; R13-R2004: 256 bytes of text area; R2007+: 512 bytes text area only if a shape flag has bit 2. Handles: per dash the shape file STYLE |
| APPID 0x43 | RC unknown (71). The name is the registered application name used by EED |
| DIMSTYLE 0x45 | the `DIM*` values in nearly the same order as the header (8.2), then B unknown, handles for text style and arrow blocks. The **name** is usually enough for knowledge |
| VIEW 0x3D | view height, width, center, target, direction, twist, lens, clips, mode bits, R2000+ render mode, pspace flag, UCS data; the **name** identifies a named view |
| UCS 0x3F | origin, x and y direction, R2000+ elevation, orthographic types; **name** |
| VPORT 0x41 | viewport configuration; the active one is named "*Active" |

### 10.26 TABLE (class "ACAD_TABLE")

R2004 to R2007: the TABLE entity starts like an INSERT (insertion point, scale, rotation, extrusion, attributes, owned count) and continues with BS table value flags (90), 3BD horizontal direction (11), BL column count (92), BL row count (91), BD column widths, BD row heights, then per cell: BS cell type (1 text, 2 block), RC edge flags, B merged, B autofit, BL merged width, BL merged height, BD rotation, for text cells TV **cell text** (1), for block cells BD block scale, B has attributes (then BS count, per attribute BS index and TV **text**), B has overrides (then BL override flags and the overridden properties in flag order), R2007+ BL unknown and a value structure (10.27). Then table level override blocks. Handles: common, block record (anonymous `*T` block with the graphics), attribute and SEQEND handles, table style, per cell text style or block handles.

R2010+: after common entity data: RC unknown, handle (table style), BL unknown, R2010 only B unknown, R2013 BL unknown, then the table content (TABLECONTENT structure: TV name, TV description, BL column count with per column TV name, BL custom data, custom data items, cell style data, BL cell style id, BD width; BL row count with per row BL cell count and per cell BL state flags, TV tooltip, BL custom data, custom data items, BL has linked data, BL content count with per content BL type (1 value, 2 field, 4 block) and the value (10.27), field handle or block handle, BL attribute count with handle + TV value + BL index each, BS format override flag and override data, cell style data, BL cell style id, BL geometry flag and optional geometry; per row BL custom data, custom data items, cell style data, BL style id, BD height; then field reference handles, table cell style overrides, merged ranges; then BS unknown, 3BD horizontal direction, break data).

The R2010+ layout contains several nested variable structures that are not fully confirmed (Q17). A robust fallback for knowledge extraction: the table's anonymous block record (`*T...`, found through the TABLE's block record handle) contains the rendered cell texts as MTEXT entities placed in a grid; their positions give row and column.

### 10.27 Value structure (TABLE cells, FIELD)

| Versions | Type | Field |
|---|---|---|
| R2007+ | BL | flags (bit 0: general type) |
| all | BL | data type: 0 unknown, 1 long, 2 double, 4 string, 8 date, 16 2D point, 32 3D point, 64 object id, 128 buffer, 256 result buffer, 512 general |
| | | value: long BL; double BD; string TV; date BL size + bytes; 2D point BL size + 2RD; 3D point BL size + 3RD; object id = handle in the handle stream; general BL size + bytes |
| R2007+ | BL | unit type: 0 none, 1 distance, 2 angle, 4 area, 8 volume |
| R2007+ | TV | format string |
| R2007+ | TV | **value as formatted string** |

FIELD objects (class "FIELD") hold the field code (for example `%<\AcVar Filename>%`) and the evaluated value; MTEXT and ATTRIB entities with fields still contain the last evaluated text, which is what the drawing shows.

---

## 11. Metadata sections

### 11.1 Overview

| Section | R13-R2000 location | R2004+ |
|---|---|---|
| Summary info | not present | AcDb:SummaryInfo |
| Application info | not present | AcDb:AppInfo |
| File dependencies | not present | AcDb:FileDepList |
| Preview | address at 0x0D | AcDb:Preview |
| Auxiliary header | locator 5 (R2000) | AcDb:AuxHeader |
| Security | not present | AcDb:Security |

These sections are byte-oriented (no bit coding).

### 11.2 AcDb:SummaryInfo

| Type | Field |
|---|---|
| T16 | **title** |
| T16 | **subject** |
| T16 | **author** |
| T16 | **keywords** |
| T16 | **comments** |
| T16 | **last saved by** |
| T16 | **revision number** |
| T16 | **hyperlink base** |
| TIMERLL | total editing time (days, ms) |
| TIMERLL | **creation date** (Julian day, ms) |
| TIMERLL | **modification date** (Julian day, ms) |
| RS | number of **custom properties** |
| T16 x 2 per property | name, value |
| RL | unknown, 0 |
| RL | unknown, 0 |

T16 is 8-bit with RS byte length in R2004 (the drawing code page; strings are often zero terminated within the length), and UTF-16 with RS character count in R2007+. The section is unencrypted unless security flag 0x02 is set.

### 11.3 AcDb:AppInfo

The format depends on the writer, not only on the file version.

R2004 style (writer before R2007):

| Type | Field |
|---|---|
| T16 | name ("AppInfoDataList" or "AppInfoData") |
| RL | number of strings |
| T16 | comment |
| T16 | product |
| T16 | version |

R2007+ style:

| Type | Field |
|---|---|
| RL | class version (3) |
| T16 (UTF-16) | name ("AppInfoDataList") |
| RL | number of strings (3) |
| 16 bytes | checksum of the version |
| T16 (UTF-16) | **version** (for example "19.0.55.0.0" or "Teigha(R) 4.3.2.0") |
| 16 bytes | checksum of the comment |
| T16 (UTF-16) | **comment** (for example "Autodesk DWG. This file is a Trusted DWG last saved by an Autodesk application ...") |
| 16 bytes | checksum of the product |
| T16 (UTF-16) | **product information** as XML (`<ProductInformation name="AutoCAD" build_version="..." registry_version="..." install_id_string="ACAD-..." registry_localeID="1033"/>`) |

Strings in the R2007+ style are UTF-16 with an RS character count that includes a terminating zero. This section tells which program wrote the file (AutoCAD, AutoCAD LT, a vertical product, a third-party DWG library, ...).

### 11.4 AcDb:FileDepList

| Type | Field |
|---|---|
| RL | feature count |
| T32 x count | feature names: "Acad:XRef" (block records), "Acad:Image" (image definitions), "Acad:PlotConfig" (plot settings), "Acad:Text" (text styles / fonts), others possible |
| RL | file count |
| per file: T32 | **full file name** as stored |
| per file: T32 | **found path** (where the file was found when saved) |
| per file: T32 | fingerprint GUID (xrefs only) |
| per file: T32 | version GUID (xrefs only) |
| per file: RL | feature index (into the feature list) |
| per file: RL | time stamp, seconds since 1980-01-01 |
| per file: RL | file size |
| per file: RS | affects graphics (1/0) |
| per file: RL | reference count |

This is the most direct source of external dependencies: xref drawings, images, fonts and plot configurations.

### 11.5 Preview (thumbnail)

R13-R2000: at the address in the file header (0x0D). R2004+: the content of the AcDb:Preview section.

```
SN  preview start sentinel
RL  overall size of the image area
RC  number of entries
per entry:
    RC  code: 1 = header data, 2 = BMP, 3 = WMF, 6 = PNG (R2013+)
    RL  start address of the data
    RL  size of the data
data blocks
SN  preview end sentinel
```

The BMP data is a Windows device independent bitmap without the 14-byte file header (it starts with a 40-byte BITMAPINFOHEADER, usually 8-bit with a 256-entry palette). To store it as a .bmp file, prepend the file header ("BM", file size, 0, pixel data offset = 14 + 40 + palette size). PNG and WMF data are complete files. Start addresses are absolute file offsets in R13-R2000; in practice the data blocks follow the entry list in order (header data first, then the image), which is also how R2004+ sections must be read (see Q20). Newer AutoCAD versions write PNG previews.

### 11.6 AcDb:AuxHeader

| Type | Field |
|---|---|
| 3 bytes | FF 77 01 |
| RS | DWG version code (2.0 table) |
| RS | maintenance version |
| RL | **number of saves** (starts at 1) |
| RL | -1 |
| RS | saves part 1 |
| RS | saves part 2 |
| RL | 0 |
| RS, RS | DWG version code, maintenance version (again) |
| RS, RS | DWG version code, maintenance version (again) |
| RS x6 | 5, 0x893, 5, 0x893, 0, 1 (constants of unknown meaning) |
| RL x5 | 0 |
| TIMERLL | creation date (copy of `$TDCREATE`) |
| TIMERLL | update date (copy of `$TDUPDATE`) |
| RL | `$HANDSEED` if below 0x7FFFFFFF, else -1 |
| RL | educational plot stamp (0 normally) |
| RS | 0 |
| RS | saves part 1 - saves part 2 |
| RL x3 | 0 |
| RL | number of saves |
| RL x2 or x4 | 0 (R2004+: 4 values) |
| R2018+ RS x3 | 0 |

The educational plot stamp is non-zero for drawings created with student versions; it is worth reporting.

### 11.7 AcDb:Security

| Type | Field |
|---|---|
| RL | unknown (0x0C) |
| RL | unknown (0) |
| RL | 0xABCDABCD |
| RL | cryptographic provider id |
| T32 | cryptographic provider name |
| RL | algorithm id (RC4) |
| RL | key length (bits, default 40) |
| RL | size of the test buffer |
| bytes | encrypted test buffer |

Presence of this section, the security flags (5.10) or encrypted sections in the section map mean the drawing is password protected. Report it; do not try to decrypt.

---

## 12. Text content: MTEXT formatting codes and TEXT control codes

### 12.1 TEXT, ATTRIB, ATTDEF control codes

| Code | Meaning |
|---|---|
| `%%c` | diameter sign (U+2205 or U+00D8) |
| `%%d` | degree sign (U+00B0) |
| `%%p` | plus-minus sign (U+00B1) |
| `%%%` | a single percent sign |
| `%%nnn` | character with decimal code nnn |
| `%%u` | toggle underline (remove) |
| `%%o` | toggle overline (remove) |
| `\U+XXXX` | Unicode character |

### 12.2 MTEXT formatting codes

| Code | Meaning | Plain text handling |
|---|---|---|
| `\P` | new paragraph | line break |
| `\N` | new column | line break |
| `\X` | paragraph wrap on the dimension line (dimension text only) | line break |
| `\~` | non-breaking space | space (U+00A0) |
| `\\` | backslash | `\` |
| `\{`, `\}` | literal braces | `{`, `}` |
| `{ ... }` | grouping for formatting | remove the braces |
| `\L` / `\l` | underline on / off | remove |
| `\O` / `\o` | overline on / off | remove |
| `\K` / `\k` | strike-through on / off | remove |
| `\f...;` or `\F...;` | font (for example `\fArial\|b1\|i0\|c0\|p34;`) | remove up to `;` |
| `\H...;` | text height (`\H2.5;` absolute, `\H0.5x;` relative) | remove |
| `\W...;` | width factor | remove |
| `\Q...;` | oblique angle | remove |
| `\T...;` | tracking (character spacing) | remove |
| `\A0;` `\A1;` `\A2;` | vertical alignment bottom, center, top | remove |
| `\C...;` | color by ACI number | remove |
| `\c...;` | true color | remove |
| `\p...;` | paragraph properties (indents `i`, `l`, `r`, tabs `t`, alignment `q`, spacing `s`, bullets `x`) | remove up to `;` |
| `\S...;` | stacked text: `\Sa^b;` (tolerance a over b), `\Sa/b;` (fraction), `\Sa#b;` (diagonal fraction) | render as "a/b" (fraction) or "a b" / "+a -b" (tolerance) |
| `\U+XXXX` | Unicode character | decode |
| `\M+nXXXX` | multibyte character in code page n | decode |
| `%%c`, `%%d`, `%%p`, `%%%` | as in TEXT | decode |

Codes and braces can nest. To get plain text: scan left to right; on `\` look at the next character and apply the table (unknown letters followed by `;`-terminated arguments are removed up to the `;`); drop unescaped braces. Keep paragraph breaks as line breaks so that the reader sees the original lines.

Field codes appear as `%<\AcVar ...>%`, `%<\AcObjProp ...>%` and similar only in FIELD objects; MTEXT contains the evaluated text.

---

## 13. Ownership and structure summary

```
header
 |- BLOCK control -> BLOCK_HEADER (block records)
 |       |- *Model_Space -> entities (model space)
 |       |- *Paper_Space, *Paper_SpaceN -> entities of layouts
 |       |- named blocks -> BLOCK, entities, ENDBLK; ATTDEFs inside
 |       |- xref blocks (xref path name)
 |- LAYER control -> LAYER
 |- STYLE control -> STYLE (fonts)
 |- LTYPE control -> LTYPE
 |- VIEW, UCS, VPORT, APPID, DIMSTYLE controls -> entries
 |- named objects dictionary
         |- ACAD_LAYOUT -> LAYOUT -> block record
         |- ACAD_GROUP -> GROUP -> entities
         |- ACAD_IMAGE_DICT -> IMAGEDEF (file path)
         |- other dictionaries and XRECORDs
entities
 |- INSERT -> block record; ATTRIB* (values); SEQEND
 |- DIMENSION -> DIMSTYLE, *D block
 |- TABLE -> *T block
 |- every entity -> layer, linetype, extension dictionary (optional), EED
```

---

## 14. Robustness

### 14.1 What can be wrong

- Truncated files (download errors, crashed saves).
- Wrong CRCs and checksums (some third-party writers produce them).
- Section locators or page map addresses beyond the end of the file.
- Duplicate handles in the object map, objects not reachable from the header, table entries with duplicate names, invalid characters in names.
- Objects with a "size in bits" larger than the object size, handle references with counter > 8, EED blocks with unknown codes.
- Unknown class types (vertical products, third-party applications), proxy objects.
- Encrypted (password protected) content.
- Lone UTF-16 surrogates and invalid bytes in the code page.
- R2004+ files with a gap page map, zero pages omitted, or pages in a different physical order than logical order.
- Writers that produce slightly non-standard files (third-party libraries, older converters): AppInfo tells the writer.

### 14.2 Rules

1. Never trust a size or count before checking it against the available data. Every read must be bounds checked; read past the end must stop the current object, not the whole file.
2. Recommended limits:

| Item | Limit |
|---|---|
| file size | configurable, default 2 GiB |
| decompressed logical section | 790 MiB and at most 8 x the file size per section |
| R2004 section page count | 1,000,000 |
| R2007 page count per section | 0xF0000 |
| R2007 section name length | 48 bytes |
| class count | 5000 |
| object map chunk size | 2048 bytes |
| number of objects | number of object map entries; `$HANDSEED` is an upper bound for handle values |
| single object size | 2^30 bytes (MS limit), and at most the remaining section size |
| reactors per object | 100,000 (normally under 20) |
| points in LWPOLYLINE, vertices, knots, cells | 10 million total per object, checked against remaining bits |
| string length | remaining bits of the stream |
| nesting of INSERT expansion | 64 levels, detect cycles |
| decompression output | never larger than the declared size |

3. Decompressors must check every back reference distance and length against the output written so far.
4. A CRC or checksum mismatch is a warning, not a fatal error. Report it in the output.
5. Decoding errors in one object must not stop the file: record "object at handle X could not be read (reason)" and continue with the next map entry.
6. When a value is out of range (for example a scale flag 4, a negative count), treat the object as damaged and skip it.
7. Report every unreadable part with its reason (encrypted, unknown class, damaged), as the core principle requires.

### 14.3 Detecting encrypted or protected files

- R2004+: security flags at 0x18 with bit 0x01 or 0x02; an `AcDb:Security` section; section map entries with encrypted = 1.
- R2007: section map "encryption" = 1, security flags as above.
- R13-R2000 have no password protection.
- Digitally signed files (0x10) are readable; the `AcDb:Signature` section can be reported as "signed".

---

## 15. What a reader needs for knowledge extraction

| Knowledge | Where it comes from |
|---|---|
| File version and writer | version string (2), maintenance byte, AcDb:AppInfo (11.3), AuxHeader (number of saves, 11.6) |
| Units | `$INSUNITS`, `$LUNITS`, `$LUPREC`, `$AUNITS`, `$AUPREC`, `$ANGDIR`, `$ANGBASE` (8.2), `$MEASUREMENT` (template section), block record insert units (R2007+) |
| Extents and limits | `$EXTMIN`/`$EXTMAX`, `$LIMMIN`/`$LIMMAX`, paper space equivalents, layout extents |
| Dates | `$TDCREATE`, `$TDUPDATE`, `$TDINDWG`, summary info dates |
| Identity | `$FINGERPRINTGUID`, `$VERSIONGUID`, `$PROJECTNAME`, `$HANDSEED` |
| Metadata | AcDb:SummaryInfo (title, subject, author, keywords, comments, last saved by, revision, hyperlink base, custom properties) |
| Layer table | LAYER entries (10.4): name, on/off, frozen, locked, plot, color, linetype, lineweight; current layer `$CLAYER`; entity counts per layer by scanning entities |
| Block definitions | BLOCK_HEADER entries (10.5): name, description, base point, flags, ATTDEFs inside (tag, prompt, default) |
| Insert counts (parts list) | count INSERT and MINSERT (rows x columns) by block record, separately for model space, each layout and inside other blocks; exclude anonymous blocks; resolve dynamic block variants to the original block |
| Attribute values (title blocks) | ATTRIB entities under each INSERT (10.9): tag and value; together with the block name they form the title block |
| All text | TEXT, MTEXT (plain text per 12.2), ATTRIB, ATTDEF, MULTILEADER text label and block labels, TABLE cells (or `*T` block MTEXT), DIMENSION user text and measurement, LEADER annotations, proxy graphics text, XRECORD and EED strings (as low priority data) |
| Dimension values | DIMENSION actual measurement (R2000+), user text, dimension style name; the displayed text from the `*D` block MTEXT |
| External references | block records with xref flag and path; AcDb:FileDepList; IMAGEDEF paths; underlay definitions; STYLE font files |
| Layouts | LAYOUT objects: name, tab order, paper size, plot settings, viewports and their scale; entity lists through the layout's block record |
| Groups | GROUP objects with names from ACAD_GROUP |
| Application data | EED grouped by APPID, XRECORDs in dictionaries, DICTIONARYVAR custom variables, proxy class names |
| Preview | AcDb:Preview (11.5) as an image file listed as "needs vision" |
| Protection state | security flags and AcDb:Security (14.3) |

Minimum decoding scope for a first useful reader (R2000):

1. File header and locators, header variables up to `$INSUNITS` and the handles listed above.
2. Classes.
3. Object map.
4. Object prefix, EED (skip or decode), common entity data and handles.
5. Objects: control objects, LAYER, STYLE, LTYPE (name and description), APPID, DIMSTYLE (name only), BLOCK_HEADER, BLOCK, ENDBLK, SEQEND, INSERT, MINSERT, ATTRIB, ATTDEF, TEXT, MTEXT, DIMENSION family, LEADER, DICTIONARY, XRECORD, LAYOUT, GROUP, IMAGE, IMAGEDEF, LWPOLYLINE, LINE, CIRCLE, ARC (geometry only for counts and extents).
6. Everything else: skip by size and report as "not interpreted" with its type name.

---

## 16. Open questions

These points are unclear or the available descriptions disagree. Each should be settled with test files (expected output from a known-good tool) before the implementation relies on it.

| No. | Topic | Question |
|---|---|---|
| Q1 | Common entity data R2007+ | Order of the material flags and plot style flags. One description lists linetype, plot style, material; another reads linetype, material, plot style. Both are usually 0, so most files do not show the difference. |
| Q2 | ENC color R2004+ | Order of the transparency BL (flag 0x20) and the RGB BL (flag 0x80), and the exact layout of the transparency value (type in the high byte, alpha in the low byte, or the reverse). Also: whether the DBCOLOR handle (flag 0x40) is the first entry in the handle stream or comes after the previous/next links. |
| Q3 | R2007+ header string stream | Whether `$MENUNAME` and the four unknown unit strings are present in the string stream of R2007+ files. One description marks MENUNAME as R13-R2004 only. Order given in 8.2 is the observed one. |
| Q4 | Header dates | Whether the stored `$TDCREATE`/`$TDUPDATE` are local time or universal time (`$TDUCREATE`/`$TDUUPDATE`). Compare with summary info dates and the file system time. |
| Q5 | BLL length prefix | Fixed 3 bits versus 1 to 3 bits stopping at the first zero (3B). Affects `$REQUIREDVERSIONS` (R2013+) and the entity graphics size (R2010+). |
| Q6 | R2010 maintenance version 3 | The extra RL after the size in the header and classes sections is present for maintenance version > 3; it is unclear whether the string stream position at exactly 3 follows the same rule. |
| Q7 | R2007 section map name length | Byte count including terminator, or character count (needing x2 and +2). |
| Q8 | EED 1003 | Byte order of the 8-byte layer handle (most significant first like 1005, or little-endian). |
| Q9 | EED string code page | Byte order of the RS code page that follows the length byte in R13-R2004. |
| Q10 | LAYOUT points | Which 3BD comes first after the layout flags: the insertion base (DXF 12) or the UCS origin (DXF 13). |
| Q11 | Proxy drawing format BL | Whether the maintenance version is in bits 8-15 or bits 16-31. |
| Q12 | INSERT owned count | Whether the R2004+ owned object count is stored even when "has attributes" is 0 (TABLE seems to store it always). |
| Q13 | MULTILEADER context | Whether the annotation context data starts with the common context fields (version, default flag, scale handle) in the DWG stream, and whether connection point and direction of a leader root are always present or only when their flags are set. |
| Q14 | Dynamic blocks | Exact way to resolve an anonymous `*U` block record to its original dynamic block (XDATA "AcDbBlockRepBTag" versus the extension dictionary entry). |
| Q15 | R2018 multi-line attributes | Exact layout of the embedded MTEXT data in ATTRIB/ATTDEF when the attribute type is 2 or 4, and whether the plain value is also in the TEXT value field. |
| Q16 | Underlays | Field layout of PDFDEFINITION/DWFDEFINITION/DGNDEFINITION and the underlay entities (file path and page name expected as the first strings). |
| Q17 | TABLE R2010+ | Full layout of the TABLE entity and TABLECONTENT, cell styles and custom data. The `*T` block fallback avoids it. |
| Q18 | R2004 data page header | Whether offset 0x10 is an 8-byte start offset or a 4-byte offset plus 4 unknown bytes (always 0 so far). |
| Q19 | STYLE flags | Order of the "shape file" and "vertical" bits. |
| Q20 | Preview addresses | Whether entry start addresses in R2004+ preview data are absolute file offsets, section offsets or ignored; reading the blocks sequentially works in practice. |
| Q21 | Reed-Solomon polynomials | Exact field polynomials and generator roots for R2007 error correction (only needed when correcting errors, not for reading undamaged files). |
| Q22 | Classes R2007 | Whether the class "DWG version" and "maintenance version" are BS or BL in R2007 (BS works for R2004 and R2010+). |
| Q23 | Object map chunk limit | Documented maximum 2032 bytes; files with 2036 and up to 2040 exist. |
| Q24 | File header bytes 0x11-0x17 | Which bytes hold the writer's version and maintenance numbers in R2004+ (0x11/0x12 or 0x16/0x17). |
| Q25 | Unknown header values | Meaning of the unknown header fields (R2004+ extra BL triples, R2013+ unknown dictionary handle). |
| Q26 | R2007 encoding value 1 | Whether pages of sections with encoding 1 (not interleaved) must be read with a different de-interleave rule when they span more than one code word. |
