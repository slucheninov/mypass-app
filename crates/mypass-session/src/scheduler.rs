//! Delayed jobs behind one trait, so autosave, auto-lock, clipboard clear and retries share a
//! single timing seam: production uses [`ThreadScheduler`] (one worker thread), tests use
//! [`ManualScheduler`] (a manual clock, jobs run inline on `advance`). Production code never
//! calls `thread::sleep` for a timing rule; every rule has a deterministic test.
//!
//! Two clocks matter. Delays run on a monotonic timeline (`Instant` in production), which on
//! macOS does not advance while the machine sleeps; wall time ([`Scheduler::wall_now`]) does.
//! Rules that must notice a long sleep (auto-lock on wake) compare wall times, and
//! [`ManualScheduler::jump_wall_clock`] simulates exactly that gap in tests.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// A boxed one-shot job.
pub type Job = Box<dyn FnOnce() + Send>;

/// Cancels a scheduled job. Cloning shares the flag; cancelling an already-run job is a no-op.
#[derive(Clone, Debug, Default)]
pub struct TaskHandle(Arc<AtomicBool>);

impl TaskHandle {
    /// The job will not run if it has not started yet.
    pub fn cancel(&self) {
        self.0.store(true, AtomicOrdering::SeqCst);
    }

    /// Whether [`TaskHandle::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(AtomicOrdering::SeqCst)
    }
}

/// Runs jobs after a delay. Jobs run one at a time, in due order (ties: scheduling order), never
/// while the scheduler holds its own lock, so a job may schedule further jobs.
pub trait Scheduler: Send + Sync {
    /// Runs `job` once `after` has elapsed on the monotonic timeline, unless cancelled first.
    fn schedule(&self, after: Duration, job: Job) -> TaskHandle;
    /// Current wall-clock time (the injected clock in tests).
    fn wall_now(&self) -> SystemTime;
    /// Position on the monotonic timeline the delays run on: time since an arbitrary fixed
    /// origin, never going backwards (`Instant` in production, the manual timeline in tests).
    /// Rules that must survive a wall clock set back (auto-lock) measure with it as well.
    fn monotonic_now(&self) -> Duration;
}

/// Locks a mutex, recovering the data if a panicking thread poisoned it (jobs are caught, so the
/// guarded state is always consistent between statements).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a job, containing a panic so one faulty job cannot stop the scheduler.
fn run_job(job: Job) {
    let _ = catch_unwind(AssertUnwindSafe(job));
}

// ---------------------------------------------------------------------------------------------
// ThreadScheduler

struct ThreadItem {
    due: Instant,
    seq: u64,
    job: Job,
    handle: TaskHandle,
}

impl PartialEq for ThreadItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for ThreadItem {}
impl PartialOrd for ThreadItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ThreadItem {
    /// Reversed so the max-heap pops the earliest due time (then the oldest) first.
    fn cmp(&self, other: &Self) -> Ordering {
        (other.due, other.seq).cmp(&(self.due, self.seq))
    }
}

#[derive(Default)]
struct ThreadState {
    heap: BinaryHeap<ThreadItem>,
    seq: u64,
    shutdown: bool,
}

#[derive(Default)]
struct ThreadShared {
    state: Mutex<ThreadState>,
    wake: Condvar,
}

/// Above this many queued jobs, `schedule` drops cancelled ones (an idle timer re-armed on every
/// user action would otherwise queue one dead job per action until its due time).
const PURGE_THRESHOLD: usize = 64;

/// The production scheduler: one named worker thread waiting on a `Condvar` for the earliest due
/// job of a binary heap. Jobs run on the worker, outside the lock. Dropping the last handle stops
/// the worker (queued jobs are discarded). A panicking job is contained and the worker goes on.
///
/// Cancellation is checked when the job is popped, not while it runs: a `cancel` that lands
/// between that check and the call still lets the job run once. Jobs must therefore re-check the
/// session state they act on (for example, an auto-lock job checks that the session is still
/// unlocked and still idle) instead of trusting that a cancelled timer never fires.
pub struct ThreadScheduler {
    shared: Arc<ThreadShared>,
    /// Origin of [`Scheduler::monotonic_now`].
    origin: Instant,
}

impl ThreadScheduler {
    /// Starts the worker thread. Errors only when the OS refuses to create a thread.
    pub fn new(thread_name: &str) -> std::io::Result<Arc<Self>> {
        let shared = Arc::new(ThreadShared::default());
        let worker = shared.clone();
        std::thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || worker_loop(&worker))?;
        Ok(Arc::new(Self {
            shared,
            origin: Instant::now(),
        }))
    }
}

fn worker_loop(shared: &ThreadShared) {
    let mut state = lock(&shared.state);
    loop {
        if state.shutdown {
            return;
        }
        let now = Instant::now();
        let next_due = state.heap.peek().map(|item| item.due);
        match next_due {
            None => {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            Some(due) if due > now => {
                state = shared
                    .wake
                    .wait_timeout(state, due - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            Some(_) => {
                if let Some(item) = state.heap.pop() {
                    if !item.handle.is_cancelled() {
                        drop(state);
                        run_job(item.job);
                        state = lock(&shared.state);
                    }
                }
            }
        }
    }
}

impl Scheduler for ThreadScheduler {
    fn schedule(&self, after: Duration, job: Job) -> TaskHandle {
        let handle = TaskHandle::default();
        let now = Instant::now();
        // A delay too large for `Instant` means "never" in practice: park it a year out.
        let due = now
            .checked_add(after)
            .or_else(|| now.checked_add(Duration::from_secs(86_400 * 365)))
            .unwrap_or(now);
        let mut state = lock(&self.shared.state);
        if state.heap.len() >= PURGE_THRESHOLD {
            state.heap.retain(|item| !item.handle.is_cancelled());
        }
        state.seq += 1;
        let seq = state.seq;
        state.heap.push(ThreadItem {
            due,
            seq,
            job,
            handle: handle.clone(),
        });
        drop(state);
        self.shared.wake.notify_one();
        handle
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn monotonic_now(&self) -> Duration {
        self.origin.elapsed()
    }
}

impl Drop for ThreadScheduler {
    fn drop(&mut self) {
        lock(&self.shared.state).shutdown = true;
        self.shared.wake.notify_one();
    }
}

// ---------------------------------------------------------------------------------------------
// ManualScheduler

struct ManualItem {
    due: Duration,
    seq: u64,
    job: Job,
    handle: TaskHandle,
}

struct ManualState {
    /// Position on the monotonic timeline (time since construction).
    elapsed: Duration,
    wall: SystemTime,
    seq: u64,
    items: Vec<ManualItem>,
}

/// The test scheduler: nothing happens until [`ManualScheduler::advance`], which runs every job
/// that becomes due, inline and in order, moving both clocks to each job's due time first. A
/// panicking job panics `advance` (test support only; production uses [`ThreadScheduler`]).
pub struct ManualScheduler {
    state: Mutex<ManualState>,
}

impl ManualScheduler {
    /// A scheduler whose wall clock starts at `start`.
    pub fn new(start: SystemTime) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ManualState {
                elapsed: Duration::ZERO,
                wall: start,
                seq: 0,
                items: Vec::new(),
            }),
        })
    }

    /// Moves both clocks forward by `by`, running due jobs (oldest due first) on the way. Jobs
    /// scheduled by a running job run in the same call when they fall due within it.
    pub fn advance(&self, by: Duration) {
        let target = lock(&self.state).elapsed.saturating_add(by);
        loop {
            let job = {
                let mut st = lock(&self.state);
                st.items.retain(|item| !item.handle.is_cancelled());
                let next = st
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.due <= target)
                    .min_by_key(|(_, item)| (item.due, item.seq))
                    .map(|(i, _)| i);
                match next {
                    Some(i) => {
                        let item = st.items.remove(i);
                        let step = item.due.saturating_sub(st.elapsed);
                        st.elapsed = st.elapsed.max(item.due);
                        st.wall = st.wall.checked_add(step).unwrap_or(st.wall);
                        item.job
                    }
                    None => {
                        let step = target.saturating_sub(st.elapsed);
                        st.elapsed = target;
                        st.wall = st.wall.checked_add(step).unwrap_or(st.wall);
                        return;
                    }
                }
            };
            // Unlike the production scheduler, a panic is NOT contained: an `assert!` inside a
            // scheduled test job must fail the test. The job is already removed and both clocks
            // already moved, and no lock is held here, so the queue stays consistent and a later
            // `advance` still runs the remaining jobs. Only tests use `ManualScheduler`.
            if let Err(panic) = catch_unwind(AssertUnwindSafe(job)) {
                resume_unwind(panic);
            }
        }
    }

    /// Moves only the wall clock (the monotonic timeline stands still, as during system sleep);
    /// no job runs.
    pub fn jump_wall_clock(&self, by: Duration) {
        let mut st = lock(&self.state);
        st.wall = st.wall.checked_add(by).unwrap_or(st.wall);
    }

    /// Sets only the wall clock, in either direction (a user or NTP changing the system time);
    /// the monotonic timeline stands still and no job runs.
    pub fn set_wall_clock(&self, at: SystemTime) {
        lock(&self.state).wall = at;
    }

    /// Number of scheduled jobs that are neither run nor cancelled.
    pub fn pending(&self) -> usize {
        lock(&self.state)
            .items
            .iter()
            .filter(|item| !item.handle.is_cancelled())
            .count()
    }
}

impl Scheduler for ManualScheduler {
    fn schedule(&self, after: Duration, job: Job) -> TaskHandle {
        let handle = TaskHandle::default();
        let mut st = lock(&self.state);
        st.seq += 1;
        let item = ManualItem {
            due: st.elapsed.saturating_add(after),
            seq: st.seq,
            job,
            handle: handle.clone(),
        };
        st.items.push(item);
        handle
    }

    fn wall_now(&self) -> SystemTime {
        lock(&self.state).wall
    }

    fn monotonic_now(&self) -> Duration {
        lock(&self.state).elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn start() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    #[test]
    fn manual_monotonic_clock_ignores_wall_clock_changes() {
        let s = ManualScheduler::new(start());
        s.advance(Duration::from_secs(10));
        s.set_wall_clock(start() - Duration::from_secs(3600));
        s.jump_wall_clock(Duration::from_secs(5));
        assert_eq!(s.monotonic_now(), Duration::from_secs(10));
        assert_eq!(s.wall_now(), start() - Duration::from_secs(3595));
        s.advance(Duration::from_secs(1));
        assert_eq!(s.monotonic_now(), Duration::from_secs(11));
    }

    #[test]
    fn thread_monotonic_clock_never_goes_backwards() {
        let s = ThreadScheduler::new("test-monotonic").unwrap();
        let a = s.monotonic_now();
        let b = s.monotonic_now();
        assert!(b >= a);
    }

    type Log = Arc<Mutex<Vec<u32>>>;

    fn recorder() -> (Log, impl Fn(u32) -> Job) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let make = move |n: u32| -> Box<dyn FnOnce() + Send> {
            let l = l.clone();
            Box::new(move || l.lock().unwrap().push(n))
        };
        (log, make)
    }

    #[test]
    fn manual_scheduler_runs_due_jobs_in_order() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        s.schedule(Duration::from_secs(5), job(5));
        s.schedule(Duration::from_secs(1), job(1));
        s.schedule(Duration::from_secs(3), job(3));
        assert_eq!(s.pending(), 3);
        s.advance(Duration::from_secs(5));
        assert_eq!(*log.lock().unwrap(), vec![1, 3, 5]);
        assert_eq!(s.pending(), 0);
        assert_eq!(s.wall_now(), start() + Duration::from_secs(5));
    }

    #[test]
    fn equal_due_times_run_oldest_first() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        s.schedule(Duration::from_secs(1), job(1));
        s.schedule(Duration::from_secs(1), job(2));
        s.advance(Duration::from_secs(1));
        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn cancelled_job_never_runs() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        let h = s.schedule(Duration::from_secs(1), job(1));
        s.schedule(Duration::from_secs(2), job(2));
        h.cancel();
        assert!(h.is_cancelled());
        assert_eq!(s.pending(), 1);
        s.advance(Duration::from_secs(10));
        assert_eq!(*log.lock().unwrap(), vec![2]);
    }

    #[test]
    fn advance_short_of_due_time_runs_nothing() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        s.schedule(Duration::from_secs(5), job(5));
        s.advance(Duration::from_millis(4_999));
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(s.pending(), 1);
        s.advance(Duration::from_millis(1));
        assert_eq!(*log.lock().unwrap(), vec![5]);
    }

    #[test]
    fn jump_wall_clock_moves_time_without_running_jobs() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        s.schedule(Duration::from_secs(5), job(5));
        s.jump_wall_clock(Duration::from_secs(3600));
        assert_eq!(s.wall_now(), start() + Duration::from_secs(3600));
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(s.pending(), 1);
        // The monotonic timeline did not move: the job still needs its full 5 s.
        s.advance(Duration::from_secs(4));
        assert!(log.lock().unwrap().is_empty());
        s.advance(Duration::from_secs(1));
        assert_eq!(*log.lock().unwrap(), vec![5]);
    }

    #[test]
    fn job_may_schedule_another_job() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        let s2 = s.clone();
        let second = job(2);
        let l = log.clone();
        s.schedule(
            Duration::from_secs(1),
            Box::new(move || {
                l.lock().unwrap().push(1);
                s2.schedule(Duration::from_secs(1), second);
            }),
        );
        s.advance(Duration::from_secs(2));
        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn thread_scheduler_runs_job_after_delay() {
        let s = ThreadScheduler::new("mypass-test-scheduler").unwrap();
        let (tx, rx) = mpsc::channel();
        s.schedule(
            Duration::from_millis(10),
            Box::new(move || tx.send(42).unwrap()),
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(42));
    }

    #[test]
    fn thread_scheduler_runs_jobs_in_due_order() {
        let s = ThreadScheduler::new("mypass-test-scheduler").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        s.schedule(
            Duration::from_millis(60),
            Box::new(move || tx.send(2).unwrap()),
        );
        s.schedule(
            Duration::from_millis(5),
            Box::new(move || tx2.send(1).unwrap()),
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(1));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(2));
    }

    #[test]
    fn thread_scheduler_cancel_prevents_run() {
        let s = ThreadScheduler::new("mypass-test-scheduler").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let h = s.schedule(
            Duration::from_millis(20),
            Box::new(move || tx.send(1).unwrap()),
        );
        h.cancel();
        // A later job proves the worker passed the cancelled one's due time.
        s.schedule(
            Duration::from_millis(80),
            Box::new(move || tx2.send(2).unwrap()),
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(2));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn thread_scheduler_survives_a_panicking_job() {
        let s = ThreadScheduler::new("mypass-test-scheduler").unwrap();
        let (tx, rx) = mpsc::channel();
        s.schedule(Duration::from_millis(1), Box::new(|| panic!("job panic")));
        s.schedule(
            Duration::from_millis(20),
            Box::new(move || tx.send(7).unwrap()),
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(7));
    }

    #[test]
    fn manual_scheduler_advance_propagates_a_job_panic() {
        let s = ManualScheduler::new(start());
        let (log, job) = recorder();
        s.schedule(
            Duration::from_secs(1),
            Box::new(|| panic!("job assert failed")),
        );
        s.schedule(Duration::from_secs(2), job(2));
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| s.advance(Duration::from_secs(5))));
        assert!(r.is_err(), "a failing assert in a job must fail the test");
        // The queue is consistent: the panicking job is gone, the later one still runs.
        assert_eq!(s.pending(), 1);
        assert_eq!(s.wall_now(), start() + Duration::from_secs(1));
        s.advance(Duration::from_secs(4));
        assert_eq!(*log.lock().unwrap(), vec![2]);
    }
}
