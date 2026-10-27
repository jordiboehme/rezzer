// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small pool of decoder threads on one shared queue, so a scan can stop waiting at its
//! time limit and a call left behind does not hold up the calls after it.
//!
//! The counts, all kept under the queue lock:
//! - `threads`: the live decoder threads
//! - `idle`: the threads that wait for a call
//! - `stuck`: the threads that run a call whose caller stopped waiting
//!
//! A new thread starts only when every live thread is stuck and fewer than [`MAX_STUCK`] are,
//! so there are at most [`MAX_STUCK`] threads and normally one. With [`MAX_STUCK`] calls
//! stuck, calls wait in the queue until one of them ends. A thread whose call ended takes
//! the next call; with none queued it leaves when another thread is idle, so the surplus
//! goes and the last thread stays.

use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::read::{self, Hit, Job};
use crate::{Limits, Picture, Runner, Scan, Stop};

/// The most calls left behind by their callers that may still run before calls wait for
/// them.
pub(crate) const MAX_STUCK: usize = 2;

/// How long an idle thread waits before it checks again whether it is surplus.
const IDLE_CHECK: Duration = Duration::from_secs(1);

/// Where a call is. Changed only under the queue lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Running,
    /// Its caller stopped waiting while it ran: its thread counts as stuck.
    Stuck,
    Done,
}

/// One call in the queue. The job sends its own answer. A call whose caller stopped
/// waiting before it ran is taken out of the queue, so it is skipped.
struct Task {
    job: Box<dyn FnOnce() + Send>,
    id: u64,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Task>,
    /// The phase of each call that a caller still waits for or a thread still runs.
    phases: Vec<(u64, Phase)>,
    next_id: u64,
    threads: usize,
    idle: usize,
    stuck: usize,
    /// The worker was dropped: idle threads leave.
    closed: bool,
}

impl State {
    fn phase(&mut self, id: u64) -> Option<&mut Phase> {
        self.phases
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, p)| p)
    }

    fn forget(&mut self, id: u64) {
        self.phases.retain(|(i, _)| *i != id);
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    /// The state. A poisoned lock is taken as it is: no code panics while it holds the lock,
    /// and the counts must never be lost.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The decoder threads of the process and their queue.
pub struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    /// The worker of the process, started on first use.
    pub fn global() -> &'static Worker {
        static WORKER: OnceLock<Worker> = OnceLock::new();
        WORKER.get_or_init(Worker::new)
    }

    /// A worker with no thread yet: the first call starts one.
    pub(crate) fn new() -> Worker {
        Worker {
            shared: Arc::new(Shared::default()),
        }
    }

    /// Scans `picture` like [`crate::decode`], with each decoder call on a worker thread.
    /// At the time limit the scan stops waiting and returns [`Stop::Time`] with the codes
    /// found until then; the call that runs is left to end on its own, and a new thread
    /// takes the next call, with at most [`MAX_STUCK`] such calls at once. A call left
    /// behind holds one tile, never the whole picture.
    pub fn decode(&self, picture: &Picture<'_>, limits: &Limits) -> Result<Scan, Stop> {
        crate::scan(picture, limits, &mut Remote(self))
    }

    /// Runs `f` on a worker thread and waits for it until `deadline`. At the deadline the
    /// caller stops waiting ([`Stop::Time`]): a call still queued is skipped, a call that
    /// runs is left to end on its own and its thread counts as stuck. A panic in `f` is
    /// [`Stop::Internal`].
    pub(crate) fn call<R: Send + 'static>(
        &self,
        f: Box<dyn FnOnce() -> R + Send>,
        deadline: Instant,
    ) -> Result<R, Stop> {
        let (reply, answer) = mpsc::sync_channel(1);
        let job = Box::new(move || {
            let got = panic::catch_unwind(AssertUnwindSafe(f)).map_err(|_| Stop::Internal);
            // The caller may have stopped waiting.
            let _ = reply.send(got);
        });
        let id = {
            let mut state = self.shared.lock();
            let id = state.next_id;
            state.next_id += 1;
            state.queue.push_back(Task { job, id });
            state.phases.push((id, Phase::Queued));
            if !grow(&self.shared, &mut state) {
                state.queue.retain(|t| t.id != id);
                state.forget(id);
                return Err(Stop::Internal);
            }
            self.shared.ready.notify_one();
            id
        };
        let wait = deadline.saturating_duration_since(Instant::now());
        let got = answer.recv_timeout(wait);
        let mut state = self.shared.lock();
        let phase = state.phase(id).copied();
        match got {
            Ok(found) => {
                state.forget(id);
                return found;
            }
            Err(RecvTimeoutError::Disconnected) => {
                state.forget(id);
                return Err(Stop::Internal);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        match phase {
            Some(Phase::Queued) => {
                // Skipped: taken out of the queue, and dropped once the lock is free.
                let left = state.queue.iter().position(|t| t.id == id);
                let task = left.and_then(|at| state.queue.remove(at));
                state.forget(id);
                drop(state);
                drop(task);
            }
            Some(Phase::Running) => {
                if let Some(p) = state.phase(id) {
                    *p = Phase::Stuck;
                }
                state.stuck += 1;
                // Calls queued behind it may now have no thread left to run them.
                if !state.queue.is_empty() && grow(&self.shared, &mut state) {
                    self.shared.ready.notify_one();
                }
            }
            Some(Phase::Done) => {
                // It ended just at the deadline: its answer is there.
                state.forget(id);
                drop(state);
                return answer.try_recv().unwrap_or(Err(Stop::Internal));
            }
            Some(Phase::Stuck) | None => state.forget(id),
        }
        Err(Stop::Time(Scan::default()))
    }

    /// The live decoder threads.
    #[cfg(test)]
    pub(crate) fn threads(&self) -> usize {
        self.shared.lock().threads
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.ready.notify_all();
    }
}

/// Starts a thread when every live thread is stuck and fewer than [`MAX_STUCK`] are.
/// False when a thread is needed and none can be had: no thread lives and the system gives
/// none.
fn grow(shared: &Arc<Shared>, state: &mut State) -> bool {
    if state.threads > state.stuck || state.stuck >= MAX_STUCK {
        return true;
    }
    state.threads += 1;
    let shared = shared.clone();
    let started = std::thread::Builder::new()
        .name("barcodes".into())
        .spawn(move || serve(&shared))
        .is_ok();
    if !started {
        state.threads -= 1;
    }
    started || state.threads > 0
}

/// Puts the counts right when a decoder thread unwinds. The job catches the panics of the
/// decoder, but a panic can still get out of it, for example from the drop of a panic
/// payload. The caller of that job then gets [`Stop::Internal`].
struct Unwind<'s> {
    shared: &'s Arc<Shared>,
    /// The call the thread runs.
    running: Option<u64>,
}

impl Drop for Unwind<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let mut state = self.shared.lock();
        state.threads -= 1;
        if let Some(id) = self.running {
            if state.phase(id).is_some_and(|p| *p == Phase::Stuck) {
                state.stuck -= 1;
            }
            state.forget(id);
        }
        // Calls queued now may have no thread left to run them.
        if !state.queue.is_empty() {
            grow(self.shared, &mut state);
            self.shared.ready.notify_one();
        }
    }
}

/// The loop of one decoder thread.
fn serve(shared: &Arc<Shared>) {
    let mut unwind = Unwind {
        shared,
        running: None,
    };
    let mut state = shared.lock();
    loop {
        if let Some(task) = state.queue.pop_front() {
            if let Some(p) = state.phase(task.id) {
                *p = Phase::Running;
            }
            drop(state);
            unwind.running = Some(task.id);
            // The job catches the decoder's panics and sends its answer.
            (task.job)();
            unwind.running = None;
            state = shared.lock();
            match state.phase(task.id) {
                Some(Phase::Stuck) => {
                    state.stuck -= 1;
                    state.forget(task.id);
                }
                Some(p) => *p = Phase::Done,
                None => {}
            }
            continue;
        }
        // No call queued: leave when another thread is idle, or the worker is gone.
        if state.idle > 0 || state.closed {
            state.threads -= 1;
            return;
        }
        state.idle += 1;
        state = shared
            .ready
            .wait_timeout(state, IDLE_CHECK)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
        state.idle -= 1;
    }
}

struct Remote<'w>(&'w Worker);

impl Runner for Remote<'_> {
    fn run(&mut self, job: Job, deadline: Instant) -> Result<Vec<Hit>, Stop> {
        if Instant::now() >= deadline {
            return Err(Stop::Time(Scan::default()));
        }
        self.0
            .call(Box::new(move || read::run(job)), deadline)
            .and_then(|found| found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The safety stop of the waits below: generous, so a loaded machine cannot make them
    /// fail.
    const SAFETY: Duration = Duration::from_secs(30);

    /// A call that says when it starts, then blocks until its release sends or is dropped.
    /// Returns the release, the start signal and the call. A call that is skipped, never
    /// run, is dropped, so its start signal ends without a message.
    fn blocked() -> (
        mpsc::Sender<()>,
        mpsc::Receiver<()>,
        Box<dyn FnOnce() -> u32 + Send>,
    ) {
        let (release, wait) = mpsc::channel::<()>();
        let (start, started) = mpsc::channel::<()>();
        (
            release,
            started,
            Box::new(move || {
                let _ = start.send(());
                let _ = wait.recv();
                1
            }),
        )
    }

    fn soon(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    /// Leaves one blocked call stuck on a thread and returns its release. A call whose
    /// thread was not scheduled before the deadline is skipped, not stuck: it is made
    /// again, so the outcome does not depend on the load of the machine.
    fn stick(worker: &Worker) -> mpsc::Sender<()> {
        let stop = Instant::now() + SAFETY;
        loop {
            let (release, started, f) = blocked();
            assert!(matches!(worker.call(f, soon(200)), Err(Stop::Time(_))));
            // Waits until the call runs (stuck) or is dropped (skipped).
            if started.recv().is_ok() {
                return release;
            }
            assert!(Instant::now() < stop, "no blocked call ever started");
        }
    }

    #[test]
    fn a_stuck_call_does_not_hold_up_the_next() {
        let worker = Worker::new();
        let release = stick(&worker);
        let b: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| 7);
        assert_eq!(worker.call(b, Instant::now() + SAFETY), Ok(7));
        // A is still blocked here: it is released only now.
        release.send(()).expect("A still waits");
    }

    #[test]
    fn stuck_threads_are_capped_and_go_away() {
        let worker = Worker::new();
        let mut releases: Vec<_> = (0..MAX_STUCK).map(|_| stick(&worker)).collect();
        // Each stuck call holds a thread, and no new one may start.
        assert_eq!(worker.threads(), MAX_STUCK);
        // The next blocked call waits in the queue and is skipped, never run.
        let (release, started, f) = blocked();
        assert!(matches!(worker.call(f, soon(200)), Err(Stop::Time(_))));
        assert!(started.recv().is_err(), "a skipped call never starts");
        releases.push(release);
        assert_eq!(worker.threads(), MAX_STUCK);
        // Every thread is stuck and no new one may start: the call waits and gives up.
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| 7);
        assert!(matches!(worker.call(f, soon(200)), Err(Stop::Time(_))));
        assert_eq!(worker.threads(), MAX_STUCK);
        // The skipped call's release finds no receiver.
        for release in releases {
            let _ = release.send(());
        }
        let stop = Instant::now() + SAFETY;
        while worker.threads() != 1 && Instant::now() < stop {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(worker.threads(), 1);
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| 7);
        assert_eq!(worker.call(f, Instant::now() + SAFETY), Ok(7));
    }

    #[test]
    fn a_panic_is_the_calls_internal_stop() {
        let worker = Worker::new();
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| panic!("a decoder bug"));
        assert_eq!(worker.call(f, Instant::now() + SAFETY), Err(Stop::Internal));
        assert_eq!(worker.threads(), 1);
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| 7);
        assert_eq!(worker.call(f, Instant::now() + SAFETY), Ok(7));
    }

    /// A panic payload whose drop panics too.
    struct Bomb;

    impl Drop for Bomb {
        fn drop(&mut self) {
            panic!("the payload's drop");
        }
    }

    #[test]
    fn a_thread_that_unwinds_keeps_the_counts_right() {
        let worker = Worker::new();
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| std::panic::panic_any(Bomb));
        assert_eq!(worker.call(f, Instant::now() + SAFETY), Err(Stop::Internal));
        // The thread is gone; the next call gets a new one, also when it comes before the
        // old thread has put its count right.
        let f: Box<dyn FnOnce() -> u32 + Send> = Box::new(|| 7);
        assert_eq!(worker.call(f, Instant::now() + SAFETY), Ok(7));
        assert_eq!(worker.threads(), 1);
    }
}
