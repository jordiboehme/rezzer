// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Size, depth and time limits that every reader enforces, plus panic containment.

use std::fmt;
use std::panic::{AssertUnwindSafe, UnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const GIB: u64 = 1024 * 1024 * 1024;

/// Configurable limits. Missing fields in a config file fall back to the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_unpacked_bytes: u64,
    pub max_ratio: u32,
    pub max_depth: u32,
    /// In config files this is a whole number of seconds; sub-second parts are dropped on
    /// serialization.
    #[serde(with = "duration_secs")]
    pub max_time: Duration,
    pub max_memory_bytes: u64,
    pub page_tokens: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 2 * GIB,
            max_unpacked_bytes: 4 * GIB,
            max_ratio: 100,
            max_depth: 256,
            max_time: Duration::from_secs(120),
            max_memory_bytes: 2 * GIB,
            page_tokens: 20_000,
        }
    }
}

mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_secs)
    }
}

/// A limit that was exceeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitHit {
    pub limit: &'static str,
    pub value: u64,
    pub max: u64,
    /// Unit of `value` and `max`: "bytes", "levels", "ms" or "count".
    pub unit: &'static str,
}

impl fmt::Display for LimitHit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} limit reached ({} {unit} > {} {unit})",
            self.limit,
            self.value,
            self.max,
            unit = self.unit
        )
    }
}

impl std::error::Error for LimitHit {}

/// A cancel flag shared between the one who may cancel (a server, a signal handler) and a
/// [`Budget`]. Clones share the flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Running counters for one conversion. Shareable between threads.
#[derive(Debug)]
pub struct Budget {
    limits: Limits,
    start: Instant,
    unpacked: AtomicU64,
    depth: AtomicU32,
    cancelled: CancelToken,
}

impl Budget {
    pub fn new(limits: Limits) -> Budget {
        Budget::with_cancel(limits, CancelToken::default())
    }

    /// A budget that is cancelled when `cancel` is: [`Budget::check_time`] then fails with
    /// the limit `cancelled`, so the reader stops at its next check.
    pub fn with_cancel(limits: Limits, cancel: CancelToken) -> Budget {
        Budget {
            limits,
            start: Instant::now(),
            unpacked: AtomicU64::new(0),
            depth: AtomicU32::new(0),
            cancelled: cancel,
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Fails when the time limit is over or the budget was cancelled.
    /// Values in the error are milliseconds.
    pub fn check_time(&self) -> Result<(), LimitHit> {
        let elapsed = self.start.elapsed();
        let limit = if self.is_cancelled() {
            "cancelled"
        } else if elapsed > self.limits.max_time {
            "time"
        } else {
            return Ok(());
        };
        Err(LimitHit {
            limit,
            value: millis(elapsed),
            max: millis(self.limits.max_time),
            unit: "ms",
        })
    }

    /// Adds `n` unpacked bytes to the running total and checks the limit.
    pub fn add_unpacked(&self, n: u64) -> Result<(), LimitHit> {
        let previous = self
            .unpacked
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                Some(t.saturating_add(n))
            })
            .unwrap_or(u64::MAX);
        let total = previous.saturating_add(n);
        if total > self.limits.max_unpacked_bytes {
            return Err(LimitHit {
                limit: "unpacked",
                value: total,
                max: self.limits.max_unpacked_bytes,
                unit: "bytes",
            });
        }
        Ok(())
    }

    /// Enters one nesting level. The level is left when the guard is dropped.
    pub fn enter(&self) -> Result<DepthGuard<'_>, LimitHit> {
        let depth = self.depth.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if depth > self.limits.max_depth {
            self.depth.fetch_sub(1, Ordering::Relaxed);
            return Err(LimitHit {
                limit: "depth",
                value: u64::from(depth),
                max: u64::from(self.limits.max_depth),
                unit: "levels",
            });
        }
        Ok(DepthGuard { budget: self })
    }

    pub fn cancel(&self) {
        self.cancelled.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Decrements the depth counter on drop.
#[derive(Debug)]
pub struct DepthGuard<'a> {
    budget: &'a Budget,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.budget.depth.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Runs `f` and turns a panic into an error message.
pub fn contain<T>(f: impl FnOnce() -> T + UnwindSafe) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "panic with a non-text payload".to_string()
        }
    })
}
