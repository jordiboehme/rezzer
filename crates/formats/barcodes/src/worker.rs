// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One shared worker thread for decoder calls, so a scan can stop waiting at its time limit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::read::{self, Hit, Job};
use crate::{Limits, Picture, Runner, Scan, Stop};

/// One call for the worker: the job, whether its caller stopped waiting, where the answer
/// goes.
struct Task {
    job: Job,
    cancelled: Arc<AtomicBool>,
    reply: SyncSender<Result<Vec<Hit>, Stop>>,
}

/// The one worker thread of the process. Calls wait in its queue, so a call that runs past
/// its caller's time limit delays the calls behind it, but never starts a second thread.
/// The queue holds at most one call for each thread that scans, and a call whose caller
/// stopped waiting is skipped.
pub struct Worker {
    queue: Mutex<Option<Sender<Task>>>,
}

impl Worker {
    /// The worker of the process, started on first use.
    pub fn global() -> &'static Worker {
        static WORKER: OnceLock<Worker> = OnceLock::new();
        WORKER.get_or_init(|| Worker {
            queue: Mutex::new(start()),
        })
    }

    /// Scans `picture` like [`crate::decode`], with each decoder call on the worker thread.
    /// At the time limit the scan stops waiting and returns [`Stop::Time`] with the codes
    /// found until then; the call that runs is abandoned and ends on its own. It holds one
    /// tile, never the whole picture.
    pub fn decode(&self, picture: &Picture<'_>, limits: &Limits) -> Result<Scan, Stop> {
        crate::scan(picture, limits, &mut Remote(self))
    }

    fn send(&self, task: Task) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        let Some(tx) = queue.as_ref() else {
            return false;
        };
        match tx.send(task) {
            Ok(()) => true,
            Err(mpsc::SendError(task)) => {
                // The thread is gone (it cannot panic, but a start can fail): start again.
                *queue = start();
                queue.as_ref().is_some_and(|tx| tx.send(task).is_ok())
            }
        }
    }
}

/// Starts the worker thread, `None` when the system gives no thread.
fn start() -> Option<Sender<Task>> {
    let (tx, rx) = mpsc::channel::<Task>();
    std::thread::Builder::new()
        .name("barcodes".into())
        .spawn(move || {
            for task in rx {
                if task.cancelled.load(Ordering::Relaxed) {
                    continue;
                }
                let found = read::run(task.job);
                // The caller may have stopped waiting.
                let _ = task.reply.send(found);
            }
        })
        .ok()
        .map(|_| tx)
}

struct Remote<'w>(&'w Worker);

impl Runner for Remote<'_> {
    fn run(&mut self, job: Job, deadline: Instant) -> Result<Vec<Hit>, Stop> {
        let now = Instant::now();
        if now >= deadline {
            return Err(Stop::Time(Scan::default()));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, answer) = mpsc::sync_channel(1);
        let task = Task {
            job,
            cancelled: cancelled.clone(),
            reply,
        };
        if !self.0.send(task) {
            return Err(Stop::Internal);
        }
        match answer.recv_timeout(deadline - now) {
            Ok(found) => found,
            Err(RecvTimeoutError::Timeout) => {
                cancelled.store(true, Ordering::Relaxed);
                Err(Stop::Time(Scan::default()))
            }
            Err(RecvTimeoutError::Disconnected) => Err(Stop::Internal),
        }
    }
}
