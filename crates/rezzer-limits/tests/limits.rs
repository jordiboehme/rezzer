// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::Duration;

use rezzer_limits::{Budget, CancelToken, Limits, contain};

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn defaults_match_spec() {
    let l = Limits::default();
    assert_eq!(l.max_file_bytes, 2 * GIB);
    assert_eq!(l.max_unpacked_bytes, 4 * GIB);
    assert_eq!(l.max_ratio, 100);
    assert_eq!(l.max_depth, 256);
    assert_eq!(l.max_time, Duration::from_secs(120));
    assert_eq!(l.max_memory_bytes, 2 * GIB);
    assert_eq!(l.page_tokens, 20_000);
    assert_eq!(l.max_elements, 2_000_000);
    assert_eq!(l.max_text_bytes, 256 * 1024 * 1024);
}

#[test]
fn depth_limit_hits_at_max() {
    let b = Budget::new(Limits {
        max_depth: 3,
        ..Limits::default()
    });
    let _a = b.enter().unwrap();
    let _b = b.enter().unwrap();
    let _c = b.enter().unwrap();
    let err = b.enter().unwrap_err();
    assert_eq!(err.limit, "depth");
    assert_eq!(err.to_string(), "depth limit reached (4 levels > 3 levels)");
}

#[test]
fn depth_guard_decrements_on_drop() {
    let b = Budget::new(Limits {
        max_depth: 1,
        ..Limits::default()
    });
    drop(b.enter().unwrap());
    assert!(b.enter().is_ok());
    let _g = b.enter();
    assert!(b.enter().is_err());
    assert!(b.enter().is_err());
}

#[test]
fn time_limit_hits() {
    let b = Budget::new(Limits {
        max_time: Duration::from_millis(0),
        ..Limits::default()
    });
    std::thread::sleep(Duration::from_millis(1));
    assert_eq!(b.check_time().unwrap_err().limit, "time");
}

#[test]
fn unpacked_limit_hits() {
    let b = Budget::new(Limits {
        max_unpacked_bytes: 10,
        ..Limits::default()
    });
    assert!(b.add_unpacked(6).is_ok());
    let err = b.add_unpacked(6).unwrap_err();
    assert_eq!(err.limit, "unpacked");
    assert_eq!((err.value, err.max), (12, 10));
    assert_eq!(
        err.to_string(),
        "unpacked limit reached (12 bytes > 10 bytes)"
    );
}

#[test]
fn unpacked_counter_saturates() {
    let b = Budget::new(Limits::default());
    assert!(b.add_unpacked(u64::MAX).is_err());
    let err = b.add_unpacked(u64::MAX).unwrap_err();
    assert_eq!((err.value, err.max), (u64::MAX, 4 * GIB));
    assert!(b.add_unpacked(1).is_err());
}

#[test]
fn time_hit_names_milliseconds() {
    let b = Budget::new(Limits {
        max_time: Duration::from_millis(0),
        ..Limits::default()
    });
    std::thread::sleep(Duration::from_millis(2));
    let err = b.check_time().unwrap_err();
    assert_eq!(err.unit, "ms");
    assert!(err.to_string().starts_with("time limit reached ("));
    assert!(err.to_string().ends_with(" ms > 0 ms)"));
}

#[test]
fn unknown_config_key_is_rejected() {
    assert!(serde_json::from_str::<Limits>(r#"{"max_dept": 5}"#).is_err());
}

#[test]
fn cancel_makes_check_time_fail() {
    let b = Budget::new(Limits::default());
    assert!(!b.is_cancelled());
    b.cancel();
    assert!(b.is_cancelled());
    assert_eq!(b.check_time().unwrap_err().limit, "cancelled");
}

#[test]
fn contain_catches_panic() {
    let r: Result<(), String> = contain(|| panic!("boom"));
    assert!(r.unwrap_err().contains("boom"));
    assert_eq!(contain(|| 7), Ok(7));
}

#[test]
fn contain_catches_formatted_panic() {
    let n = 5;
    let r = contain(|| panic!("bad {n}"));
    assert_eq!(r.unwrap_err(), "bad 5");
}

#[test]
fn budget_is_send_sync() {
    fn assert_ss<T: Send + Sync>() {}
    assert_ss::<Budget>();
}

#[test]
fn limits_serde_roundtrip_with_seconds() {
    let json = serde_json::to_value(Limits::default()).unwrap();
    assert_eq!(json["max_time"], 120);
    let back: Limits = serde_json::from_value(json).unwrap();
    assert_eq!(back, Limits::default());
    let partial: Limits = serde_json::from_str(r#"{"max_depth": 5}"#).unwrap();
    assert_eq!(partial.max_depth, 5);
    assert_eq!(partial.max_ratio, 100);
}

#[test]
fn a_shared_cancel_token_stops_the_budget() {
    let token = CancelToken::default();
    let b = Budget::with_cancel(Limits::default(), token.clone());
    assert!(b.check_time().is_ok());
    assert!(!token.is_cancelled());
    token.cancel();
    assert!(b.is_cancelled());
    assert_eq!(b.check_time().unwrap_err().limit, "cancelled");
    // Cancelling through the budget is seen by every holder of the token.
    let other = CancelToken::default();
    Budget::with_cancel(Limits::default(), other.clone()).cancel();
    assert!(other.is_cancelled());
}

#[test]
fn element_budget_hits_after_max() {
    let b = Budget::new(Limits {
        max_elements: 10,
        ..Limits::default()
    });
    assert!(b.add_elements(4).is_ok());
    assert!(b.add_elements(6).is_ok());
    let hit = b.add_elements(1).unwrap_err();
    assert_eq!(hit.limit, "elements");
    assert_eq!(hit.value, 11);
    assert_eq!(hit.max, 10);
    assert_eq!(hit.unit, "count");
    // Stays over the limit.
    assert!(b.add_elements(0).is_err());
    assert_eq!(b.elements(), 11);
}

#[test]
fn element_budget_saturates() {
    let b = Budget::new(Limits::default());
    assert!(b.add_elements(u64::MAX).is_err());
    assert!(b.add_elements(u64::MAX).is_err());
    assert_eq!(b.elements(), u64::MAX);
}

#[test]
fn max_elements_reads_from_config() {
    let l: Limits = serde_json::from_str(r#"{"max_elements": 7}"#).unwrap();
    assert_eq!(l.max_elements, 7);
}

#[test]
fn text_runs_are_checked_one_by_one() {
    let b = Budget::new(Limits {
        max_text_bytes: 10,
        ..Limits::default()
    });
    assert!(b.charge_text(10).is_ok());
    assert!(b.charge_text(10).is_ok());
    let hit = b.charge_text(11).unwrap_err();
    assert_eq!(
        (hit.limit, hit.value, hit.max, hit.unit),
        ("text", 11, 10, "bytes")
    );
}

#[test]
fn max_text_bytes_reads_from_config() {
    let l: Limits = serde_json::from_str(r#"{"max_text_bytes": 7}"#).unwrap();
    assert_eq!(l.max_text_bytes, 7);
}
