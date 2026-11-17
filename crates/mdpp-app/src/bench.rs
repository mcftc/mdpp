//! Built-in measurements for the Phase 0 gates (PLAN §7): run the real app, print numbers, exit.
//!
//! `MDPP_BENCH=startup`  — time from `main()` to the first painted frame (G7).
//! `MDPP_BENCH=typing:N` — type N characters into the middle of the document at a steady cadence
//!                         and report keystroke → paint latency percentiles (G1);
//!                         `typing:N:preview` does the same with the preview open.
//! `MDPP_BENCH=idle:S`   — after the caret stops blinking, count frames painted and CPU used
//!                         during S seconds without input (G7: an idle window must cost 0 %).
//! `MDPP_BENCH=check`    — end-to-end check through the real actions: type, Save, compare the
//!                         bytes on disk; open the preview, move to the middle, check scroll sync.
//!                         Destructive: run it on a scratch copy of a file.
//! `MDPP_BENCH=crash`    — panic after the first frame, to test crash reports.
//!
//! "Paint" is the moment a marker element placed after the editor is painted, i.e. the editor has
//! been laid out and painted for the frame that contains the keystroke.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Environment variable that selects a measurement.
pub const ENV: &str = "MDPP_BENCH";
/// Time between simulated keystrokes: ~33 characters per second, a fast typist.
pub const KEYSTROKE_INTERVAL: Duration = Duration::from_millis(30);
/// Quiet time before typing starts and before reporting, so startup work does not skew samples.
pub const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Startup,
    /// Number of keystrokes, and whether the preview is open.
    Typing(usize, bool),
    Idle(u64),
    Check,
    Crash,
}

impl Mode {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var(ENV).ok()?;
        match value.split_once(':') {
            None if value == "startup" => Some(Mode::Startup),
            None if value == "check" => Some(Mode::Check),
            None if value == "crash" => Some(Mode::Crash),
            Some(("typing", rest)) => {
                let (count, preview) = match rest.split_once(':') {
                    Some((count, "preview")) => (count, true),
                    _ => (rest, false),
                };
                count.parse().ok().map(|n| Mode::Typing(n, preview))
            }
            Some(("idle", secs)) => secs.parse().ok().map(Mode::Idle),
            _ => {
                eprintln!(
                    "{ENV}: expected `startup`, `check`, `typing:<count>[:preview]` or `idle:<seconds>`, got `{value}`"
                );
                None
            }
        }
    }
}

/// Shared between the typing loop and the paint marker.
#[derive(Debug, Default)]
pub struct Probe {
    /// Set when a keystroke is sent, taken by the next paint.
    pub pending: Option<Instant>,
    pub samples: Vec<Duration>,
    /// Start of the frame that follows a keystroke (set by a next-frame callback).
    pub frame_start: Option<Instant>,
    /// Frame start → paint: the CPU work of drawing a frame, excluding the wait for vsync.
    pub work: Vec<Duration>,
    pub first_paint: Option<Instant>,
    pub paints: usize,
}

pub type SharedProbe = Rc<RefCell<Probe>>;

impl Probe {
    /// Called from the paint marker.
    pub fn painted(&mut self) {
        let now = Instant::now();
        self.paints += 1;
        self.first_paint.get_or_insert(now);
        if let Some(sent) = self.pending.take() {
            self.samples.push(now - sent);
        }
        if let Some(start) = self.frame_start.take() {
            self.work.push(now - start);
        }
    }

    pub fn report(&self) -> String {
        format!(
            "keystroke->paint {} | frame work {}",
            percentiles(&self.samples),
            percentiles(&self.work)
        )
    }
}

fn percentiles(samples: &[Duration]) -> String {
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
    if ms.is_empty() {
        return "no samples".into();
    }
    ms.sort_by(f64::total_cmp);
    let pct = |p: f64| ms[((ms.len() - 1) as f64 * p).round() as usize];
    format!(
        "n={} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
        ms.len(),
        pct(0.50),
        pct(0.95),
        pct(0.99),
        ms[ms.len() - 1]
    )
}

/// CPU time (user + kernel) this process has used, where the OS exposes it cheaply.
pub fn process_cpu_time() -> Option<Duration> {
    // Linux: fields 14 and 15 of /proc/self/stat, in clock ticks (USER_HZ = 100 on Linux).
    const TICKS_PER_SECOND: u64 = 100;
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some(Duration::from_millis(ticks * 1000 / TICKS_PER_SECOND))
}
