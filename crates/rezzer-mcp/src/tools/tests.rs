// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests of the tools.

use super::file_uri::{BASE64_CHUNK, DEFAULT_NAME, base64_decoded_len, percent_decoded_len};
use super::*;

/// A run that has a document but no pages, as after a render panic.
fn run_without_pages(output: OutputKind) -> RunResult {
    let mut registry = rezzer_model::Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let config = rezzer_pipeline::Config {
        cache: false,
        ..rezzer_pipeline::Config::default()
    };
    let pipeline = Pipeline::new(config, registry);
    let request = Request {
        output,
        ..Request::default()
    };
    let mut result = pipeline.run_bytes("a.csv", b"a,b\n1,2\n".to_vec(), None, &request);
    assert!(result.document.is_some());
    result.completeness = Completeness::Failed;
    result.markdown_pages.clear();
    result.json = None;
    result.message = Some("rendering stopped with an internal error: boom".into());
    result
}

#[test]
fn a_run_without_pages_answers_with_its_message() {
    for output in [OutputKind::Markdown, OutputKind::Json] {
        let answer = rez_answer(run_without_pages(output), output, 1);
        let message = "rendering stopped with an internal error: boom";
        assert!(answer.failed);
        assert_eq!(answer.text, message, "{output:?}");
        let s = &answer.structured;
        assert_eq!(s["status"], "failed");
        assert_eq!(s["message"], message);
        assert_eq!(s["pages"], 0);
        assert_eq!(s["page"], Value::Null);
        assert_eq!(s["format"], "csv");
        assert_eq!(s["sha256"].as_str().unwrap().len(), 64);
    }
}

#[test]
fn wrapped_base64_across_chunks_decodes_into_one_exact_buffer() {
    use base64::Engine as _;
    let raw: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
    let wrapped: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    let uri = format!("data:;base64,{}", wrapped.join("\r\n"));
    let data = parse_file_uri(uri, u64::MAX).unwrap();
    assert_eq!(data.bytes, raw);
    assert!(
        data.bytes.capacity() <= raw.len() + 3,
        "{}",
        data.bytes.capacity()
    );
    let flat = parse_file_uri(format!("data:;base64,{encoded}"), u64::MAX).unwrap();
    assert_eq!(flat.bytes, raw);
    assert!(flat.bytes.capacity() <= raw.len() + 3);
}

#[test]
fn decoded_sizes_are_exact() {
    assert_eq!(base64_decoded_len(b"YSxiCg=="), 4);
    assert_eq!(base64_decoded_len(b"YSxi\nCg"), 4);
    assert_eq!(base64_decoded_len(b"aGk"), 2);
    assert_eq!(base64_decoded_len(b""), 0);
    assert_eq!(percent_decoded_len(b"a%2Cb%0A%zz%4"), 9);
    let data = parse_file_uri("data:,a%2Cb%0A%zz%4".into(), 100).unwrap();
    assert_eq!(data.bytes, b"a,b\n%zz%4");
    assert_eq!(data.bytes.capacity(), data.bytes.len());
}

#[test]
fn a_bad_symbol_is_named_with_its_position_after_a_chunk() {
    let mut encoded = "A".repeat(BASE64_CHUNK + 8);
    encoded.insert(BASE64_CHUNK + 4, '$');
    let err = parse_file_uri(format!("data:;base64,{encoded}\n"), u64::MAX).unwrap_err();
    assert!(
        err.contains(&format!("at symbol {}", BASE64_CHUNK + 5)),
        "{err}"
    );
    assert!(err.ends_with("not base64"), "{err}");
}

#[test]
fn data_uri_with_type_name_and_base64() {
    let data = parse_file_uri(
        "data:text/csv;name=a%20b.csv;base64,YSxiCg==".to_string(),
        100,
    )
    .unwrap();
    assert_eq!(data.name, "a b.csv");
    assert_eq!(data.mime.as_deref(), Some("text/csv"));
    assert_eq!(data.bytes, b"a,b\n");
}

#[test]
fn data_uri_without_type_or_name() {
    let data = parse_file_uri("data:,hello%20world".to_string(), 100).unwrap();
    assert_eq!(data.name, DEFAULT_NAME);
    assert_eq!(data.mime, None);
    assert_eq!(data.bytes, b"hello world");
}

#[test]
fn data_uri_ignores_other_parameters_and_unpadded_base64() {
    let data = parse_file_uri(
        "DATA:text/plain;charset=utf-8;name=x.txt;BASE64,aGk".into(),
        100,
    )
    .unwrap();
    assert_eq!(data.name, "x.txt");
    assert_eq!(data.mime.as_deref(), Some("text/plain"));
    assert_eq!(data.bytes, b"hi");
}

#[test]
fn data_uri_name_keeps_only_the_file_name() {
    let data = parse_file_uri("data:;name=..%2Fsecret%2Fa.csv,x".to_string(), 100).unwrap();
    assert_eq!(data.name, "a.csv");
}

#[test]
fn data_uri_base64_is_a_parameter_only_at_the_end() {
    // `base64` as the type position is not the encoding flag.
    let data = parse_file_uri("data:base64,aGk".to_string(), 100).unwrap();
    assert_eq!(data.bytes, b"aGk");
}

#[test]
fn oversize_data_is_refused_before_decoding() {
    let big = format!("data:;base64,{}", "A".repeat(4000));
    let err = parse_file_uri(big, 100).unwrap_err();
    assert!(err.contains("too large"), "{err}");
    let err = parse_file_uri("data:,12345".to_string(), 4).unwrap_err();
    assert!(err.contains("5 bytes, the limit is 4 bytes"), "{err}");
}
