//! Opt-in frame, chunk-mesh, upload, and startup-stage measurements.
//!
//! Set `NEPHELE_PERF=1` to collect metrics. Frame intervals and mesh-build
//! durations go into fixed 10-microsecond histograms (up to 100 ms); reported
//! percentiles are conservative upper bucket bounds. Each 30-second window is
//! reported separately, including a bad startup window.
//!
//! Startup stages are recorded for the first five seconds. Set
//! `NEPHELE_PERF_JSON=1` to emit one machine-readable JSON line per window; the
//! measurement script consumes those lines for its baseline file.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

const HISTOGRAM_BUCKET_WIDTH_NS: u64 = 10_000;
const HISTOGRAM_MAX_NS: u64 = 100_000_000;
const HISTOGRAM_BUCKET_COUNT: usize = (HISTOGRAM_MAX_NS / HISTOGRAM_BUCKET_WIDTH_NS) as usize + 1;
const REPORT_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_WINDOW: Duration = Duration::from_secs(5);
const FRAME_BUDGET_NS: u64 = 1_000_000_000 / 60;
const LONG_FRAME_NS: u64 = 100_000_000;

static ENABLED: OnceLock<bool> = OnceLock::new();
static JSON_ENABLED: OnceLock<bool> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();

/// Snapshot of resident state for one measurement window.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemorySample {
    pub chunks: u64,
    pub meshes: u64,
    pub gpu_buffer_bytes: u64,
    pub gpu_texture_bytes: u64,
    pub vram_estimate_bytes: u64,
    pub rss_bytes: u64,
}

/// Mark the start of the measured process, as early as possible.
pub fn start() {
    let _ = START.get_or_init(Instant::now);
}

/// Whether opt-in performance measurements are enabled for this process.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("NEPHELE_PERF")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    })
}

fn json_enabled() -> bool {
    *JSON_ENABLED.get_or_init(|| {
        std::env::var("NEPHELE_PERF_JSON")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    })
}

fn origin() -> Instant {
    *START.get_or_init(Instant::now)
}

/// Record the start-to-start interval between rendered frames.
pub fn record_frame_interval(interval: Duration) {
    if !enabled() {
        return;
    }

    let report = {
        let mut recorder = lock_recorder();
        recorder.record_frame_interval(interval)
    };

    if let Some(report) = report {
        report.emit();
    }
}

/// Record time spent building one column mesh on a mesher worker.
pub fn record_mesh_build_time(started_at: Instant, duration: Duration) {
    if !enabled() {
        return;
    }

    let mut recorder = lock_recorder();
    recorder.mesh_builds.record(duration);
    recorder.record_stage_span("initial_mesh_build", started_at, started_at + duration);
}

/// Record a synchronous chunk upload and its duration.
pub fn record_chunk_upload(started_at: Instant, duration: Duration) {
    if !enabled() {
        return;
    }

    let mut recorder = lock_recorder();
    recorder.chunk_uploads = recorder.chunk_uploads.saturating_add(1);
    recorder.chunk_upload_total = recorder.chunk_upload_total.saturating_add(duration);
    recorder.record_stage_span("chunk_upload", started_at, started_at + duration);
}

/// Resident set size of this process, when the platform exposes it cheaply.
pub fn process_rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(
                process: isize,
                counters: *mut ProcessMemoryCounters,
                size: u32,
            ) -> i32;
        }
        let mut counters: ProcessMemoryCounters = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                counters.cb,
            )
        };
        if ok != 0 {
            return counters.working_set_size as u64;
        }
        0
    }
    #[cfg(not(windows))]
    {
        // Linux exposes RSS in pages in statm; macOS/BSD return 0.
        let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
            return 0;
        };
        statm
            .split_whitespace()
            .nth(1)
            .and_then(|pages| pages.parse::<u64>().ok())
            .map(|pages| pages.saturating_mul(4096))
            .unwrap_or(0)
    }
}

/// Store the latest chunk/mesh/GPU/RSS snapshot for the next report.
pub fn record_memory(sample: MemorySample) {
    if !enabled() {
        return;
    }
    lock_recorder().memory = sample;
}

/// Record an arbitrary startup stage whose start and end are already known.
pub fn record_stage_span(stage: &'static str, started_at: Instant, ended_at: Instant) {
    if !enabled() {
        return;
    }

    lock_recorder().record_stage_span(stage, started_at, ended_at);
}

fn lock_recorder() -> std::sync::MutexGuard<'static, Recorder> {
    RECORDER
        .get_or_init(|| Mutex::new(Recorder::new(origin())))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy)]
struct FrameSample {
    start: Duration,
    end: Duration,
}

impl FrameSample {
    fn duration(self) -> Duration {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Clone, Copy)]
struct StageSpan {
    stage: &'static str,
    start: Duration,
    end: Duration,
}

impl StageSpan {
    fn duration(self) -> Duration {
        self.end.saturating_sub(self.start)
    }

    fn overlap_ns(self, frame: FrameSample) -> u64 {
        let start = self.start.max(frame.start);
        let end = self.end.min(frame.end);
        end.saturating_sub(start)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64
    }
}

struct Recorder {
    started_at: Instant,
    next_report_at: Instant,
    window: u64,
    frame_intervals: Histogram,
    mesh_builds: Histogram,
    frame_over_budget: u64,
    chunk_uploads: u64,
    chunk_upload_total: Duration,
    startup_frames: Vec<FrameSample>,
    startup_stages: Vec<StageSpan>,
    memory: MemorySample,
}

impl Recorder {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            next_report_at: started_at + REPORT_INTERVAL,
            window: 0,
            frame_intervals: Histogram::new(),
            mesh_builds: Histogram::new(),
            frame_over_budget: 0,
            chunk_uploads: 0,
            chunk_upload_total: Duration::ZERO,
            startup_frames: Vec::new(),
            startup_stages: Vec::new(),
            memory: MemorySample::default(),
        }
    }

    fn record_frame_interval(&mut self, interval: Duration) -> Option<WindowReport> {
        let now = Instant::now();
        let start = now.checked_sub(interval).unwrap_or(now);
        let start_offset = start.saturating_duration_since(self.started_at);
        let end_offset = now.saturating_duration_since(self.started_at);

        self.frame_intervals.record(interval);
        if interval.as_nanos() > u128::from(FRAME_BUDGET_NS) {
            self.frame_over_budget = self.frame_over_budget.saturating_add(1);
        }
        if start_offset < STARTUP_WINDOW {
            self.startup_frames.push(FrameSample {
                start: start_offset,
                end: end_offset,
            });
        }

        if now < self.next_report_at {
            return None;
        }

        let report = self.snapshot(now);
        self.frame_intervals.clear();
        self.mesh_builds.clear();
        self.frame_over_budget = 0;
        self.chunk_uploads = 0;
        self.chunk_upload_total = Duration::ZERO;
        self.startup_frames.clear();
        self.startup_stages.clear();
        self.window = self.window.saturating_add(1);
        while self.next_report_at <= now {
            self.next_report_at += REPORT_INTERVAL;
        }
        Some(report)
    }

    fn record_stage_span(&mut self, stage: &'static str, started_at: Instant, ended_at: Instant) {
        let start = started_at.saturating_duration_since(self.started_at);
        if start >= STARTUP_WINDOW {
            return;
        }
        let end = ended_at
            .saturating_duration_since(self.started_at)
            .max(start);
        self.startup_stages.push(StageSpan { stage, start, end });
    }

    fn snapshot(&self, now: Instant) -> WindowReport {
        let frame_samples = self.frame_intervals.samples;
        let p99_rank = percentile_rank(frame_samples, 99);
        let p99_within_budget = (frame_samples > 0)
            .then(|| frame_samples.saturating_sub(self.frame_over_budget) >= p99_rank);

        WindowReport {
            window: self.window + 1,
            elapsed_s: now.saturating_duration_since(self.started_at).as_secs_f64(),
            frame_samples,
            frame_p50_ms: duration_ms(self.frame_intervals.percentile_upper_ns(50)),
            frame_p99_ms: duration_ms(self.frame_intervals.percentile_upper_ns(99)),
            startup_p50_ms: (self.window == 0)
                .then(|| duration_ms(self.frame_intervals.percentile_upper_ns(50)))
                .flatten(),
            startup_p99_ms: (self.window == 0)
                .then(|| duration_ms(self.frame_intervals.percentile_upper_ns(99)))
                .flatten(),
            frame_over_budget: self.frame_over_budget,
            frame_over_100ms: self.frame_intervals.over_max_count(),
            p99_within_budget,
            mesh_build_samples: self.mesh_builds.samples,
            mesh_build_p50_ms: duration_ms(self.mesh_builds.percentile_upper_ns(50)),
            mesh_build_p99_ms: duration_ms(self.mesh_builds.percentile_upper_ns(99)),
            mesh_build_over_100ms: self.mesh_builds.over_max_count(),
            chunk_uploads: self.chunk_uploads,
            chunk_upload_total_ms: self.chunk_upload_total.as_secs_f64() * 1000.0,
            startup: (self.window == 0).then(|| self.startup_summary()),
            memory: self.memory,
        }
    }

    fn startup_summary(&self) -> StartupSummary {
        let long_frames: Vec<FrameSample> = self
            .startup_frames
            .iter()
            .copied()
            .filter(|frame| frame.duration().as_nanos() > u128::from(LONG_FRAME_NS))
            .collect();

        let mut stages: BTreeMap<&'static str, StageAccumulator> = BTreeMap::new();
        let mut stage_long_frames: BTreeMap<&'static str, BTreeSet<usize>> = BTreeMap::new();
        for span in &self.startup_stages {
            let entry = stages.entry(span.stage).or_default();
            entry.samples = entry.samples.saturating_add(1);
            entry.total_ns = entry
                .total_ns
                .saturating_add(span.duration().as_nanos().min(u128::from(u64::MAX)) as u64);
            entry.max_ns = entry
                .max_ns
                .max(span.duration().as_nanos().min(u128::from(u64::MAX)) as u64);

            for (index, frame) in long_frames.iter().enumerate() {
                let overlap = span.overlap_ns(*frame);
                if overlap > 0 {
                    entry.long_frame_overlap_ns =
                        entry.long_frame_overlap_ns.saturating_add(overlap);
                    stage_long_frames
                        .entry(span.stage)
                        .or_default()
                        .insert(index);
                }
            }
        }

        let long_frames = long_frames
            .into_iter()
            .map(|frame| LongFrame {
                start_ms: frame.start.as_secs_f64() * 1000.0,
                duration_ms: frame.duration().as_secs_f64() * 1000.0,
                stages: {
                    let mut names = Vec::new();
                    for span in &self.startup_stages {
                        if span.overlap_ns(frame) > 0
                            && !names.iter().any(|name: &String| name == span.stage)
                        {
                            names.push(span.stage.to_string());
                        }
                    }
                    names
                },
            })
            .collect();

        StartupSummary {
            window_s: STARTUP_WINDOW.as_secs_f64(),
            long_frame_threshold_ms: LONG_FRAME_NS as f64 / 1_000_000.0,
            long_frames,
            stages: stages
                .into_iter()
                .map(|(name, stage)| StageSummary {
                    name: name.to_string(),
                    samples: stage.samples,
                    total_ms: stage.total_ns as f64 / 1_000_000.0,
                    max_ms: stage.max_ns as f64 / 1_000_000.0,
                    long_frame_overlap_ms: stage.long_frame_overlap_ns as f64 / 1_000_000.0,
                    long_frames: stage_long_frames
                        .get(name)
                        .map_or(0, |frames| frames.len() as u64),
                })
                .collect(),
        }
    }
}

#[derive(Default)]
struct StageAccumulator {
    samples: u64,
    total_ns: u64,
    max_ns: u64,
    long_frame_overlap_ns: u64,
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

    fn clear(&mut self) {
        self.buckets.fill(0);
        self.samples = 0;
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

#[derive(Serialize)]
struct WindowReport {
    window: u64,
    elapsed_s: f64,
    frame_samples: u64,
    frame_p50_ms: Option<f64>,
    frame_p99_ms: Option<f64>,
    startup_p50_ms: Option<f64>,
    startup_p99_ms: Option<f64>,
    frame_over_budget: u64,
    frame_over_100ms: u64,
    p99_within_budget: Option<bool>,
    mesh_build_samples: u64,
    mesh_build_p50_ms: Option<f64>,
    mesh_build_p99_ms: Option<f64>,
    mesh_build_over_100ms: u64,
    chunk_uploads: u64,
    chunk_upload_total_ms: f64,
    startup: Option<StartupSummary>,
    memory: MemorySample,
}

#[derive(Serialize)]
struct StartupSummary {
    window_s: f64,
    long_frame_threshold_ms: f64,
    long_frames: Vec<LongFrame>,
    stages: Vec<StageSummary>,
}

#[derive(Serialize)]
struct LongFrame {
    start_ms: f64,
    duration_ms: f64,
    stages: Vec<String>,
}

#[derive(Serialize)]
struct StageSummary {
    name: String,
    samples: u64,
    total_ms: f64,
    max_ms: f64,
    long_frame_overlap_ms: f64,
    long_frames: u64,
}

impl WindowReport {
    fn emit(&self) {
        let p99_budget = match self.p99_within_budget {
            Some(true) => "PASS",
            Some(false) => "MISS",
            None => "N/A",
        };
        eprintln!(
            "[perf] window={} elapsed_s={:.1} frame_samples={} frame_ms_p50={} \
             frame_ms_p99={} startup_ms_p50={} startup_ms_p99={} budget_60fps_ms={:.3} \
             frame_over_budget={} frame_over_100ms={} p99_budget={} mesh_build_samples={} \
             mesh_build_ms_p50={} mesh_build_ms_p99={} chunk_uploads={} \
             chunk_upload_total_ms={:.3} chunks={} meshes={} gpu_buffer_mb={:.3} \
             gpu_texture_mb={:.3} vram_mb={:.3} rss_mb={:.3} histogram_bucket_us=10",
            self.window,
            self.elapsed_s,
            self.frame_samples,
            format_ms(self.frame_p50_ms),
            format_ms(self.frame_p99_ms),
            format_ms(self.startup_p50_ms),
            format_ms(self.startup_p99_ms),
            FRAME_BUDGET_NS as f64 / 1_000_000.0,
            self.frame_over_budget,
            self.frame_over_100ms,
            p99_budget,
            self.mesh_build_samples,
            format_ms(self.mesh_build_p50_ms),
            format_ms(self.mesh_build_p99_ms),
            self.chunk_uploads,
            self.chunk_upload_total_ms,
            self.memory.chunks,
            self.memory.meshes,
            self.memory.gpu_buffer_bytes as f64 / 1_048_576.0,
            self.memory.gpu_texture_bytes as f64 / 1_048_576.0,
            self.memory.vram_estimate_bytes as f64 / 1_048_576.0,
            self.memory.rss_bytes as f64 / 1_048_576.0,
        );

        if json_enabled() {
            if let Ok(json) = serde_json::to_string(self) {
                eprintln!("[perf-json] {json}");
            }
        }
    }
}

fn duration_ms(nanos: Option<u64>) -> Option<f64> {
    nanos.map(|nanos| nanos as f64 / 1_000_000.0)
}

fn format_ms(nanos: Option<f64>) -> String {
    nanos
        .map(|millis| format!("{millis:.3}"))
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
        let mut recorder = Recorder::new(Instant::now());
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

    #[test]
    fn a_long_startup_frame_names_the_stage_that_overlaps_it() {
        let started = Instant::now() - Duration::from_millis(120);
        let mut recorder = Recorder::new(started);
        recorder.record_stage_span(
            "texture_atlas_upload",
            started + Duration::from_millis(10),
            started + Duration::from_millis(70),
        );
        recorder.record_stage_span(
            "initial_mesh_build",
            started + Duration::from_millis(130),
            started + Duration::from_millis(140),
        );
        recorder.record_frame_interval(Duration::from_millis(120));

        let startup = recorder.snapshot(Instant::now()).startup.expect("startup");
        assert_eq!(startup.long_frames.len(), 1);
        assert_eq!(startup.long_frames[0].stages, vec!["texture_atlas_upload"]);
        let atlas = startup
            .stages
            .iter()
            .find(|stage| stage.name == "texture_atlas_upload")
            .expect("atlas stage");
        assert_eq!(atlas.long_frames, 1);
        assert_eq!(atlas.long_frame_overlap_ms, 60.0);
    }

    #[test]
    fn each_window_has_its_own_frame_and_mesh_samples() {
        let started = Instant::now();
        let mut recorder = Recorder::new(started);
        recorder.record_frame_interval(Duration::from_millis(10));
        recorder.mesh_builds.record(Duration::from_millis(2));
        recorder.chunk_uploads = 1;
        let first = recorder.snapshot(started + Duration::from_secs(30));
        assert_eq!(first.window, 1);
        assert_eq!(first.frame_samples, 1);
        assert_eq!(first.mesh_build_samples, 1);
        assert!(first.startup.is_some());

        recorder.frame_intervals.clear();
        recorder.mesh_builds.clear();
        recorder.chunk_uploads = 0;
        recorder.window = 1;
        let second = recorder.snapshot(started + Duration::from_secs(60));
        assert_eq!(second.window, 2);
        assert_eq!(second.frame_samples, 0);
        assert!(second.startup.is_none());
    }
}
