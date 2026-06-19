//! Token-savings telemetry. Port of `src/stats.ts` (← semble `stats.py`).
//!
//! Appends one JSONL record per search/find_related call to
//! `~/.csp/savings.jsonl`, and renders an aggregated report. Writes are
//! best-effort — telemetry never throws into a live search.
//!
//! Time bucketing uses UTC `YYYY-MM-DD` (compared lexicographically, which is
//! chronological); `now_secs` is injected so summaries/reports are testable.

use std::collections::{BTreeMap, HashMap};
use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::search::SearchResult;
use crate::types::CallType;

/// Default stats file: `~/.csp/savings.jsonl`.
pub fn default_stats_file() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".csp").join("savings.jsonl")
}

/// Current wall-clock time in seconds since the Unix epoch.
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn call_type_str(call: CallType) -> &'static str {
    match call {
        CallType::Search => "search",
        CallType::FindRelated => "find_related",
    }
}

/// Per-bucket aggregate counters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BucketStats {
    pub calls: u64,
    pub snippet_chars: u64,
    pub file_chars: u64,
    pub saved_chars: u64,
}

impl BucketStats {
    /// Record a call and its character counts (`saved` clamped to ≥ 0).
    pub fn add(&mut self, snippet_chars: u64, file_chars: u64) {
        self.calls += 1;
        self.snippet_chars += snippet_chars;
        self.file_chars += file_chars;
        self.saved_chars += file_chars.saturating_sub(snippet_chars);
    }
}

/// Aggregated savings: time buckets + per-call-type counts.
#[derive(Debug, Clone, PartialEq)]
pub struct SavingsSummary {
    /// Keyed `"Today"` / `"Last 7 days"` / `"All time"`.
    pub buckets: BTreeMap<String, BucketStats>,
    pub call_type_counts: BTreeMap<String, u64>,
}

#[derive(Serialize, Deserialize)]
struct StatsRecord {
    ts: f64,
    call: String,
    results: usize,
    snippet_chars: u64,
    file_chars: u64,
}

/// UTF-16 code-unit length (matches JS `String.length`).
fn utf16_len(s: &str) -> u64 {
    s.encode_utf16().count() as u64
}

/// Append one telemetry record. Best-effort: any I/O error is swallowed.
pub fn save_search_stats(
    stats_file: &Path,
    results: &[SearchResult],
    call_type: CallType,
    file_sizes: &HashMap<String, u64>,
) {
    let snippet_chars: u64 = results.iter().map(|r| utf16_len(&r.chunk.content)).sum();
    let mut unique_paths: Vec<&str> = Vec::new();
    for r in results {
        if !unique_paths.contains(&r.chunk.file_path.as_str()) {
            unique_paths.push(r.chunk.file_path.as_str());
        }
    }
    let file_chars: u64 = unique_paths
        .iter()
        .filter_map(|p| file_sizes.get(*p).copied())
        .sum();

    let record = StatsRecord {
        ts: now_secs(),
        call: call_type_str(call_type).to_string(),
        results: results.len(),
        snippet_chars,
        file_chars,
    };

    let _ = write_record(stats_file, &record);
}

fn write_record(stats_file: &Path, record: &StatsRecord) -> std::io::Result<()> {
    if let Some(dir) = stats_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stats_file)?;
    writeln!(file, "{json}")
}

/// Delete the savings file (not truncate), so `savings` falls back to the
/// "No stats yet" message. Best-effort.
pub fn clear_savings(stats_file: &Path) -> (PathBuf, bool) {
    if !stats_file.exists() {
        return (stats_file.to_path_buf(), false);
    }
    match std::fs::remove_file(stats_file) {
        Ok(()) => (stats_file.to_path_buf(), true),
        Err(_) => (stats_file.to_path_buf(), false),
    }
}

/// `civil_from_days` (Howard Hinnant): days-since-epoch → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// UTC `YYYY-MM-DD` for a Unix timestamp in seconds.
fn ymd_utc(timestamp_seconds: f64) -> String {
    let days = (timestamp_seconds / 86_400.0).floor() as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Aggregate `savings.jsonl` into a [`SavingsSummary`]. Malformed/NaN lines are
/// skipped; a missing file yields an empty summary.
pub fn build_savings_summary(stats_file: &Path, now: f64) -> SavingsSummary {
    let today = ymd_utc(now);
    let seven_days_ago = ymd_utc(now - 7.0 * 24.0 * 60.0 * 60.0);

    let mut buckets: BTreeMap<String, BucketStats> = BTreeMap::new();
    buckets.insert("Today".to_string(), BucketStats::default());
    buckets.insert("Last 7 days".to_string(), BucketStats::default());
    buckets.insert("All time".to_string(), BucketStats::default());
    let mut call_type_counts: BTreeMap<String, u64> = BTreeMap::new();

    let Ok(raw) = std::fs::read_to_string(stats_file) else {
        return SavingsSummary {
            buckets,
            call_type_counts,
        };
    };

    for line in raw.split('\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<StatsRecord>(line) else {
            continue;
        };
        if record.ts.is_nan() {
            continue;
        }

        *call_type_counts.entry(record.call.clone()).or_insert(0) += 1;

        let day = ymd_utc(record.ts);
        let in_today = day == today;
        let in_last7 = day > seven_days_ago;

        buckets
            .get_mut("All time")
            .unwrap()
            .add(record.snippet_chars, record.file_chars);
        if in_last7 {
            buckets
                .get_mut("Last 7 days")
                .unwrap()
                .add(record.snippet_chars, record.file_chars);
        }
        if in_today {
            buckets
                .get_mut("Today")
                .unwrap()
                .add(record.snippet_chars, record.file_chars);
        }
    }

    SavingsSummary {
        buckets,
        call_type_counts,
    }
}

fn use_color() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").ok().as_deref() != Some("dumb")
        && std::io::stdout().is_terminal()
}

fn color(code: &str, text: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn color_ratio(pct: i64, enabled: bool) -> String {
    let code = if pct >= 80 {
        "32"
    } else if pct >= 50 {
        "33"
    } else {
        "31"
    };
    color(code, &format!("{pct}%"), enabled)
}

fn format_saved_tokens(saved: u64) -> String {
    if saved >= 1_000_000 {
        format!("~{:.1}M", saved as f64 / 1_000_000.0)
    } else if saved >= 1000 {
        format!("~{:.1}k", saved as f64 / 1000.0)
    } else {
        format!("~{saved}")
    }
}

fn format_calls(calls: u64) -> String {
    if calls >= 1000 {
        format!("{:.1}k", calls as f64 / 1000.0)
    } else {
        calls.to_string()
    }
}

fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - len))
    }
}

/// Render a token-savings report. Returns the "No stats yet" message when the
/// file is missing. `verbose` adds the per-call-type breakdown.
pub fn format_savings_report(stats_file: &Path, verbose: bool, now: f64) -> String {
    if !stats_file.exists() {
        return "No stats yet. Run a search first.".to_string();
    }

    let summary = build_savings_summary(stats_file, now);
    let enabled = use_color();
    let bar_width = 24usize;
    let border_width = 72usize;
    let heavy_line = format!(
        "  {}",
        color("38;5;244", &"═".repeat(border_width), enabled)
    );
    let light_line = format!(
        "  {}",
        color("38;5;244", &"─".repeat(border_width), enabled)
    );

    let all_time = &summary.buckets["All time"];
    let total_saved_tokens = all_time.saved_chars / 4;
    let overall_pct = if all_time.file_chars > 0 {
        ((all_time.saved_chars as f64 / all_time.file_chars as f64) * 100.0).round() as i64
    } else {
        0
    };
    let eff_filled = ((overall_pct as f64 / 100.0) * bar_width as f64).round() as usize;
    let eff_filled = eff_filled.min(bar_width);
    let efficiency_bar = color("32", &"█".repeat(eff_filled), enabled)
        + &color("38;5;244", &"░".repeat(bar_width - eff_filled), enabled);

    let mut lines: Vec<String> = vec![
        String::new(),
        format!("  {}", color("1;36", "Csp Token Savings", enabled)),
        heavy_line.clone(),
        String::new(),
        format!(
            "  {}  {}  ({})",
            color("1", "Total saved:", enabled),
            color(
                "1;33",
                &format!("{} tokens", format_saved_tokens(total_saved_tokens)),
                enabled
            ),
            color_ratio(overall_pct, enabled)
        ),
        format!(
            "  {}  {}",
            color("1", "Total calls:", enabled),
            color("1;33", &format_calls(all_time.calls), enabled)
        ),
        format!(
            "  {}  {}  {}",
            color("1", "Efficiency:", enabled),
            efficiency_bar,
            color_ratio(overall_pct, enabled)
        ),
        String::new(),
        format!("  {}", color("1", "By Period", enabled)),
        light_line.clone(),
        format!(
            "  {}  {}  {}  Ratio",
            pad_right("Period", 14),
            pad_left("Calls", 8),
            pad_left("Saved", 14)
        ),
        light_line.clone(),
    ];

    // Render in the fixed order Today / Last 7 days / All time.
    for label in ["Today", "Last 7 days", "All time"] {
        let bucket = &summary.buckets[label];
        let saved_tokens = bucket.saved_chars / 4;
        let saved_str = format!("{} tokens", format_saved_tokens(saved_tokens));
        let calls_str = format_calls(bucket.calls);
        let (row_bar, ratio_str) = if bucket.file_chars > 0 {
            let ratio = bucket.saved_chars as f64 / bucket.file_chars as f64;
            let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);
            (
                color("32", &"█".repeat(filled), enabled)
                    + &color("38;5;244", &"░".repeat(bar_width - filled), enabled),
                color_ratio((ratio * 100.0).round() as i64, enabled),
            )
        } else {
            (
                color("38;5;244", &"░".repeat(bar_width), enabled),
                color("38;5;244", "–", enabled),
            )
        };
        lines.push(format!(
            "  {}  {}  {}  {}  {}",
            color("1", &pad_right(label, 14), enabled),
            color("1;33", &pad_left(&calls_str, 8), enabled),
            color("1;33", &pad_left(&saved_str, 14), enabled),
            row_bar,
            ratio_str
        ));
    }

    if verbose && !summary.call_type_counts.is_empty() {
        lines.push(String::new());
        lines.push(format!("  {}", color("1", "By Call Type", enabled)));
        lines.push(light_line.clone());
        lines.push(format!(
            "  {}  {}  {}  Share",
            pad_right("#", 4),
            pad_right("Call type", 16),
            pad_left("Calls", 8)
        ));
        lines.push(light_line.clone());
        let total: u64 = summary.call_type_counts.values().sum();
        let mut sorted: Vec<(&String, &u64)> = summary.call_type_counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (i, (call_type, count)) in sorted.into_iter().enumerate() {
            let share = if total > 0 {
                *count as f64 / total as f64
            } else {
                0.0
            };
            let filled = ((share * 16.0).round() as usize).clamp(1, 16);
            let bar = color("32", &"█".repeat(filled), enabled)
                + &color("38;5;244", &"░".repeat(16 - filled), enabled);
            lines.push(format!(
                "  {}  {}  {}  {}  {}",
                color("38;5;244", &pad_right(&format!("{}.", i + 1), 4), enabled),
                pad_right(call_type, 16),
                color("1;33", &pad_left(&format_calls(*count), 8), enabled),
                bar,
                color(
                    "38;5;244",
                    &pad_left(&format!("{}%", (share * 100.0).round() as i64), 4),
                    enabled
                )
            ));
        }
    }

    lines.push(heavy_line);
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DAY: f64 = 24.0 * 60.0 * 60.0;

    fn result(content: &str, file_path: &str) -> SearchResult {
        SearchResult {
            chunk: crate::types::Chunk {
                content: content.to_string(),
                file_path: file_path.to_string(),
                start_line: 1,
                end_line: 1,
                language: None,
            },
            score: 1.0,
        }
    }

    fn sizes(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(p, s)| ((*p).to_string(), *s)).collect()
    }

    #[test]
    fn bucket_add_accumulates_and_clamps() {
        let mut b = BucketStats::default();
        b.add(100, 400);
        b.add(100, 400);
        assert_eq!(b.calls, 2);
        assert_eq!(b.snippet_chars, 200);
        assert_eq!(b.file_chars, 800);
        assert_eq!(b.saved_chars, 600);
    }

    #[test]
    fn bucket_add_no_negative_saved() {
        let mut b = BucketStats::default();
        b.add(500, 100);
        assert_eq!(b.saved_chars, 0);
        assert_eq!(b.snippet_chars, 500);
        assert_eq!(b.file_chars, 100);
    }

    #[test]
    fn save_appends_one_record() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let results = vec![result("hello world", "a.ts"), result("foo bar baz", "b.ts")];
        save_search_stats(
            &file,
            &results,
            CallType::Search,
            &sizes(&[("a.ts", 100), ("b.ts", 200)]),
        );

        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1);
        let record: StatsRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(record.call, "search");
        assert_eq!(record.results, 2);
        assert_eq!(record.snippet_chars, 22);
        assert_eq!(record.file_chars, 300);
    }

    #[test]
    fn save_dedups_file_chars_per_path() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let results = vec![result("abc", "a.ts"), result("def", "a.ts")];
        save_search_stats(&file, &results, CallType::Search, &sizes(&[("a.ts", 100)]));
        let content = std::fs::read_to_string(&file).unwrap();
        let record: StatsRecord = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(record.file_chars, 100);
        assert_eq!(record.snippet_chars, 6);
    }

    #[test]
    fn save_ignores_unknown_paths() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let results = vec![result("x", "a.ts"), result("y", "missing.ts")];
        save_search_stats(&file, &results, CallType::Search, &sizes(&[("a.ts", 100)]));
        let content = std::fs::read_to_string(&file).unwrap();
        let record: StatsRecord = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(record.file_chars, 100);
    }

    #[test]
    fn two_calls_two_lines() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        save_search_stats(
            &file,
            &[result("a", "a.ts")],
            CallType::Search,
            &sizes(&[("a.ts", 10)]),
        );
        save_search_stats(
            &file,
            &[result("b", "b.ts")],
            CallType::FindRelated,
            &sizes(&[("b.ts", 10)]),
        );
        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let r1: StatsRecord = serde_json::from_str(lines[0]).unwrap();
        let r2: StatsRecord = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(r1.call, "search");
        assert_eq!(r2.call, "find_related");
    }

    #[test]
    fn summary_missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let summary = build_savings_summary(&dir.path().join("none.jsonl"), 1_000_000.0);
        assert_eq!(summary.buckets["All time"].calls, 0);
        assert!(summary.call_type_counts.is_empty());
    }

    #[test]
    fn summary_parses_and_skips_malformed() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let now = 1_700_000_000.0;
        let lines = format!(
            "{{\"ts\":{now},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n\
             not json\n\
             {{\"ts\":{now},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n\
             {{\"ts\":{now},\"call\":\"find_related\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n"
        );
        std::fs::write(&file, lines).unwrap();
        let summary = build_savings_summary(&file, now);
        assert_eq!(summary.buckets["All time"].calls, 3);
        assert_eq!(summary.call_type_counts.get("search"), Some(&2));
        assert_eq!(summary.call_type_counts.get("find_related"), Some(&1));
    }

    #[test]
    fn summary_skips_nan_ts() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let now = 1_700_000_000.0;
        // serde_json can't emit NaN, so simulate a hand-written NaN line + valid one.
        let lines = format!(
            "{{\"ts\":NaN,\"call\":\"search\",\"results\":1,\"snippet_chars\":1,\"file_chars\":1}}\n\
             {{\"ts\":{now},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n"
        );
        std::fs::write(&file, lines).unwrap();
        let summary = build_savings_summary(&file, now);
        assert_eq!(summary.buckets["All time"].calls, 1);
        assert_eq!(summary.call_type_counts.get("search"), Some(&1));
    }

    #[test]
    fn summary_time_buckets() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let now = 1_700_000_000.0;
        let old = now - 8.0 * DAY;
        let lines = format!(
            "{{\"ts\":{now},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n\
             {{\"ts\":{old},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n"
        );
        std::fs::write(&file, lines).unwrap();
        let summary = build_savings_summary(&file, now);
        assert_eq!(summary.buckets["All time"].calls, 2);
        assert_eq!(summary.buckets["Last 7 days"].calls, 1);
        assert_eq!(summary.buckets["Today"].calls, 1);
    }

    #[test]
    fn clear_deletes_existing() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        let (_, cleared) = clear_savings(&file);
        assert!(cleared);
        assert!(!file.exists());

        let (_, cleared2) = clear_savings(&file);
        assert!(!cleared2);
    }

    #[test]
    fn report_no_stats_message() {
        let dir = tempdir().unwrap();
        let msg = format_savings_report(&dir.path().join("none.jsonl"), false, 1_700_000_000.0);
        assert_eq!(msg, "No stats yet. Run a search first.");
    }

    #[test]
    fn report_contains_header() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("savings.jsonl");
        let now = 1_700_000_000.0;
        std::fs::write(
            &file,
            format!("{{\"ts\":{now},\"call\":\"search\",\"results\":1,\"snippet_chars\":10,\"file_chars\":40}}\n"),
        )
        .unwrap();
        let report = format_savings_report(&file, false, now);
        assert!(report.contains("Csp Token Savings"));
        assert!(report.contains("By Period"));
    }

    #[test]
    fn ymd_utc_known_dates() {
        assert_eq!(ymd_utc(0.0), "1970-01-01");
        assert_eq!(ymd_utc(1_700_000_000.0), "2023-11-14");
    }
}
