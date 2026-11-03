//! Opt-in frame, chunk-mesh, and upload measurements for the client.
//!
//! Set `NEPHELE_PERF=1` to collect metrics. Frame intervals and mesh build
//! durations are recorded into fixed 10-microsecond histograms (up to 100 ms);
//! reported percentile values are conservative upper bucket bounds. Reports
//! are cumulative and emitted every 30 seconds so p99 has a useful sample size.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const HISTOGRAM_BUCKET_WIDTH_NS: u64 = 10_000;
const HISTOGRAM_MAX_NS: u64 = 100_000_000;
const HISTOGRAM_BUCKET_COUNT: usize = (HISTOGRAM_MAX_NS / HISTOGRAM_BUCKET_WIDTH_NS) as usize + 1;
const REPORT_INTERVAL: Duration = Duration::from_secs(30);
const FRAME_BUDGET_NS: u64 = 1_000_000_000 / 60;

static ENABLED: OnceLock<bool> = OnceLock::new();
static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();

/// Whether opt-in performance measurements are enabled for this process.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("NEPHELE_PERF")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    })
}

/// Record the start-to-start interval between rendered frames.
pub fn record_frame_interval(interval: Duration) {
    if !enabled() {
        return;
    }

    let report = {
        let mut recorder = lock_recorder();
        recorder.record_frame_interval(interval);
        let now = Instant::now();
        if now >= recorder.next_report_at {
            while recorder.next_report_at <= now {
                recorder.next_report_at += REPORT_INTERVAL;
            }
            Some(recorder.snapshot(now))
        } else {
            None
        }
    };

    if let Some(report) = report {
        report.emit();
    }
}

/// Record time spent building one column mesh on a mesher worker.
pub fn record_mesh_build_time(duration: Duration) {
    if !enabled() {
        return;
    }

    lock_recorder().mesh_builds.record(duration);
}

/// Count a non-empty chunk mesh submitted to the renderer for upload.
pub fn record_chunk_upload() {
    if !enabled() {
        return;
    }

    let mut recorder = lock_recorder();
    recorder.chunk_uploads = recorder.chunk_uploads.saturating_add(1);
}

fn lock_recorder() -> std::sync::MutexGuard<'static, Recorder> {
    RECORDER
        .get_or_init(|| Mutex::new(Recorder::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Recorder {
    started_at: Instant,
    next_report_at: Instant,
    frame_intervals: Histogram,
    mesh_builds: Histogram,
    frame_over_budget: u64,
    chunk_uploads: u64,
}

impl Recorder {
    fn new() -> Self {
        let started_at = Instant::now();
        Self {
            started_at,
            next_report_at: started_at + REPORT_INTERVAL,
            frame_intervals: Histogram::new(),
            mesh_builds: Histogram::new(),
            frame_over_budget: 0,
            chunk_uploads: 0,
        }
    }

    fn record_frame_interval(&mut self, interval: Duration) {
        self.frame_intervals.record(interval);
        if interval.as_nanos() > u128::from(FRAME_BUDGET_NS) {
            self.frame_over_budget = self.frame_over_budget.saturating_add(1);
        }
    }

    fn snapshot(&self, now: Instant) -> PerfReport {
        let frame_samples = self.frame_intervals.samples;
        let p99_rank = percentile_rank(frame_samples, 99);
        let p99_within_budget = (frame_samples > 0)
            .then(|| frame_samples.saturating_sub(self.frame_over_budget) >= p99_rank);

        PerfReport {
            elapsed: now.saturating_duration_since(self.started_at),
            frame_samples,
            frame_p50_ns: self.frame_intervals.percentile_upper_ns(50),
            frame_p99_ns: self.frame_intervals.percentile_upper_ns(99),
            frame_over_budget: self.frame_over_budget,
            p99_within_budget,
            mesh_build_samples: self.mesh_builds.samples,
            mesh_build_p50_ns: self.mesh_builds.percentile_upper_ns(50),
            mesh_build_p99_ns: self.mesh_builds.percentile_upper_ns(99),
            frame_over_100ms: self.frame_intervals.over_max_count(),
            mesh_build_over_100ms: self.mesh_builds.over_max_count(),
            chunk_uploads: self.chunk_uploads,
        }
    }
}

struct Histogram {
    buckets: Vec<u64>,
    samples: u64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: vec![0; HISTOGRAM_BUCKET_COUNT],
            samples: 0,
        }
    }

    fn record(&mut self, duration: Duration) {
        let nanos = duration.as_nanos().min(u128::from(u64::MAX)) as u64;
        let bucket = if nanos == 0 {
            0
        } else {
            ((nanos - 1) / HISTOGRAM_BUCKET_WIDTH_NS) as usize
        }
        .min(HISTOGRAM_BUCKET_COUNT - 1);

        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.samples = self.samples.saturating_add(1);
    }

    /// Nearest-rank percentile, rounded to the upper edge of its 10 μs bucket.
    fn percentile_upper_ns(&self, percentile: u64) -> Option<u64> {
        if self.samples == 0 {
            return None;
        }

        let rank = percentile_rank(self.samples, percentile);
        let mut cumulative = 0_u64;
        for (index, count) in self.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*count);
            if cumulative >= rank {
                return Some(
                    ((index as u64 + 1) * HISTOGRAM_BUCKET_WIDTH_NS).min(HISTOGRAM_MAX_NS),
                );
            }
        }

        Some(HISTOGRAM_MAX_NS)
    }

    fn over_max_count(&self) -> u64 {
        self.buckets[HISTOGRAM_BUCKET_COUNT - 1]
    }
}

fn percentile_rank(samples: u64, percentile: u64) -> u64 {
    (u128::from(samples) * u128::from(percentile)).div_ceil(100) as u64
}

struct PerfReport {
    elapsed: Duration,
    frame_samples: u64,
    frame_p50_ns: Option<u64>,
    frame_p99_ns: Option<u64>,
    frame_over_budget: u64,
    p99_within_budget: Option<bool>,
    mesh_build_samples: u64,
    mesh_build_p50_ns: Option<u64>,
    mesh_build_p99_ns: Option<u64>,
    frame_over_100ms: u64,
    mesh_build_over_100ms: u64,
    chunk_uploads: u64,
}

impl PerfReport {
    fn emit(self) {
        let p99_budget = match self.p99_within_budget {
            Some(true) => "PASS",
            Some(false) => "MISS",
            None => "N/A",
        };
        eprintln!(
            "[perf] report elapsed_s={:.1} frame_samples={} frame_interval_ms_p50={} \
             frame_interval_ms_p99={} budget_60fps_ms={:.3} frame_over_budget={} \
             p99_budget={} frame_over_100ms={} mesh_build_samples={} \
             mesh_build_ms_p50={} mesh_build_ms_p99={} mesh_build_over_100ms={} \
             chunk_uploads={} histogram_bucket_us=10",
            self.elapsed.as_secs_f64(),
            self.frame_samples,
            format_ms(self.frame_p50_ns),
            format_ms(self.frame_p99_ns),
            FRAME_BUDGET_NS as f64 / 1_000_000.0,
            self.frame_over_budget,
            p99_budget,
            self.frame_over_100ms,
            self.mesh_build_samples,
            format_ms(self.mesh_build_p50_ns),
            format_ms(self.mesh_build_p99_ns),
            self.mesh_build_over_100ms,
            self.chunk_uploads,
        );
    }
}

fn format_ms(nanos: Option<u64>) -> String {
    nanos
        .map(|nanos| format!("{:.3}", nanos as f64 / 1_000_000.0))
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_percentiles_use_nearest_rank_and_upper_bucket_edges() {
        let mut histogram = Histogram::new();
        for millis in [1, 2, 3, 4, 5] {
            histogram.record(Duration::from_millis(millis));
        }

        assert_eq!(histogram.percentile_upper_ns(50), Some(3_000_000));
        assert_eq!(histogram.percentile_upper_ns(99), Some(5_000_000));

        histogram.record(Duration::from_nanos(16_666_667));
        assert_eq!(histogram.percentile_upper_ns(99), Some(16_670_000));
    }

    #[test]
    fn empty_histogram_has_no_percentiles() {
        let histogram = Histogram::new();

        assert_eq!(histogram.percentile_upper_ns(50), None);
        assert_eq!(histogram.percentile_upper_ns(99), None);
    }

    #[test]
    fn p99_budget_uses_the_nearest_rank_not_an_average() {
        let mut recorder = Recorder::new();
        for _ in 0..99 {
            recorder.record_frame_interval(Duration::from_nanos(FRAME_BUDGET_NS));
        }
        recorder.record_frame_interval(Duration::from_nanos(FRAME_BUDGET_NS + 1));
        assert_eq!(
            recorder.snapshot(Instant::now()).p99_within_budget,
            Some(true)
        );

        recorder.record_frame_interval(Duration::from_nanos(FRAME_BUDGET_NS + 1));
        assert_eq!(
            recorder.snapshot(Instant::now()).p99_within_budget,
            Some(false)
        );
    }

    #[test]
    fn histogram_reports_samples_over_its_range() {
        let mut histogram = Histogram::new();
        histogram.record(Duration::from_nanos(HISTOGRAM_MAX_NS));
        histogram.record(Duration::from_nanos(HISTOGRAM_MAX_NS + 1));

        assert_eq!(histogram.over_max_count(), 1);
        assert_eq!(histogram.percentile_upper_ns(99), Some(HISTOGRAM_MAX_NS));
    }
}
