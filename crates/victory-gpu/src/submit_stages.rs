//! Original per-stage timing for submission preparation (`VICTORY_SUBMIT_STAGES=1`).
//!
//! Off by default (`VICTORY_SUBMIT_STAGES` unset or `0`), incurring only a single
//! relaxed load of an initialized setting when disabled. Set `=1` for a 5-second
//! reporting interval, or `<N>` (`1..=60`) for every `N` seconds.
//!
//! Idea after Kelvaroq `0053-submit.patch` (stage breakdown of the submit worker);
//! original implementation with nested thread-local stage pausing so inner waits do
//! not double-count an outer preparation stage.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

/// Stages measured across submission preparation and dispatch.
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Querying write-watch / memory guards (`ask_once`, `ask_once_end`, `writes`).
    Watch = 0,
    /// Copying dirty host shadow ranges and staging uploads (`batch.flush`).
    Copies = 1,
    /// Preparing card-copy upload/hash passes (`card_before`).
    Hashes = 2,
    /// Synchronizing descriptor heaps and staged views (`sync_heaps`, `sync_end`, `make_waiting`).
    Descriptors = 3,
    /// Recording deferred commands, list preludes, and back-buffer snapshots.
    Record = 4,
    /// Driver submission calls (`ExecuteCommandLists` and `Signal`).
    Execute = 5,
    /// Waiting on fences, settles, or relay landings (`settle`, `relay_wait`).
    Wait = 6,
}

const STAGE_COUNT: usize = 7;

static STAGE_US: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static SUBMISSIONS: AtomicU64 = AtomicU64::new(0);
static LAST_REPORT_MS: AtomicU64 = AtomicU64::new(0);
static ACCUM_FRAMES: AtomicU64 = AtomicU64::new(0);

/// Returns reporting interval in milliseconds (`0` = disabled).
pub(crate) fn interval_ms() -> u64 {
    static MS: OnceLock<u64> = OnceLock::new();
    *MS.get_or_init(|| {
        let secs = match std::env::var("VICTORY_SUBMIT_STAGES") {
            Ok(v) => match v.trim() {
                "" | "0" => 0,
                "1" => 5,
                other => other.parse::<u64>().unwrap_or(0).clamp(1, 60),
            },
            Err(_) => 0,
        };
        if secs > 0 {
            tracing::info!(
                interval_secs = secs,
                "submission stage timing enabled (VICTORY_SUBMIT_STAGES)"
            );
        }
        secs * 1000
    })
}

#[derive(Clone, Copy)]
struct ActiveFrame {
    stage: Stage,
    started: Instant,
    paused_ns: u64,
}

thread_local! {
    static STACK: RefCell<Vec<ActiveFrame>> = const { RefCell::new(Vec::new()) };
}

/// Counts one prepared host submission when [`VICTORY_SUBMIT_STAGES`] is enabled.
pub(crate) fn count_submission() {
    if interval_ms() != 0 {
        SUBMISSIONS.fetch_add(1, Relaxed);
    }
}

/// RAII scope timing `stage` on the current thread. Pauses any outer active stage
/// while this guard lives so nested waits are attributed solely to [`Stage::Wait`].
pub(crate) struct StageGuard {
    active: bool,
}

pub(crate) fn enter(stage: Stage) -> StageGuard {
    if interval_ms() == 0 {
        return StageGuard { active: false };
    }
    let now = Instant::now();
    STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if let Some(outer) = stack.last_mut() {
            // Account outer elapsed up to `now`, then pause until inner exits.
            let elapsed = now.saturating_duration_since(outer.started).as_nanos() as u64;
            outer.paused_ns = outer.paused_ns.saturating_add(elapsed);
        }
        stack.push(ActiveFrame {
            stage,
            started: now,
            paused_ns: 0,
        });
    });
    StageGuard { active: true }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let now = Instant::now();
        STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            let Some(frame) = stack.pop() else {
                return;
            };
            let tail_ns = now.saturating_duration_since(frame.started).as_nanos() as u64;
            let total_us = frame.paused_ns.saturating_add(tail_ns) / 1000;
            STAGE_US[frame.stage as usize].fetch_add(total_us, Relaxed);
            if let Some(outer) = stack.last_mut() {
                outer.started = now;
            }
        });
    }
}

/// Accumulates `frames` from the 1-second frame meter and logs once every
/// `interval_ms()` milliseconds.
pub(crate) fn report(frames: f64) {
    let interval = interval_ms();
    if interval == 0 || frames <= 0.0 {
        return;
    }
    // Accumulate integer scaled frame count (x1000) to avoid float atomics.
    let add_milli = (frames * 1000.0).round() as u64;
    ACCUM_FRAMES.fetch_add(add_milli, Relaxed);

    let now_ms = crate::host::now_ms();
    let prev = LAST_REPORT_MS.load(Relaxed);
    if prev == 0 {
        let _ = LAST_REPORT_MS.compare_exchange(0, now_ms, Relaxed, Relaxed);
        return;
    }
    if now_ms.saturating_sub(prev) < interval {
        return;
    }
    if LAST_REPORT_MS
        .compare_exchange(prev, now_ms, Relaxed, Relaxed)
        .is_err()
    {
        return;
    }

    let frames_milli = ACCUM_FRAMES.swap(0, Relaxed);
    let total_frames = (frames_milli as f64 / 1000.0).max(1.0);
    let subs = SUBMISSIONS.swap(0, Relaxed);
    let us: [u64; STAGE_COUNT] = std::array::from_fn(|i| STAGE_US[i].swap(0, Relaxed));
    let ms = |i: usize| us[i] as f64 / 1000.0 / total_frames;
    let total_ms: f64 = us.iter().map(|&v| v as f64 / 1000.0 / total_frames).sum();

    tracing::info!(
        subs_per_frame = format_args!("{:.1}", subs as f64 / total_frames),
        total_ms = format_args!("{total_ms:.1}"),
        "submit stages/frame (ms): watch {:.1}, copies {:.1}, hashes {:.1}, descriptors {:.1}, record {:.1}, execute {:.1}, wait {:.1}",
        ms(Stage::Watch as usize),
        ms(Stage::Copies as usize),
        ms(Stage::Hashes as usize),
        ms(Stage::Descriptors as usize),
        ms(Stage::Record as usize),
        ms(Stage::Execute as usize),
        ms(Stage::Wait as usize),
    );
}
