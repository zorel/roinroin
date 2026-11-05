//! Opt-in, low-overhead profiling for the UI thread.
//!
//! Enable it with `--profile` or `ROINROIN_PROFILE=1`. Every few seconds a
//! summary is printed to stderr (message rate, view rate, analysis cost), and
//! individual slow updates/views/analyses are logged as they happen. This is
//! meant to diagnose transient CPU spikes without a debugger: the view rate
//! immediately shows when the UI is rebuilding too often, while the analysis
//! counters show when rebuilding the conversation index dominates.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub const PROFILE_ENV: &str = "ROINROIN_PROFILE";

const REPORT_INTERVAL: Duration = Duration::from_secs(5);
const SLOW_UPDATE: Duration = Duration::from_millis(40);
const SLOW_VIEW: Duration = Duration::from_millis(20);
const SLOW_ANALYSIS: Duration = Duration::from_millis(40);
const MAX_SLOW_LOGS: u32 = 8;

static ENABLED: OnceLock<bool> = OnceLock::new();
static STATS: OnceLock<Mutex<Stats>> = OnceLock::new();

struct Stats {
    interval_start: Instant,
    last_report: Option<Instant>,
    updates: u64,
    update_nanos: u64,
    update_max_nanos: u64,
    views: u64,
    view_nanos: u64,
    view_max_nanos: u64,
    analyses: u64,
    analysis_nanos: u64,
    analysis_max_nanos: u64,
    last_posts: usize,
    last_conversations: usize,
    message_counts: Vec<(&'static str, u64)>,
    slow_update_logs: u32,
    slow_view_logs: u32,
    slow_analysis_logs: u32,
}

impl Stats {
    fn new() -> Self {
        Self {
            interval_start: Instant::now(),
            last_report: None,
            updates: 0,
            update_nanos: 0,
            update_max_nanos: 0,
            views: 0,
            view_nanos: 0,
            view_max_nanos: 0,
            analyses: 0,
            analysis_nanos: 0,
            analysis_max_nanos: 0,
            last_posts: 0,
            last_conversations: 0,
            message_counts: Vec::new(),
            slow_update_logs: 0,
            slow_view_logs: 0,
            slow_analysis_logs: 0,
        }
    }
}

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var_os(PROFILE_ENV)
            .map(|value| {
                let value = value.to_string_lossy();
                !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
            })
            .unwrap_or(false)
    })
}

/// Start a measurement, or return `None` when profiling is disabled.
pub fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

pub fn record_update(started: Option<Instant>, label: &'static str) {
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed();
    let mut lines = Vec::new();
    {
        let mut stats = lock();
        stats.updates += 1;
        stats.update_nanos += elapsed.as_nanos() as u64;
        stats.update_max_nanos = stats.update_max_nanos.max(elapsed.as_nanos() as u64);
        if let Some((_, count)) = stats
            .message_counts
            .iter_mut()
            .find(|(name, _)| *name == label)
        {
            *count += 1;
        } else {
            stats.message_counts.push((label, 1));
        }
        if elapsed >= SLOW_UPDATE && stats.slow_update_logs < MAX_SLOW_LOGS {
            stats.slow_update_logs += 1;
            lines.push(format!(
                "[profile] slow update {label}: {:.1} ms",
                millis(elapsed)
            ));
        }
        if let Some(report) = report_if_due(&mut stats) {
            lines.push(report);
        }
    }
    for line in lines {
        eprintln!("{line}");
    }
}

pub fn record_view(started: Option<Instant>, posts: usize, conversations: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed();
    let mut lines = Vec::new();
    {
        let mut stats = lock();
        stats.views += 1;
        stats.view_nanos += elapsed.as_nanos() as u64;
        stats.view_max_nanos = stats.view_max_nanos.max(elapsed.as_nanos() as u64);
        stats.last_posts = posts;
        stats.last_conversations = conversations;
        if elapsed >= SLOW_VIEW && stats.slow_view_logs < MAX_SLOW_LOGS {
            stats.slow_view_logs += 1;
            lines.push(format!(
                "[profile] slow view: {:.1} ms ({posts} posts, {conversations} conversations)",
                millis(elapsed)
            ));
        }
        if let Some(report) = report_if_due(&mut stats) {
            lines.push(report);
        }
    }
    for line in lines {
        eprintln!("{line}");
    }
}

pub fn record_analysis(started: Option<Instant>, posts: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed();
    let mut lines = Vec::new();
    {
        let mut stats = lock();
        stats.analyses += 1;
        stats.analysis_nanos += elapsed.as_nanos() as u64;
        stats.analysis_max_nanos = stats.analysis_max_nanos.max(elapsed.as_nanos() as u64);
        stats.last_posts = posts;
        if elapsed >= SLOW_ANALYSIS && stats.slow_analysis_logs < MAX_SLOW_LOGS {
            stats.slow_analysis_logs += 1;
            lines.push(format!(
                "[profile] slow analysis: {:.1} ms ({posts} posts)",
                millis(elapsed)
            ));
        }
        if let Some(report) = report_if_due(&mut stats) {
            lines.push(report);
        }
    }
    for line in lines {
        eprintln!("{line}");
    }
}

fn lock() -> MutexGuard<'static, Stats> {
    STATS
        .get_or_init(|| Mutex::new(Stats::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn report_if_due(stats: &mut Stats) -> Option<String> {
    let now = Instant::now();
    match stats.last_report {
        None => {
            stats.last_report = Some(now);
            return None;
        }
        Some(last) if now.duration_since(last) < REPORT_INTERVAL => return None,
        Some(_) => {}
    }

    let seconds = now
        .duration_since(stats.interval_start)
        .as_secs_f64()
        .max(0.001);
    let mut top = stats.message_counts.clone();
    top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let top = top
        .iter()
        .take(6)
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    let report = format!(
        "[profile] {seconds:.1}s: update {:.0}/s avg {:.2} ms max {:.1} ms | \
         view {:.0}/s avg {:.2} ms max {:.1} ms | analysis {} avg {:.1} ms max {:.1} ms \
         ({} posts, {} conversations) | {top}",
        stats.updates as f64 / seconds,
        average(stats.update_nanos, stats.updates),
        millis_nanos(stats.update_max_nanos),
        stats.views as f64 / seconds,
        average(stats.view_nanos, stats.views),
        millis_nanos(stats.view_max_nanos),
        stats.analyses,
        average(stats.analysis_nanos, stats.analyses),
        millis_nanos(stats.analysis_max_nanos),
        stats.last_posts,
        stats.last_conversations,
    );
    *stats = Stats::new();
    Some(report)
}

fn average(total_nanos: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total_nanos as f64 / count as f64 / 1_000_000.0
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn millis_nanos(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.0
}
