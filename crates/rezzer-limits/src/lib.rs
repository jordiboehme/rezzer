// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Size, depth, element and time limits that every reader enforces, plus panic containment.

mod alloc;
pub use alloc::{Allocator, moved_bytes};

use std::fmt;
use std::panic::{AssertUnwindSafe, UnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Configurable limits. Missing fields in a config file fall back to the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_file_bytes: u64,
    /// The largest file sent inline (a `data:` URI over MCP), at most `max_file_bytes`. A
    /// file by path is bound by `max_file_bytes` only. See [`check_inline_bytes`].
    pub max_inline_bytes: u64,
    pub max_unpacked_bytes: u64,
    pub max_ratio: u32,
    pub max_depth: u32,
    /// In config files this is a whole number of seconds; sub-second parts are dropped on
    /// serialization. It bounds reading inside the readers and the enrich checks, not
    /// opening, hashing, rendering or serialization.
    #[serde(with = "duration_secs")]
    pub max_time: Duration,
    /// Memory cap, enforced where memory is countable. Readers bound the memory of the
    /// document they build through [`Limits::max_elements`].
    pub max_memory_bytes: u64,
    pub page_tokens: usize,
    /// The largest part in tokens when Rezzer writes files: a larger document is written as
    /// a main document plus parts. 0 writes one file. See [`check_part_size`].
    pub part_size: usize,
    /// Model elements (blocks, inlines, table cells) one read may build. This is how readers
    /// bound the memory of the document model; on a hit the reader stops with a `Limit` loss.
    pub max_elements: u64,
    /// Bytes of one text run (a paragraph, a cell, a line). The element budget counts
    /// model elements, not their size, so one huge paragraph or cell needs its own bound.
    /// See [`Budget::charge_text`].
    pub max_text_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 2 * GIB,
            max_inline_bytes: DEFAULT_INLINE_BYTES,
            max_unpacked_bytes: 4 * GIB,
            max_ratio: 100,
            max_depth: 256,
            max_time: Duration::from_secs(120),
            max_memory_bytes: 2 * GIB,
            page_tokens: 20_000,
            part_size: DEFAULT_PART_SIZE,
            max_elements: 2_000_000,
            max_text_bytes: 256 * MIB,
        }
    }
}

impl Limits {
    /// The largest inline file that is read: `max_inline_bytes`, but never more than
    /// `max_file_bytes`.
    pub fn inline_bytes(&self) -> u64 {
        self.max_inline_bytes.min(self.max_file_bytes)
    }
}

/// The default [`Limits::max_inline_bytes`]: 32 MiB.
pub const DEFAULT_INLINE_BYTES: u64 = 32 * MIB;

/// The largest [`Limits::max_inline_bytes`]. One inline message may take six times as many
/// bytes as its file, and the server holds the whole message.
pub const MAX_INLINE_BYTES: u64 = GIB;

/// Checks an inline file size limit: at least 1 byte and at most [`MAX_INLINE_BYTES`].
pub fn check_inline_bytes(n: u64) -> Result<(), String> {
    if (1..=MAX_INLINE_BYTES).contains(&n) {
        Ok(())
    } else {
        Err(format!(
            "max_inline_bytes must be between 1 and {MAX_INLINE_BYTES}"
        ))
    }
}

/// The default [`Limits::part_size`]: Crystalline's budget for a source engram.
pub const DEFAULT_PART_SIZE: usize = 10_000;

/// The smallest [`Limits::part_size`] other than 0.
pub const MIN_PART_SIZE: usize = 2_500;

/// Checks a part size: 0 (one file) or at least [`MIN_PART_SIZE`].
pub fn check_part_size(n: usize) -> Result<(), String> {
    if n == 0 || n >= MIN_PART_SIZE {
        Ok(())
    } else {
        Err(format!("part_size must be 0 or at least {MIN_PART_SIZE}"))
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

/// The clock a [`Budget`] measures its time with. Clones share the clock.
#[derive(Debug, Clone, Default)]
pub enum Clock {
    /// The wall clock.
    #[default]
    Real,
    /// A counted clock for tests: see [`Clock::steps`].
    Steps(Arc<StepClock>),
}

/// A clock that moves by one step at each reading, so a time limit depends on the work done,
/// not on the machine's load.
#[derive(Debug)]
pub struct StepClock {
    step: Duration,
    reads: AtomicU64,
}

impl Clock {
    /// A clock that moves by `step` at each reading: each [`Budget::elapsed`] and
    /// [`Budget::check_time`] is one reading and gives `step` times the readings so far.
    /// Readings are counted per clock, not per budget, so one such clock is meant for one
    /// read.
    pub fn steps(step: Duration) -> Clock {
        Clock::Steps(Arc::new(StepClock {
            step,
            reads: AtomicU64::new(0),
        }))
    }

    /// The readings of a step clock so far; 0 for the wall clock, which does not count.
    pub fn reads(&self) -> u64 {
        match self {
            Clock::Real => 0,
            Clock::Steps(s) => s.reads.load(Ordering::Relaxed),
        }
    }

    /// The time since `start`: on the wall clock, or one reading of a step clock.
    fn since(&self, start: Instant) -> Duration {
        match self {
            Clock::Real => start.elapsed(),
            Clock::Steps(s) => {
                let reads = s.reads.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                u32::try_from(reads)
                    .ok()
                    .and_then(|n| s.step.checked_mul(n))
                    .unwrap_or(Duration::MAX)
            }
        }
    }
}

/// Running counters for one conversion. Shareable between threads.
#[derive(Debug)]
pub struct Budget {
    limits: Limits,
    clock: Clock,
    start: Instant,
    unpacked: AtomicU64,
    depth: AtomicU32,
    elements: AtomicU64,
    cancelled: CancelToken,
    /// Set once a time stop cut the work short. It stays set.
    time_stopped: AtomicBool,
}

impl Budget {
    pub fn new(limits: Limits) -> Budget {
        Budget::with_cancel(limits, CancelToken::default())
    }

    /// A budget that is cancelled when `cancel` is: [`Budget::check_time`] then fails with
    /// the limit `cancelled`, so the reader stops at its next check.
    pub fn with_cancel(limits: Limits, cancel: CancelToken) -> Budget {
        Budget::with_clock(limits, cancel, Clock::Real)
    }

    /// A budget like [`Budget::with_cancel`] that measures its time with `clock`.
    pub fn with_clock(limits: Limits, cancel: CancelToken, clock: Clock) -> Budget {
        Budget {
            limits,
            clock,
            start: Instant::now(),
            unpacked: AtomicU64::new(0),
            depth: AtomicU32::new(0),
            elements: AtomicU64::new(0),
            cancelled: cancel,
            time_stopped: AtomicBool::new(false),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The time since the budget was made, on its [`Clock`].
    pub fn elapsed(&self) -> Duration {
        self.clock.since(self.start)
    }

    /// Fails when the time limit is over or the budget was cancelled. A failure counts as a
    /// time stop (see [`Budget::time_stopped`]). Values in the error are milliseconds.
    pub fn check_time(&self) -> Result<(), LimitHit> {
        let elapsed = self.elapsed();
        let limit = if self.is_cancelled() {
            "cancelled"
        } else if elapsed > self.limits.max_time {
            "time"
        } else {
            return Ok(());
        };
        self.note_time_stop();
        Err(LimitHit {
            limit,
            value: millis(elapsed),
            max: millis(self.limits.max_time),
            unit: "ms",
        })
    }

    /// Records that a stop which depends on time cut the work short: a reader's own share of
    /// the time limit, a timeout of one call, or the limit itself. Such a result depends on
    /// the machine's load, so it must not be cached.
    pub fn note_time_stop(&self) {
        self.time_stopped.store(true, Ordering::Relaxed);
    }

    /// Whether a time stop cut the work short: a failed [`Budget::check_time`] or a
    /// [`Budget::note_time_stop`]. It stays true once set.
    pub fn time_stopped(&self) -> bool {
        self.time_stopped.load(Ordering::Relaxed)
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

    /// Adds `n` model elements (blocks, inlines, cells) to the running count and checks
    /// [`Limits::max_elements`]. Once over the limit every later call fails too.
    pub fn add_elements(&self, n: u64) -> Result<(), LimitHit> {
        let previous = self
            .elements
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                Some(t.saturating_add(n))
            })
            .unwrap_or(u64::MAX);
        let total = previous.saturating_add(n);
        if total > self.limits.max_elements {
            return Err(LimitHit {
                limit: "elements",
                value: total,
                max: self.limits.max_elements,
                unit: "count",
            });
        }
        Ok(())
    }

    /// Checks the size of one text run (a paragraph, a cell, a line) against
    /// [`Limits::max_text_bytes`]. Call it before the text is copied or built, with the size
    /// the run would have. Runs are checked one by one; nothing is added up.
    pub fn charge_text(&self, bytes: usize) -> Result<(), LimitHit> {
        let value = u64::try_from(bytes).unwrap_or(u64::MAX);
        if value > self.limits.max_text_bytes {
            return Err(LimitHit {
                limit: "text",
                value,
                max: self.limits.max_text_bytes,
                unit: "bytes",
            });
        }
        Ok(())
    }

    /// The model elements counted so far.
    pub fn elements(&self) -> u64 {
        self.elements.load(Ordering::Relaxed)
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
