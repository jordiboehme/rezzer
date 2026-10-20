# textenc

textenc turns bytes in any text encoding into UTF-8.

It tries these steps in order:

1. A byte order mark (UTF-8, UTF-16LE or UTF-16BE)
2. Strict UTF-8
3. Detection with chardetng and decoding with encoding_rs

It never fails. Bytes it can't decode become U+FFFD and the result says that it is not confident. The result also names the encoding and the line ending style it found.

```rust
// "hi" in UTF-16LE with a byte order mark
let decoded = textenc::decode(b"\xff\xfeh\0i\0");
assert_eq!(decoded.text, "hi");
assert_eq!(decoded.encoding, "UTF-16LE");
```

Large inputs are not shown whole to the detector: it looks at samples from the start, the middle and the end.

License: MPL-2.0
