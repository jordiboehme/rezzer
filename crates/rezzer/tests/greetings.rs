// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading the running binary gives the normal result, and scripts see nothing extra.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rezzer");

#[test]
fn reading_itself_with_stderr_piped_prints_no_greeting() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("rezzer-copy");
    std::fs::copy(BIN, &copy).unwrap();
    let own = Command::new(BIN)
        .args(["read", "--no-cache", BIN])
        .output()
        .unwrap();
    let other = Command::new(BIN)
        .args(["read", "--no-cache"])
        .arg(&copy)
        .output()
        .unwrap();
    assert_eq!(own.status.code(), other.status.code());
    assert!(!String::from_utf8_lossy(&own.stderr).contains("End of Line"));
    assert!(!String::from_utf8_lossy(&other.stderr).contains("End of Line"));
    assert!(!String::from_utf8_lossy(&own.stderr).contains("Greetings"));
    assert!(!String::from_utf8_lossy(&other.stderr).contains("Greetings"));
    assert_eq!(own.stdout.is_empty(), other.stdout.is_empty());
}
