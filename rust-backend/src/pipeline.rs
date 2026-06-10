use crate::run_store::RunStore;
use crate::schema::{ViewerPoint, ViewerRun, ViewerRunSummary};
use crate::viewer_runs::display_x_from_raw;
use csv::WriterBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, SystemTime};
use time::format_description::well_known::Rfc3339;
use time::{Date, Duration as TimeDuration, OffsetDateTime, Time, UtcOffset};

const PIPELINE_SOURCE: &str = "minecraft-latest-log";
const PIPELINE_STRUCTURE: &str = "latest.log Pos/Motion chat broadcast";
const PIPELINE_RUN_ID_BASE: u64 = 930000;
const DEFAULT_TARGET_SPEED: f64 = 0.5;
const DEFAULT_TARGET_DWELL_TICKS: usize = 2;
const DEFAULT_MAX_CONTIGUOUS_DX: f64 = 2.0;
const DEFAULT_MAX_CONTIGUOUS_LOG_GAP_SECONDS: f64 = 1.5;
const LOCK_STALE_AFTER_SECONDS: u64 = 120;

fn default_target_speed() -> f64 {
    DEFAULT_TARGET_SPEED
}

fn default_target_dwell_ticks() -> usize {
    DEFAULT_TARGET_DWELL_TICKS
}

#[derive(Clone, Debug, Deserialize)]
struct PipelineConfigFile {
    #[serde(alias = "windowTitleContains")]
    window_title_contains: String,
    #[serde(alias = "pollSeconds")]
    poll_seconds: f64,
    #[serde(alias = "runGapSeconds")]
    run_gap_seconds: f64,
    #[serde(alias = "xResetTolerance")]
    x_reset_tolerance: f64,
    #[allow(dead_code)]
    #[serde(alias = "twoGtTargetDx")]
    two_gt_target_dx: f64,
    #[allow(dead_code)]
    #[serde(alias = "twoGtTolerance")]
    two_gt_tolerance: f64,
    #[serde(alias = "rawLogPath")]
    raw_log_path: String,
    #[serde(alias = "parsedCsvPath")]
    parsed_csv_path: String,
    #[serde(alias = "summaryCsvPath")]
    summary_csv_path: String,
    #[serde(alias = "latestSummaryPath")]
    latest_summary_path: String,
    #[serde(alias = "plotsDir")]
    plots_dir: String,
    #[serde(alias = "runsDir")]
    runs_dir: String,
    #[serde(alias = "viewerDataDir")]
    viewer_data_dir: String,
    #[serde(alias = "deletedRunsPath")]
    deleted_runs_path: String,
    #[serde(alias = "analysisLockPath")]
    analysis_lock_path: String,
    #[serde(alias = "statePath")]
    state_path: String,
    #[serde(default, alias = "candidateKeywords")]
    candidate_keywords: Vec<String>,
    #[serde(default = "default_max_contiguous_dx", alias = "maxContiguousDx")]
    max_contiguous_dx: f64,
    #[serde(
        default = "default_max_contiguous_log_gap_seconds",
        alias = "maxContiguousLogGapSeconds"
    )]
    max_contiguous_log_gap_seconds: f64,
}

#[derive(Clone, Debug)]
struct PipelineConfig {
    window_title_contains: String,
    poll_seconds: f64,
    run_gap_seconds: f64,
    x_reset_tolerance: f64,
    raw_log_path: PathBuf,
    parsed_csv_path: PathBuf,
    summary_csv_path: PathBuf,
    latest_summary_path: PathBuf,
    plots_dir: PathBuf,
    runs_dir: PathBuf,
    merged_viewer_data_dir: PathBuf,
    deleted_runs_path: PathBuf,
    analysis_lock_path: PathBuf,
    state_path: PathBuf,
    candidate_keywords: Vec<String>,
    max_contiguous_dx: f64,
    max_contiguous_log_gap_seconds: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct RawRecord {
    #[serde(default)]
    captured_at: String,
    #[serde(default)]
    #[allow(dead_code)]
    console_pid: Option<u32>,
    #[serde(default)]
    line: String,
}

#[derive(Clone, Debug, Serialize)]
struct Sample {
    source_index: usize,
    captured_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_time: Option<String>,
    run_id: i32,
    tick_index: usize,
    x_raw: f64,
    x: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    derived_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    z: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vz: Option<f64>,
    raw_line: String,
}

#[derive(Clone, Debug)]
struct ExtractedSample {
    x: f64,
    y: Option<f64>,
    z: Option<f64>,
    speed: Option<f64>,
    vx: Option<f64>,
    vy: Option<f64>,
    vz: Option<f64>,
}

#[derive(Clone, Debug)]
struct DedupeRun {
    collapsed_signature: Vec<f64>,
    end_log_time: Option<String>,
    end_x_raw: f64,
    samples: Vec<Sample>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SummaryRow {
    run_id: i32,
    sample_count: usize,
    start_x_raw: f64,
    end_x_raw: f64,
    start_x: f64,
    end_x: f64,
    duration_gt: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_logged_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_derived_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overall_avg_derived_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_speed_x_gt_3: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    two_gt_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    two_gt_hit_rate_x_gt_3: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_block_two_gt_dwell_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    long_per_block_two_gt_dwell_hit_rate: Option<f64>,
    #[serde(default)]
    dwell_blocks: usize,
    #[serde(default)]
    dwell_failures: usize,
    #[serde(default = "default_target_speed")]
    target_speed: f64,
    #[serde(default = "default_target_dwell_ticks")]
    target_dwell_ticks: usize,
    deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_z: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_z: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_log_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_log_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_moving_log_time: Option<String>,
    #[serde(default)]
    original_pos_motion_line_count: usize,
    #[serde(default)]
    initial_stationary_line_count: usize,
    #[serde(default)]
    removed_initial_stationary_line_count: usize,
    #[serde(default)]
    kept_initial_stationary_line_count: usize,
    #[serde(default)]
    log_path: String,
    #[serde(default)]
    note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSummary {
    #[serde(default, alias = "raw_records")]
    raw_records: usize,
    #[serde(default, alias = "merged_records")]
    merged_records: usize,
    #[serde(default, alias = "parsed_samples")]
    parsed_samples: usize,
    #[serde(default)]
    runs: usize,
    #[serde(default, alias = "active_runs")]
    active_runs: usize,
    #[serde(default, alias = "latest_run")]
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_run: Option<SummaryRow>,
    #[serde(default, alias = "plotting_available")]
    plotting_available: bool,
    #[serde(default, alias = "candidate_lines")]
    candidate_lines: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PipelineState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_cursor_y: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_capture_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    analysis: Option<AnalysisSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    last_visible_lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn main_cli(argv: &[String]) -> Result<(), String> {
    match argv.first().map(String::as_str) {
        Some("analyze") => cmd_analyze(),
        Some("status") => cmd_status(),
        Some("delete-run") => {
            let run_id = argv
                .get(1)
                .ok_or_else(|| "pipeline delete-run requires a run id".to_string())?
                .parse::<i32>()
                .map_err(|error| format!("invalid pipeline run id: {error}"))?;
            cmd_delete_run(run_id)
        }
        Some("restore-run") => {
            let run_id = argv
                .get(1)
                .ok_or_else(|| "pipeline restore-run requires a run id".to_string())?
                .parse::<i32>()
                .map_err(|error| format!("invalid pipeline run id: {error}"))?;
            cmd_restore_run(run_id)
        }
        Some("monitor") => {
            let once = argv.iter().any(|arg| arg == "--once");
            cmd_monitor(once)
        }
        Some(other) => Err(format!("unknown pipeline command: {other}")),
        None => Err(pipeline_usage()),
    }
}

pub fn pipeline_usage() -> String {
    "Usage: item-waterway-solver pipeline [analyze|status|delete-run <id>|restore-run <id>|monitor [--once]]".to_string()
}

fn cmd_analyze() -> Result<(), String> {
    let config = load_config()?;
    let summary = analyze(&config)?;
    print_json(&summary)
}

fn cmd_status() -> Result<(), String> {
    let config = load_config()?;
    let state = load_state(&config.state_path)?;
    print_json(&state)
}

fn cmd_delete_run(run_id: i32) -> Result<(), String> {
    let config = load_config()?;
    cmd_delete_run_with_config(run_id, &config)
}

fn cmd_delete_run_with_config(run_id: i32, config: &PipelineConfig) -> Result<(), String> {
    let mut deleted_runs = read_deleted_runs_any(&config.deleted_runs_path)?;
    deleted_runs.insert(run_id);
    write_deleted_runs(&config.deleted_runs_path, &deleted_runs)?;
    let analysis = analyze(&config)?;
    print_json(&json!({
        "deleted_run_id": run_id,
        "analysis": analysis
    }))
}

fn cmd_restore_run(run_id: i32) -> Result<(), String> {
    let config = load_config()?;
    cmd_restore_run_with_config(run_id, &config)
}

fn cmd_restore_run_with_config(run_id: i32, config: &PipelineConfig) -> Result<(), String> {
    let mut deleted_runs = read_deleted_runs_any(&config.deleted_runs_path)?;
    deleted_runs.remove(&run_id);
    write_deleted_runs(&config.deleted_runs_path, &deleted_runs)?;
    let analysis = analyze(&config)?;
    print_json(&json!({
        "restored_run_id": run_id,
        "analysis": analysis
    }))
}

fn cmd_monitor(once: bool) -> Result<(), String> {
    let config = load_config()?;
    #[cfg(not(windows))]
    {
        let _ = once;
        let _ = config;
        return Err("pipeline monitor is currently supported only on Windows".to_string());
    }
    #[cfg(windows)]
    {
        let mut state = load_state(&config.state_path)?;
        let mut last_pid = state.last_pid;
        let mut last_cursor_y = state.last_cursor_y;

        loop {
            let pid = match find_target_console_pid(&config.window_title_contains)? {
                Some(pid) => pid,
                None => {
                    state.status = Some("waiting_for_window".to_string());
                    state.updated_at = Some(now_iso());
                    state.error = None;
                    save_state(&config.state_path, &state)?;
                    if once {
                        return Err("pipeline monitor could not find target window".to_string());
                    }
                    thread::sleep(Duration::from_secs_f64(config.poll_seconds.max(0.1)));
                    continue;
                }
            };

            let start_row = if last_pid == Some(pid) {
                last_cursor_y.map(|row| row.saturating_add(1))
            } else {
                None
            };
            let (mut new_rows, cursor_y) = match read_console_lines(pid, start_row) {
                Ok(value) => value,
                Err(error) => {
                    state.status = Some("read_error".to_string());
                    state.updated_at = Some(now_iso());
                    state.last_pid = Some(pid);
                    state.error = Some(error.clone());
                    save_state(&config.state_path, &state)?;
                    if once {
                        return Err(error);
                    }
                    thread::sleep(Duration::from_secs_f64(config.poll_seconds.max(0.1)));
                    continue;
                }
            };
            let (visible_lines, _) = read_console_lines(pid, None)?;
            if last_pid != Some(pid) && start_row.is_none() {
                new_rows = visible_lines.clone();
            }

            let new_logical = merge_console_rows(&new_rows);
            let visible_logical = merge_console_rows(&visible_lines);
            let new_count = append_raw_records(&config.raw_log_path, pid, &new_logical)?;
            let analysis = analyze(&config)?;

            state.status = Some("running".to_string());
            state.updated_at = Some(now_iso());
            state.last_pid = Some(pid);
            state.last_cursor_y = Some(cursor_y);
            state.last_capture_count = Some(new_count);
            state.last_visible_lines = visible_logical.into_iter().rev().take(80).collect::<Vec<_>>();
            state.last_visible_lines.reverse();
            state.analysis = Some(analysis);
            state.error = None;
            save_state(&config.state_path, &state)?;

            last_pid = Some(pid);
            last_cursor_y = Some(cursor_y);
            if once {
                return Ok(());
            }
            thread::sleep(Duration::from_secs_f64(config.poll_seconds.max(0.1)));
        }
    }
}

fn analyze(config: &PipelineConfig) -> Result<AnalysisSummary, String> {
    let _guard = FileLock::acquire(&config.analysis_lock_path, Duration::from_secs(30))?;
    let raw_records = read_raw_records(&config.raw_log_path)?;
    let merged_records = merge_wrapped_records(&raw_records);
    let (mut samples, candidate_lines) = segment_runs(&merged_records, config)?;
    samples = collapse_initial_stationary_prefixes(samples);
    samples = filter_duplicate_replayed_runs(samples);
    let deleted_runs = read_deleted_runs_any(&config.deleted_runs_path)?;
    let summary_rows = summarize_runs(&samples, config, &deleted_runs);

    write_samples_csv(&config.parsed_csv_path, &samples)?;
    write_summary_csv(&config.summary_csv_path, &summary_rows)?;
    write_run_csvs(&config.runs_dir, &samples)?;
    fs::create_dir_all(&config.plots_dir).map_err(|error| format!("Failed to create plots dir: {error}"))?;
    write_latest_summary(&config.latest_summary_path, &summary_rows, &candidate_lines)?;
    merge_pipeline_runs(config, &samples, &summary_rows)?;

    let active_rows = summary_rows
        .iter()
        .filter(|row| !row.deleted)
        .cloned()
        .collect::<Vec<_>>();
    Ok(AnalysisSummary {
        raw_records: raw_records.len(),
        merged_records: merged_records.len(),
        parsed_samples: samples.len(),
        runs: summary_rows.len(),
        active_runs: active_rows.len(),
        latest_run: active_rows.last().cloned(),
        plotting_available: false,
        candidate_lines: candidate_lines.len(),
    })
}

fn load_config() -> Result<PipelineConfig, String> {
    let config_path = std::env::var_os("WATERWAY_PIPELINE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("pipeline_config.json")
        });
    let merged_viewer_data_dir = resolve_viewer_data_dir();
    load_config_from_path(&config_path, merged_viewer_data_dir)
}

fn load_config_from_path(
    config_path: &Path,
    merged_viewer_data_dir_override: Option<PathBuf>,
) -> Result<PipelineConfig, String> {
    let text = fs::read_to_string(&config_path)
        .map_err(|error| format!("Failed to read pipeline config {}: {error}", config_path.display()))?;
    let file: PipelineConfigFile =
        serde_json::from_str(strip_utf8_bom(&text)).map_err(|error| format!("Invalid pipeline config JSON: {error}"))?;
    let root = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_viewer_data_dir = resolve_relative(&root, &file.viewer_data_dir);
    let merged_viewer_data_dir =
        merged_viewer_data_dir_override.unwrap_or_else(|| config_viewer_data_dir.clone());
    Ok(PipelineConfig {
        window_title_contains: file.window_title_contains,
        poll_seconds: file.poll_seconds,
        run_gap_seconds: file.run_gap_seconds,
        x_reset_tolerance: file.x_reset_tolerance,
        raw_log_path: resolve_relative(&root, &file.raw_log_path),
        parsed_csv_path: resolve_relative(&root, &file.parsed_csv_path),
        summary_csv_path: resolve_relative(&root, &file.summary_csv_path),
        latest_summary_path: resolve_relative(&root, &file.latest_summary_path),
        plots_dir: resolve_relative(&root, &file.plots_dir),
        runs_dir: resolve_relative(&root, &file.runs_dir),
        merged_viewer_data_dir,
        deleted_runs_path: resolve_relative(&root, &file.deleted_runs_path),
        analysis_lock_path: resolve_relative(&root, &file.analysis_lock_path),
        state_path: resolve_relative(&root, &file.state_path),
        candidate_keywords: file.candidate_keywords,
        max_contiguous_dx: file.max_contiguous_dx,
        max_contiguous_log_gap_seconds: file.max_contiguous_log_gap_seconds,
    })
}

fn resolve_relative(root: &Path, raw: &str) -> PathBuf {
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    }
}

fn read_raw_records(path: &Path) -> Result<Vec<RawRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).map_err(|error| format!("Failed to open raw records {}: {error}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|error| format!("Failed to read raw log line: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<RawRecord>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

fn merge_wrapped_records(records: &[RawRecord]) -> Vec<RawRecord> {
    let mut merged: Vec<RawRecord> = Vec::new();
    for record in records {
        if let Some(last) = merged.last_mut()
            && !is_timestamped_console_line(&record.line)
            && line_needs_continuation(&last.line)
        {
            last.line.push_str(&record.line);
            last.captured_at = record.captured_at.clone();
            continue;
        }
        if !is_timestamped_console_line(&record.line) {
            continue;
        }
        merged.push(record.clone());
    }
    merged
}

fn merge_console_rows(lines: &[String]) -> Vec<String> {
    let mut merged = Vec::<String>::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if !is_timestamped_console_line(line) {
            if let Some(last) = merged.last_mut()
                && line_needs_continuation(last)
            {
                last.push_str(line);
            }
            continue;
        }
        merged.push(line.clone());
    }
    merged
}

fn segment_runs(records: &[RawRecord], config: &PipelineConfig) -> Result<(Vec<Sample>, Vec<String>), String> {
    let mut samples: Vec<Sample> = Vec::new();
    let mut candidate_lines = Vec::new();
    let mut run_id = 0_i32;
    let mut tick_index = 0_usize;
    let mut prev_x = None::<f64>;
    let mut prev_log_dt = None::<OffsetDateTime>;
    let mut run_start_x = None::<f64>;

    for (source_index, record) in records.iter().enumerate() {
        let Some(extracted) = extract_sample_values(&record.line) else {
            if is_candidate_sample_line(&record.line, &config.candidate_keywords) {
                candidate_lines.push(record.line.clone());
            }
            continue;
        };

        let fallback_date = parse_captured_at_or_now(&record.captured_at);
        let log_time = parse_log_time(&record.line, fallback_date, prev_log_dt)?;
        let log_dt = log_time
            .as_deref()
            .and_then(parse_iso_datetime)
            .or(prev_log_dt);

        let mut start_new_run = prev_x.is_none();
        if let Some(previous_x) = prev_x {
            if extracted.x < previous_x - config.x_reset_tolerance {
                start_new_run = true;
            } else if let (Some(previous), Some(current)) = (prev_log_dt, log_dt)
                && (current - previous).as_seconds_f64() > config.run_gap_seconds
            {
                // latest.log may flush delayed chat batches; preserve one run while the
                // physical x progression still looks contiguous, and only let wall-clock
                // gaps split runs after the motion itself becomes discontinuous.
                if (extracted.x - previous_x).abs() > config.max_contiguous_dx {
                    start_new_run = true;
                }
            }
        }

        if start_new_run {
            run_id += 1;
            tick_index = 0;
            run_start_x = Some(extracted.x);
        } else {
            tick_index += 1;
        }

        let start_x_raw = run_start_x.unwrap_or(extracted.x);
        let normalized_x = display_x_from_raw(extracted.x, start_x_raw);
        let derived_speed = samples
            .last()
            .filter(|sample| sample.run_id == run_id)
            .map(|sample| normalized_x - sample.x);

        samples.push(Sample {
            source_index,
            captured_at: record.captured_at.clone(),
            log_time,
            run_id,
            tick_index,
            x_raw: extracted.x,
            x: normalized_x,
            speed: extracted.speed,
            derived_speed,
            y: extracted.y,
            z: extracted.z,
            vx: extracted.vx.or(extracted.speed),
            vy: extracted.vy,
            vz: extracted.vz,
            raw_line: record.line.clone(),
        });
        prev_x = Some(extracted.x);
        prev_log_dt = log_dt;
    }
    Ok((samples, candidate_lines))
}

fn collapse_initial_stationary_prefixes(samples: Vec<Sample>) -> Vec<Sample> {
    let mut by_run = HashMap::<i32, Vec<Sample>>::new();
    let mut run_order = Vec::<i32>::new();
    for sample in samples {
        if !by_run.contains_key(&sample.run_id) {
            run_order.push(sample.run_id);
        }
        by_run.entry(sample.run_id).or_default().push(sample);
    }

    let mut collapsed = Vec::new();
    for run_id in run_order {
        let Some(mut run_samples) = by_run.remove(&run_id) else {
            continue;
        };
        if run_samples.is_empty() {
            continue;
        }
        let first = run_samples[0].clone();
        let mut prefix_len = 1usize;
        while prefix_len < run_samples.len()
            && same_position(&first, &run_samples[prefix_len])
            && stationary_like(&run_samples[prefix_len])
        {
            prefix_len += 1;
        }
        if prefix_len > 1 {
            let last_stationary = run_samples[prefix_len - 1].clone();
            run_samples.drain(0..prefix_len);
            run_samples.insert(0, last_stationary);
        }
        renumber_run_samples(&mut run_samples);
        collapsed.extend(run_samples);
    }
    collapsed
}

fn same_position(left: &Sample, right: &Sample) -> bool {
    nearly_equal(left.x_raw, right.x_raw)
        && option_nearly_equal(left.y, right.y)
        && option_nearly_equal(left.z, right.z)
}

fn stationary_like(sample: &Sample) -> bool {
    sample
        .speed
        .or(sample.vx)
        .is_none_or(|value| value.abs() <= 1.0e-12)
}

fn renumber_run_samples(samples: &mut [Sample]) {
    if samples.is_empty() {
        return;
    }
    let start_x_raw = samples[0].x_raw;
    let mut previous_x = None::<f64>;
    for (index, sample) in samples.iter_mut().enumerate() {
        sample.tick_index = index;
        sample.x = display_x_from_raw(sample.x_raw, start_x_raw);
        sample.derived_speed = previous_x.map(|value| sample.x - value);
        previous_x = Some(sample.x);
    }
}

fn filter_duplicate_replayed_runs(samples: Vec<Sample>) -> Vec<Sample> {
    let mut by_run = HashMap::<i32, Vec<Sample>>::new();
    let mut run_order = Vec::<i32>::new();
    for sample in samples {
        if !by_run.contains_key(&sample.run_id) {
            run_order.push(sample.run_id);
        }
        by_run.entry(sample.run_id).or_default().push(sample);
    }

    let mut kept = Vec::<DedupeRun>::new();
    for run_id in run_order {
        let Some(run_samples) = by_run.remove(&run_id) else {
            continue;
        };
        if run_samples.is_empty() {
            continue;
        }
        let collapsed_signature = collapse_signature(&run_samples);
        let end_log_time = run_samples.last().and_then(|sample| sample.log_time.clone());
        let end_x_raw = run_samples.last().map(|sample| sample.x_raw).unwrap_or(0.0);

        let mut drop_current = false;
        let mut remove_indices = Vec::<usize>::new();
        for index in (0..kept.len()).rev() {
            let previous = &kept[index];
            if previous.end_log_time != end_log_time || !nearly_equal(previous.end_x_raw, end_x_raw) {
                continue;
            }
            if collapsed_signature.len() >= previous.collapsed_signature.len()
                && collapsed_signature[collapsed_signature.len() - previous.collapsed_signature.len()..]
                    == previous.collapsed_signature[..]
            {
                remove_indices.push(index);
                continue;
            }
            if collapsed_signature.len() <= previous.collapsed_signature.len()
                && previous.collapsed_signature[previous.collapsed_signature.len() - collapsed_signature.len()..]
                    == collapsed_signature[..]
            {
                drop_current = true;
                break;
            }
        }
        if drop_current {
            continue;
        }
        for index in remove_indices.into_iter().rev() {
            kept.remove(index);
        }
        kept.push(DedupeRun {
            collapsed_signature,
            end_log_time,
            end_x_raw,
            samples: run_samples,
        });
    }

    let mut filtered = Vec::new();
    for deduped in kept {
        filtered.extend(deduped.samples);
    }
    filtered
}

fn collapse_signature(samples: &[Sample]) -> Vec<f64> {
    let mut collapsed = Vec::new();
    for value in samples.iter().map(|sample| round12(sample.x_raw)) {
        if collapsed.last().is_some_and(|last| nearly_equal(*last, value)) {
            continue;
        }
        collapsed.push(value);
    }
    collapsed
}

fn summarize_runs(samples: &[Sample], config: &PipelineConfig, deleted_runs: &HashSet<i32>) -> Vec<SummaryRow> {
    let mut by_run = HashMap::<i32, Vec<&Sample>>::new();
    for sample in samples {
        by_run.entry(sample.run_id).or_default().push(sample);
    }

    let log_path = infer_log_path(samples);
    let mut rows = Vec::new();
    let mut run_ids = by_run.keys().copied().collect::<Vec<_>>();
    run_ids.sort_unstable();
    for run_id in run_ids {
        let Some(run_samples) = by_run.get(&run_id) else {
            continue;
        };
        if run_samples.is_empty() {
            continue;
        }
        let speed_values = run_samples.iter().filter_map(|sample| sample.speed).collect::<Vec<_>>();
        let derived_values = run_samples
            .iter()
            .filter_map(|sample| sample.derived_speed)
            .collect::<Vec<_>>();
        let x_gt3_speed = run_samples
            .iter()
            .filter(|sample| sample.x > 3.0)
            .filter_map(|sample| sample.speed.or(sample.derived_speed))
            .collect::<Vec<_>>();
        let two_gt_hit_rate = two_gt_block_dwell_hit_rate(run_samples, config, None);
        let two_gt_hit_rate_x_gt_3 = two_gt_block_dwell_hit_rate(run_samples, config, Some(3.0));
        let dwell_groups = block_dwell_groups(run_samples, config);
        let dwell_blocks = dwell_groups.len();
        let dwell_hits = dwell_groups.iter().filter(|group| group.len() == DEFAULT_TARGET_DWELL_TICKS).count();
        let original_pos_motion_line_count = run_samples
            .iter()
            .filter(|sample| sample.raw_line.contains("Pos: [") && sample.raw_line.contains("Motion: ["))
            .count();
        let initial_stationary_line_count = count_initial_stationary_prefix(run_samples);
        let removed_initial_stationary_line_count = initial_stationary_line_count.saturating_sub(1);
        let kept_initial_stationary_line_count = usize::from(initial_stationary_line_count > 0);
        let first_moving_log_time = run_samples
            .iter()
            .find(|sample| sample.derived_speed.is_some_and(|speed| speed.abs() > 1.0e-12))
            .and_then(|sample| sample.log_time.clone())
            .or_else(|| run_samples.first().and_then(|sample| sample.log_time.clone()));
        let row = SummaryRow {
            run_id,
            sample_count: run_samples.len(),
            start_x_raw: run_samples.first().map(|sample| sample.x_raw).unwrap_or(0.0),
            end_x_raw: run_samples.last().map(|sample| sample.x_raw).unwrap_or(0.0),
            start_x: run_samples.first().map(|sample| sample.x).unwrap_or(0.0),
            end_x: run_samples.last().map(|sample| sample.x).unwrap_or(0.0),
            duration_gt: run_samples.len().saturating_sub(1),
            avg_logged_speed: mean(&speed_values),
            avg_derived_speed: mean(&derived_values),
            overall_avg_derived_speed: mean(&derived_values),
            avg_speed_x_gt_3: mean(&x_gt3_speed),
            two_gt_hit_rate,
            two_gt_hit_rate_x_gt_3,
            per_block_two_gt_dwell_hit_rate: two_gt_hit_rate,
            long_per_block_two_gt_dwell_hit_rate: two_gt_hit_rate,
            dwell_blocks,
            dwell_failures: dwell_blocks.saturating_sub(dwell_hits),
            target_speed: DEFAULT_TARGET_SPEED,
            target_dwell_ticks: DEFAULT_TARGET_DWELL_TICKS,
            deleted: deleted_runs.contains(&run_id),
            start_y: run_samples.first().and_then(|sample| sample.y),
            end_y: run_samples.last().and_then(|sample| sample.y),
            start_z: run_samples.first().and_then(|sample| sample.z),
            end_z: run_samples.last().and_then(|sample| sample.z),
            start_log_time: run_samples.first().and_then(|sample| sample.log_time.clone()),
            end_log_time: run_samples.last().and_then(|sample| sample.log_time.clone()),
            first_moving_log_time,
            original_pos_motion_line_count,
            initial_stationary_line_count,
            removed_initial_stationary_line_count,
            kept_initial_stationary_line_count,
            log_path: log_path.clone(),
            note: "Only Pos/Motion chat broadcast lines are included; the initial same-coordinate stationary prefix was collapsed to its last line.".to_string(),
        };
        if row.end_x <= 0.0 {
            continue;
        }
        rows.push(row);
    }
    rows
}

fn infer_log_path(samples: &[Sample]) -> String {
    if let Some(path) = std::env::var_os("WATERWAY_LATEST_LOG_PATH") {
        return PathBuf::from(path).display().to_string();
    }
    samples
        .iter()
        .find(|sample| sample.raw_line.contains("Pos: [") && sample.raw_line.contains("Motion: ["))
        .map(|_| "latest.log".to_string())
        .unwrap_or_else(String::new)
}

fn count_initial_stationary_prefix(samples: &[&Sample]) -> usize {
    let Some(first) = samples.first() else {
        return 0;
    };
    let mut count = 1usize;
    while count < samples.len() && same_position_ref(first, samples[count]) && stationary_like_ref(samples[count]) {
        count += 1;
    }
    count
}

fn same_position_ref(left: &&Sample, right: &Sample) -> bool {
    nearly_equal(left.x_raw, right.x_raw)
        && option_nearly_equal(left.y, right.y)
        && option_nearly_equal(left.z, right.z)
}

fn stationary_like_ref(sample: &Sample) -> bool {
    sample
        .speed
        .or(sample.vx)
        .is_none_or(|value| value.abs() <= 1.0e-12)
}

fn block_dwell_groups<'a>(samples: &[&'a Sample], config: &PipelineConfig) -> Vec<Vec<&'a Sample>> {
    let mut groups = Vec::<Vec<&Sample>>::new();
    let mut current = Vec::<&Sample>::new();
    for sample in samples {
        if current.is_empty() {
            current.push(*sample);
            continue;
        }
        let previous = *current.last().expect("current group has last sample");
        let previous_block = previous.x_raw.floor() as i64;
        let current_block = sample.x_raw.floor() as i64;
        if is_discontinuous(previous, sample, config) || current_block != previous_block {
            groups.push(current);
            current = vec![*sample];
        } else {
            current.push(*sample);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn is_discontinuous(previous: &Sample, next: &Sample, config: &PipelineConfig) -> bool {
    if (next.x - previous.x).abs() > config.max_contiguous_dx {
        return true;
    }
    match tick_continuity(previous, next) {
        Some(true) => return false,
        Some(false) => return true,
        None => {}
    }
    if let (Some(previous), Some(next)) = (
        previous.log_time.as_deref().and_then(parse_iso_datetime),
        next.log_time.as_deref().and_then(parse_iso_datetime),
    ) {
        return (next - previous).as_seconds_f64() > config.max_contiguous_log_gap_seconds;
    }
    false
}

fn tick_continuity(previous: &Sample, next: &Sample) -> Option<bool> {
    Some(next.tick_index == previous.tick_index.saturating_add(1))
}

fn two_gt_block_dwell_hit_rate(samples: &[&Sample], config: &PipelineConfig, min_x: Option<f64>) -> Option<f64> {
    let groups = block_dwell_groups(samples, config);
    let eligible = groups
        .into_iter()
        .filter(|group| min_x.is_none_or(|value| group.first().is_some_and(|sample| sample.x > value)))
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return None;
    }
    let hits = eligible.iter().filter(|group| group.len() == DEFAULT_TARGET_DWELL_TICKS).count();
    Some(hits as f64 / eligible.len() as f64)
}

fn merge_pipeline_runs(config: &PipelineConfig, samples: &[Sample], rows: &[SummaryRow]) -> Result<(), String> {
    fs::create_dir_all(&config.merged_viewer_data_dir)
        .map_err(|error| format!("Failed to create merged viewer data dir: {error}"))?;
    let store = RunStore::new(&config.merged_viewer_data_dir);
    let mut payload = store
        .load_runs()
        .map_err(|error| format!("Failed to load viewer runs: {error}"))?;
    let orphan_ids = payload
        .runs
        .iter()
        .filter(|run| run.summary.source.as_deref() == Some(PIPELINE_SOURCE))
        .filter_map(|run| run.run_id)
        .collect::<Vec<_>>();
    payload
        .runs
        .retain(|run| run.summary.source.as_deref() != Some(PIPELINE_SOURCE));
    let new_runs = build_viewer_runs(samples, rows);
    let new_run_ids = new_runs
        .iter()
        .filter_map(|run| run.run_id)
        .collect::<HashSet<_>>();
    payload.runs.extend(new_runs);
    payload.runs.sort_by_key(|run| run.run_id.unwrap_or_default());
    store
        .save_runs(&payload)
        .map_err(|error| format!("Failed to save merged viewer runs: {error}"))?;
    for run_id in orphan_ids {
        if new_run_ids.contains(&run_id) {
            continue;
        }
        let path = store.run_file_path(run_id);
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn build_viewer_runs(samples: &[Sample], rows: &[SummaryRow]) -> Vec<ViewerRun> {
    let mut by_run = HashMap::<i32, Vec<&Sample>>::new();
    for sample in samples {
        by_run.entry(sample.run_id).or_default().push(sample);
    }
    let mut runs = Vec::new();
    for row in rows {
        let Some(run_samples) = by_run.get(&row.run_id) else {
            continue;
        };
        let run_id = PIPELINE_RUN_ID_BASE + row.run_id as u64;
        let label = format!("latest.log 掉落物播报 {}", label_date(row.start_log_time.as_deref()));
        let points = run_samples.iter().map(|sample| sample_to_viewer_point(sample)).collect::<Vec<_>>();
        let mut summary = ViewerRunSummary {
            source: Some(PIPELINE_SOURCE.to_string()),
            model_engine: None,
            structure: Some(PIPELINE_STRUCTURE.to_string()),
            structure_count: Some(1),
            deleted: row.deleted,
            launch_mode: None,
            equivalent_fingerprint: None,
            extra: BTreeMap::new(),
        };
        insert_summary_row_metrics(&mut summary, row, run_id, &label);
        runs.push(ViewerRun {
            run_id: Some(run_id),
            label: Some(label.clone()),
            display_label: Some(label),
            summary,
            points,
            structure: None,
            extra: BTreeMap::new(),
        });
    }
    runs
}

fn label_date(value: Option<&str>) -> String {
    value
        .and_then(parse_iso_datetime)
        .map(|value| {
            format!(
                "{:04}-{:02}-{:02}",
                value.year(),
                u8::from(value.month()),
                value.day()
            )
        })
        .unwrap_or_else(|| "unknown-date".to_string())
}

fn sample_to_viewer_point(sample: &Sample) -> ViewerPoint {
    let mut extra = BTreeMap::<String, Value>::new();
    if let Some(z) = sample.z {
        extra.insert("z".to_string(), json!(z));
    }
    if let Some(vx) = sample.vx {
        extra.insert("vx".to_string(), json!(vx));
    }
    if let Some(vz) = sample.vz {
        extra.insert("vz".to_string(), json!(vz));
    }
    if sample.y.is_some() || sample.z.is_some() {
        extra.insert(
            "position".to_string(),
            json!({
                "x": sample.x_raw,
                "y": sample.y,
                "z": sample.z
            }),
        );
    }
    if sample.vx.is_some() || sample.vy.is_some() || sample.vz.is_some() {
        extra.insert(
            "motion".to_string(),
            json!({
                "x": sample.vx,
                "y": sample.vy,
                "z": sample.vz
            }),
        );
    }
    ViewerPoint {
        tick_index: sample.tick_index,
        x: Some(sample.x),
        x_raw: Some(sample.x_raw),
        speed: sample.speed,
        derived_speed: sample.derived_speed,
        y: sample.y,
        vy: sample.vy,
        floor: None,
        on_ground: None,
        log_time: sample.log_time.clone(),
        captured_at: (!sample.captured_at.is_empty()).then_some(sample.captured_at.clone()),
        raw_line: Some(sample.raw_line.clone()),
        extra,
    }
}

fn insert_summary_row_metrics(
    summary: &mut ViewerRunSummary,
    row: &SummaryRow,
    persistent_run_id: u64,
    label: &str,
) {
    summary.extra.insert("run_id".to_string(), json!(persistent_run_id));
    summary.extra.insert("label".to_string(), json!(label));
    summary.extra.insert("pipeline_run_id".to_string(), json!(row.run_id));
    summary.extra.insert("sample_count".to_string(), json!(row.sample_count));
    summary.extra.insert("start_x_raw".to_string(), json!(row.start_x_raw));
    summary.extra.insert("end_x_raw".to_string(), json!(row.end_x_raw));
    summary.extra.insert("start_x".to_string(), json!(row.start_x));
    summary.extra.insert("end_x".to_string(), json!(row.end_x));
    summary.extra.insert("duration_gt".to_string(), json!(row.duration_gt));
    summary.extra.insert("avg_logged_speed".to_string(), json!(row.avg_logged_speed));
    summary.extra.insert("avg_derived_speed".to_string(), json!(row.avg_derived_speed));
    summary.extra.insert(
        "overall_avg_derived_speed".to_string(),
        json!(row.overall_avg_derived_speed),
    );
    summary.extra.insert("avg_speed_x_gt_3".to_string(), json!(row.avg_speed_x_gt_3));
    summary.extra.insert("two_gt_hit_rate".to_string(), json!(row.two_gt_hit_rate));
    summary.extra.insert(
        "two_gt_hit_rate_x_gt_3".to_string(),
        json!(row.two_gt_hit_rate_x_gt_3),
    );
    summary.extra.insert(
        "per_block_two_gt_dwell_hit_rate".to_string(),
        json!(row.per_block_two_gt_dwell_hit_rate),
    );
    summary.extra.insert(
        "long_per_block_two_gt_dwell_hit_rate".to_string(),
        json!(row.long_per_block_two_gt_dwell_hit_rate),
    );
    summary.extra.insert("dwell_blocks".to_string(), json!(row.dwell_blocks));
    summary.extra.insert("dwell_failures".to_string(), json!(row.dwell_failures));
    summary.extra.insert("target_speed".to_string(), json!(row.target_speed));
    summary.extra.insert("target_dwell_ticks".to_string(), json!(row.target_dwell_ticks));
    summary.extra.insert("start_y".to_string(), json!(row.start_y));
    summary.extra.insert("end_y".to_string(), json!(row.end_y));
    summary.extra.insert("start_z".to_string(), json!(row.start_z));
    summary.extra.insert("end_z".to_string(), json!(row.end_z));
    summary.extra.insert("log_path".to_string(), json!(row.log_path));
    summary.extra.insert(
        "original_pos_motion_line_count".to_string(),
        json!(row.original_pos_motion_line_count),
    );
    summary.extra.insert(
        "initial_stationary_line_count".to_string(),
        json!(row.initial_stationary_line_count),
    );
    summary.extra.insert(
        "removed_initial_stationary_line_count".to_string(),
        json!(row.removed_initial_stationary_line_count),
    );
    summary.extra.insert(
        "kept_initial_stationary_line_count".to_string(),
        json!(row.kept_initial_stationary_line_count),
    );
    summary.extra.insert("start_log_time".to_string(), json!(row.start_log_time));
    summary.extra.insert("end_log_time".to_string(), json!(row.end_log_time));
    summary.extra.insert(
        "first_moving_log_time".to_string(),
        json!(row.first_moving_log_time),
    );
    summary.extra.insert("note".to_string(), json!(row.note));
}

fn write_samples_csv(path: &Path, samples: &[Sample]) -> Result<(), String> {
    ensure_parent(path)?;
    let tmp = tmp_path(path);
    let file = File::create(&tmp).map_err(|error| format!("Failed to create CSV {}: {error}", tmp.display()))?;
    let mut writer = WriterBuilder::new().from_writer(file);
    writer
        .write_record([
            "source_index",
            "captured_at",
            "log_time",
            "run_id",
            "tick_index",
            "x_raw",
            "x",
            "speed",
            "derived_speed",
            "raw_line",
        ])
        .map_err(|error| format!("Failed to write parsed samples CSV header: {error}"))?;
    for sample in samples {
        writer
            .write_record([
                sample.source_index.to_string(),
                sample.captured_at.clone(),
                sample.log_time.clone().unwrap_or_default(),
                sample.run_id.to_string(),
                sample.tick_index.to_string(),
                sample.x_raw.to_string(),
                sample.x.to_string(),
                option_f64_string(sample.speed),
                option_f64_string(sample.derived_speed),
                sample.raw_line.clone(),
            ])
            .map_err(|error| format!("Failed to write parsed samples CSV row: {error}"))?;
    }
    writer.flush().map_err(|error| format!("Failed to flush parsed samples CSV: {error}"))?;
    replace_file(&tmp, path)
}

fn write_summary_csv(path: &Path, rows: &[SummaryRow]) -> Result<(), String> {
    ensure_parent(path)?;
    let tmp = tmp_path(path);
    let file = File::create(&tmp).map_err(|error| format!("Failed to create CSV {}: {error}", tmp.display()))?;
    let mut writer = WriterBuilder::new().from_writer(file);
    writer
        .write_record([
            "run_id",
            "sample_count",
            "start_x_raw",
            "end_x_raw",
            "start_x",
            "end_x",
            "duration_gt",
            "avg_logged_speed",
            "avg_derived_speed",
            "avg_speed_x_gt_3",
            "two_gt_hit_rate",
            "two_gt_hit_rate_x_gt_3",
            "deleted",
        ])
        .map_err(|error| format!("Failed to write summary CSV header: {error}"))?;
    for row in rows {
        writer
            .write_record([
                row.run_id.to_string(),
                row.sample_count.to_string(),
                row.start_x_raw.to_string(),
                row.end_x_raw.to_string(),
                row.start_x.to_string(),
                row.end_x.to_string(),
                row.duration_gt.to_string(),
                option_f64_string(row.avg_logged_speed),
                option_f64_string(row.avg_derived_speed),
                option_f64_string(row.avg_speed_x_gt_3),
                option_f64_string(row.two_gt_hit_rate),
                option_f64_string(row.two_gt_hit_rate_x_gt_3),
                row.deleted.to_string(),
            ])
            .map_err(|error| format!("Failed to write summary CSV row: {error}"))?;
    }
    writer.flush().map_err(|error| format!("Failed to flush summary CSV: {error}"))?;
    replace_file(&tmp, path)
}

fn write_run_csvs(runs_dir: &Path, samples: &[Sample]) -> Result<(), String> {
    fs::create_dir_all(runs_dir).map_err(|error| format!("Failed to create runs dir {}: {error}", runs_dir.display()))?;
    let mut by_run = HashMap::<i32, Vec<&Sample>>::new();
    for sample in samples {
        by_run.entry(sample.run_id).or_default().push(sample);
    }
    let mut run_ids = by_run.keys().copied().collect::<Vec<_>>();
    run_ids.sort_unstable();
    for run_id in run_ids {
        let Some(run_samples) = by_run.get(&run_id) else {
            continue;
        };
        let path = runs_dir.join(format!("run_{run_id:04}.csv"));
        let tmp = tmp_path(&path);
        let file = File::create(&tmp).map_err(|error| format!("Failed to create run CSV {}: {error}", tmp.display()))?;
        let mut writer = WriterBuilder::new().from_writer(file);
        writer
            .write_record([
                "tick_index",
                "x_raw",
                "x",
                "speed",
                "derived_speed",
                "log_time",
                "raw_line",
            ])
            .map_err(|error| format!("Failed to write run CSV header: {error}"))?;
        for sample in run_samples {
            writer
                .write_record([
                    sample.tick_index.to_string(),
                    sample.x_raw.to_string(),
                    sample.x.to_string(),
                    option_f64_string(sample.speed),
                    option_f64_string(sample.derived_speed),
                    sample.log_time.clone().unwrap_or_default(),
                    sample.raw_line.clone(),
                ])
                .map_err(|error| format!("Failed to write run CSV: {error}"))?;
        }
        writer.flush().map_err(|error| format!("Failed to flush run CSV: {error}"))?;
        replace_file(&tmp, &path)?;
    }
    Ok(())
}

fn write_latest_summary(path: &Path, rows: &[SummaryRow], candidate_lines: &[String]) -> Result<(), String> {
    ensure_parent(path)?;
    let active_rows = rows.iter().filter(|row| !row.deleted).collect::<Vec<_>>();
    let mut lines = vec![
        "# Waterway Analysis".to_string(),
        String::new(),
        format!("Updated: {}", now_iso()),
        String::new(),
    ];
    if let Some(latest) = active_rows.last() {
        lines.push(format!("Latest run: `{}`", format!("{:04}", latest.run_id)));
        lines.push(String::new());
        lines.push(format!("- samples: `{}`", latest.sample_count));
        lines.push(format!("- x range: `{}` -> `{}`", latest.start_x, latest.end_x));
        lines.push(format!(
            "- avg speed for x>3: `{}`",
            format_metric(latest.avg_speed_x_gt_3)
        ));
        lines.push(format!(
            "- 2gt hit rate: `{}`",
            format_percent(latest.two_gt_hit_rate)
        ));
        lines.push(format!(
            "- 2gt hit rate for x>3: `{}`",
            format_percent(latest.two_gt_hit_rate_x_gt_3)
        ));
    } else {
        lines.push("No parsed runs yet.".to_string());
    }
    if !candidate_lines.is_empty() {
        lines.push(String::new());
        lines.push("Unparsed candidate lines:".to_string());
        for line in candidate_lines.iter().rev().take(10).rev() {
            lines.push(format!("- `{}`", line.replace('`', "'")));
        }
    }
    atomic_write_text(path, &format!("{}\n", lines.join("\n")))
}

fn read_deleted_runs_any(path: &Path) -> Result<HashSet<i32>, String> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read deleted runs {}: {error}", path.display()))?;
    let payload: Value =
        serde_json::from_str(strip_utf8_bom(&text)).map_err(|error| format!("Invalid deleted-runs JSON: {error}"))?;
    let items = match payload {
        Value::Array(values) => values,
        Value::Object(object) => object
            .get("deleted_runs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(items
        .into_iter()
        .filter_map(|value| value.as_i64())
        .filter_map(|value| i32::try_from(value).ok())
        .collect())
}

fn write_deleted_runs(path: &Path, deleted_runs: &HashSet<i32>) -> Result<(), String> {
    let mut values = deleted_runs.iter().copied().collect::<Vec<_>>();
    values.sort_unstable();
    atomic_write_json(path, &json!({ "deleted_runs": values }))
}

fn load_state(path: &Path) -> Result<PipelineState, String> {
    if !path.exists() {
        return Ok(PipelineState::default());
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read pipeline state {}: {error}", path.display()))?;
    serde_json::from_str(strip_utf8_bom(&text))
        .map_err(|error| format!("Invalid pipeline state JSON: {error}"))
}

fn save_state(path: &Path, state: &PipelineState) -> Result<(), String> {
    atomic_write_json(path, state)
}

fn append_raw_records(path: &Path, pid: u32, lines: &[String]) -> Result<usize, String> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("Failed to open raw capture log {}: {error}", path.display()))?;
    let mut count = 0usize;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let record = json!({
            "captured_at": now_iso(),
            "console_pid": pid,
            "line": line,
        });
        writeln!(
            file,
            "{}",
            serde_json::to_string(&record)
                .map_err(|error| format!("Failed to serialize raw capture record: {error}"))?
        )
        .map_err(|error| format!("Failed to append raw capture record: {error}"))?;
        count += 1;
    }
    Ok(count)
}

fn atomic_write_json(path: &Path, payload: &impl Serialize) -> Result<(), String> {
    ensure_parent(path)?;
    let tmp = tmp_path(path);
    let json = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("Failed to serialize JSON {}: {error}", path.display()))?;
    fs::write(&tmp, format!("{json}\n"))
        .map_err(|error| format!("Failed to write temp JSON {}: {error}", tmp.display()))?;
    replace_file(&tmp, path)
}

fn atomic_write_text(path: &Path, text: &str) -> Result<(), String> {
    ensure_parent(path)?;
    let tmp = tmp_path(path);
    fs::write(&tmp, text).map_err(|error| format!("Failed to write temp text {}: {error}", tmp.display()))?;
    replace_file(&tmp, path)
}

fn replace_file(tmp: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        match fs::remove_file(destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to remove existing file {}: {error}",
                    destination.display()
                ))
            }
        }
    }
    fs::rename(tmp, destination).map_err(|error| {
        format!(
            "Failed to move temp file {} into {}: {error}",
            tmp.display(),
            destination.display()
        )
    })
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create parent dir {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.tmp",
        path.file_name().and_then(|value| value.to_str()).unwrap_or("payload")
    ))
}

fn print_json(payload: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("Failed to serialize command output JSON: {error}"))?;
    println!("{text}");
    Ok(())
}

fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn now_iso() -> String {
    OffsetDateTime::now_local()
        .or_else(|_| Ok(OffsetDateTime::now_utc()))
        .and_then(|value| value.format(&Rfc3339))
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn parse_iso_datetime(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn parse_captured_at_or_now(value: &str) -> OffsetDateTime {
    value
        .trim()
        .split_whitespace()
        .next()
        .and_then(parse_iso_datetime)
        .unwrap_or_else(|| OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()))
}

fn parse_log_time(
    raw_line: &str,
    fallback_date: OffsetDateTime,
    previous_log_dt: Option<OffsetDateTime>,
) -> Result<Option<String>, String> {
    let captures = log_time_regex()
        .captures(raw_line)
        .ok_or_else(|| String::new())
        .ok();
    let Some(captures) = captures else {
        return Ok(None);
    };
    let hour = captures["h"]
        .parse::<u8>()
        .map_err(|error| format!("Invalid log-time hour in '{raw_line}': {error}"))?;
    let minute = captures["m"]
        .parse::<u8>()
        .map_err(|error| format!("Invalid log-time minute in '{raw_line}': {error}"))?;
    let second = captures["s"]
        .parse::<u8>()
        .map_err(|error| format!("Invalid log-time second in '{raw_line}': {error}"))?;
    let mut dt = build_offset_datetime(fallback_date.date(), fallback_date.offset(), hour, minute, second)?;
    if let Some(previous) = previous_log_dt {
        let half_day = 12 * 60 * 60;
        if dt.unix_timestamp() < previous.unix_timestamp() - half_day {
            dt += TimeDuration::hours(24);
        } else if dt.unix_timestamp() > previous.unix_timestamp() + half_day {
            dt -= TimeDuration::hours(24);
        }
    }
    Ok(Some(
        dt.format(&Rfc3339)
            .map_err(|error| format!("Failed to format log timestamp: {error}"))?,
    ))
}

fn build_offset_datetime(
    date: Date,
    offset: UtcOffset,
    hour: u8,
    minute: u8,
    second: u8,
) -> Result<OffsetDateTime, String> {
    let time = Time::from_hms(hour, minute, second)
        .map_err(|error| format!("Invalid log clock {hour:02}:{minute:02}:{second:02}: {error}"))?;
    Ok(date.with_time(time).assume_offset(offset))
}

fn format_metric(value: Option<f64>) -> String {
    value.map(|value| format!("{value:.6}")).unwrap_or_else(|| "n/a".to_string())
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.2}%", value * 100.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn option_f64_string(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn round12(value: f64) -> f64 {
    (value * 1.0e12).round() / 1.0e12
}

fn nearly_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12
}

fn option_nearly_equal(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => nearly_equal(left, right),
        _ => false,
    }
}

fn is_timestamped_console_line(line: &str) -> bool {
    line.starts_with('[')
        && line
            .as_bytes()
            .get(1..10)
            .is_some_and(|value| value[2] == b':' && value[5] == b':' && value[8] == b']')
}

fn line_needs_continuation(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let lower = line.to_ascii_lowercase();
    let likely = lower.contains(" pos: ")
        || lower.contains(" motion: ")
        || lower.contains("\"x\"")
        || lower.contains("\"speed\"")
        || lower.contains("\"velocity\"")
        || lower.contains("\"vel\"")
        || lower.contains("\"vx\"");
    likely && !line.trim_end().ends_with(']') && !line.trim_end().ends_with('}')
}

fn is_candidate_sample_line(raw_line: &str, candidate_keywords: &[String]) -> bool {
    let lower = raw_line.to_ascii_lowercase();
    if lower.contains('{') && lower.contains('}') {
        let has_x = ["\"x\"", "\"posx\"", "\"coordx\"", "\"坐标x\"", "\"坐标\""]
            .iter()
            .any(|pattern| lower.contains(pattern));
        let has_speed = ["\"speed\"", "\"velocity\"", "\"vel\"", "\"vx\"", "\"速度\""]
            .iter()
            .any(|pattern| lower.contains(pattern));
        if has_x && has_speed {
            return true;
        }
    }
    let default_keys = ["x", "speed", "velocity", "vel", "vx", "坐标", "速度"];
    let keys = if candidate_keywords.is_empty() {
        default_keys.iter().map(|value| value.to_string()).collect::<Vec<_>>()
    } else {
        candidate_keywords.to_vec()
    };
    keys.iter().any(|value| lower.contains(&value.to_ascii_lowercase()))
}

fn extract_sample_values(raw_line: &str) -> Option<ExtractedSample> {
    if let Some(captures) = pos_motion_regex().captures(raw_line) {
        let position = parse_doubles_csv(captures.get(1)?.as_str());
        let motion = parse_doubles_csv(captures.get(2)?.as_str());
        if position.len() >= 1 && motion.len() >= 1 {
            return Some(ExtractedSample {
                x: position[0],
                y: position.get(1).copied(),
                z: position.get(2).copied(),
                speed: motion.first().copied(),
                vx: motion.first().copied(),
                vy: motion.get(1).copied(),
                vz: motion.get(2).copied(),
            });
        }
    }

    if let Some(payload) = extract_json_object(raw_line) {
        let x = ["x", "posx", "coordx", "坐标x", "坐标"]
            .iter()
            .find_map(|key| payload.get(*key).and_then(number_from_value));
        let speed = ["speed", "velocity", "vel", "vx", "速度"]
            .iter()
            .find_map(|key| payload.get(*key).and_then(number_from_value));
        if let Some(x) = x {
            return Some(ExtractedSample {
                x,
                y: payload.get("y").and_then(number_from_value),
                z: payload.get("z").and_then(number_from_value),
                speed,
                vx: payload.get("vx").and_then(number_from_value).or(speed),
                vy: payload.get("vy").and_then(number_from_value),
                vz: payload.get("vz").and_then(number_from_value),
            });
        }
    }

    let x = labeled_x_regex()
        .captures(raw_line)
        .and_then(|captures| captures.get(1))
        .and_then(|value| parse_number(value.as_str()));
    let speed = labeled_speed_regex()
        .captures(raw_line)
        .and_then(|captures| captures.get(1))
        .and_then(|value| parse_number(value.as_str()));
    x.map(|x| ExtractedSample {
        x,
        y: None,
        z: None,
        speed,
        vx: speed,
        vy: None,
        vz: None,
    })
}

fn extract_json_object(raw_line: &str) -> Option<serde_json::Map<String, Value>> {
    let start = raw_line.find('{')?;
    let end = raw_line.rfind('}')?;
    serde_json::from_str::<Value>(&raw_line[start..=end])
        .ok()
        .and_then(|value| value.as_object().cloned())
}

fn number_from_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_number(text),
        _ => None,
    }
}

fn parse_doubles_csv(text: &str) -> Vec<f64> {
    let mut values = Vec::new();
    for part in text.split(',') {
        let Some(number) = parse_number(part) else {
            return Vec::new();
        };
        values.push(number);
    }
    values
}

fn parse_number(text: &str) -> Option<f64> {
    let trimmed = text.trim().trim_end_matches(['d', 'D', 'f', 'F']);
    trimmed.parse::<f64>().ok()
}

fn default_max_contiguous_dx() -> f64 {
    DEFAULT_MAX_CONTIGUOUS_DX
}

fn default_max_contiguous_log_gap_seconds() -> f64 {
    DEFAULT_MAX_CONTIGUOUS_LOG_GAP_SECONDS
}

fn pos_motion_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"Pos:\s*\[([^\]]+)\]\s*Motion:\s*\[([^\]]+)\]").expect("valid Pos/Motion regex"))
}

fn log_time_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^\[(?P<h>\d{2}):(?P<m>\d{2}):(?P<s>\d{2})\]").expect("valid log-time regex"))
}

fn labeled_x_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?:^|[\s,;|])(?:x|posx|coordx|坐标x|坐标)\s*[:=]\s*(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)")
            .expect("valid labeled x regex")
    })
}

fn labeled_speed_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?:^|[\s,;|])(?:speed|velocity|vel|vx|速度)\s*[:=]\s*(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)")
            .expect("valid labeled speed regex")
    })
}

struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(path: &Path, timeout: Duration) -> Result<Self, String> {
        ensure_parent(path)?;
        let started = SystemTime::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())
                        .map_err(|error| format!("Failed to write lock file {}: {error}", path.display()))?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(path)
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|elapsed| elapsed.as_secs() >= LOCK_STALE_AFTER_SECONDS);
                    if stale {
                        let _ = fs::remove_file(path);
                        continue;
                    }
                    if started.elapsed().unwrap_or_default() >= timeout {
                        return Err(format!("Timed out waiting for analysis lock {}", path.display()));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(format!("Failed to create analysis lock {}: {error}", path.display()))
                }
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn resolve_viewer_data_dir() -> Option<PathBuf> {
    std::env::var_os("MC_VIEWER_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("WATERWAY_DATA_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("viewer_data"))
        })
        .or_else(|| {
            std::env::var_os("WATERWAY_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("data").join("viewer_data"))
        })
}

#[cfg(windows)]
fn find_target_console_pid(window_title_contains: &str) -> Result<Option<u32>, String> {
    let mut windows = enum_windows()?;
    windows.retain(|(_, title)| title.to_ascii_lowercase().contains(&window_title_contains.to_ascii_lowercase()));
    Ok(windows.into_iter().map(|(pid, _)| pid).next())
}

#[cfg(windows)]
fn enum_windows() -> Result<Vec<(u32, String)>, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    unsafe extern "system" fn callback(
        hwnd: HWND,
        lparam: LPARAM,
    ) -> windows_sys::core::BOOL {
        let result = unsafe { &mut *(lparam as *mut Vec<(u32, String)>) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return 1;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        if written <= 0 {
            return 1;
        }
        let title = OsString::from_wide(&buffer[..written as usize])
            .to_string_lossy()
            .trim()
            .to_string();
        if title.is_empty() {
            return 1;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        result.push((pid, title));
        1
    }

    let mut result = Vec::<(u32, String)>::new();
    let success = unsafe { EnumWindows(Some(callback), &mut result as *mut _ as isize) };
    if success == 0 {
        return Err("EnumWindows failed".to_string());
    }
    Ok(result)
}

#[cfg(windows)]
fn read_console_lines(pid: u32, start_row: Option<i16>) -> Result<(Vec<String>, i16), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole,
    };

    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return Err(format!("AttachConsole failed for PID {pid}"));
        }
        let handle = CreateFileW(
            wide("CONOUT$").as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            FreeConsole();
            return Err("CreateFileW(CONOUT$) failed".to_string());
        }
        let result = read_console_lines_from_handle(handle, start_row);
        CloseHandle(handle);
        FreeConsole();
        result
    }
}

#[cfg(windows)]
fn read_console_lines_from_handle(
    handle: windows_sys::Win32::Foundation::HANDLE,
    start_row: Option<i16>,
) -> Result<(Vec<String>, i16), String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, COORD, GetConsoleScreenBufferInfo, ReadConsoleOutputCharacterW,
    };

    unsafe {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        if GetConsoleScreenBufferInfo(handle, &mut info) == 0 {
            return Err("GetConsoleScreenBufferInfo failed".to_string());
        }
        let width = info.dwSize.X.max(0) as usize;
        let cursor_y = info.dwCursorPosition.Y;
        let window_top = info.srWindow.Top;
        let read_start = start_row.unwrap_or(window_top).max(0);
        if read_start > cursor_y {
            return Ok((Vec::new(), cursor_y));
        }
        let mut lines = Vec::new();
        for row in read_start..=cursor_y {
            let mut buffer = vec![0u16; width];
            let mut chars_read = 0u32;
            if ReadConsoleOutputCharacterW(
                handle,
                buffer.as_mut_ptr(),
                width as u32,
                COORD { X: 0, Y: row },
                &mut chars_read,
            ) == 0
            {
                return Err(format!("ReadConsoleOutputCharacterW failed at row {row}"));
            }
            let text = OsString::from_wide(&buffer[..chars_read as usize])
                .to_string_lossy()
                .trim_end()
                .to_string();
            lines.push(text);
        }
        Ok((lines, cursor_y))
    }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ViewerRunsPayload;
    use std::time::UNIX_EPOCH;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "item-waterway-pipeline-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time after unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_config(root: &Path) -> PathBuf {
        let config_path = root.join("pipeline_config.json");
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&json!({
                "windowTitleContains": "Minecraft",
                "pollSeconds": 1.0,
                "runGapSeconds": 20.0,
                "xResetTolerance": 0.25,
                "twoGtTargetDx": 1.0,
                "twoGtTolerance": 0.05,
                "rawLogPath": "artifacts/raw/console_capture.jsonl",
                "parsedCsvPath": "artifacts/parsed_samples.csv",
                "summaryCsvPath": "artifacts/run_summaries.csv",
                "latestSummaryPath": "artifacts/latest_summary.md",
                "plotsDir": "artifacts/plots",
                "runsDir": "artifacts/runs",
                "viewerDataDir": "artifacts/viewer_data",
                "deletedRunsPath": "artifacts/deleted_runs.json",
                "analysisLockPath": "artifacts/analysis.lock",
                "statePath": "artifacts/pipeline_state.json",
                "candidateKeywords": ["x", "speed", "velocity", "vel", "vx"]
            }))
            .expect("serialize config"),
        )
        .expect("write config");
        config_path
    }

    fn write_raw_log(config_root: &Path, lines: &[&str]) {
        let path = config_root.join("artifacts/raw/console_capture.jsonl");
        fs::create_dir_all(path.parent().expect("raw log parent")).expect("create raw log parent");
        let content = lines
            .iter()
            .map(|line| {
                serde_json::to_string(&json!({
                    "captured_at": "2026-06-05T12:00:00+08:00",
                    "console_pid": 1,
                    "line": line
                }))
                .expect("serialize raw record")
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{content}\n")).expect("write raw log");
    }

    #[test]
    fn analyze_merges_pipeline_runs_without_clobbering_existing_runs() {
        let temp = TestDir::new();
        let config_path = write_config(&temp.path);
        let viewer_data = temp.path.join("shared-viewer-data");
        fs::create_dir_all(&viewer_data).expect("create shared viewer data");
        let store = RunStore::new(&viewer_data);
        store
            .save_runs(&ViewerRunsPayload {
                updated_at: None,
                latest_run_id: None,
                run_count: 0,
                runs: vec![ViewerRun {
                    run_id: Some(920001),
                    label: Some("search".to_string()),
                    display_label: Some("search".to_string()),
                    summary: ViewerRunSummary {
                        source: Some("reachability-search".to_string()),
                        model_engine: None,
                        structure: Some("demo".to_string()),
                        structure_count: Some(1),
                        deleted: false,
                        launch_mode: None,
                        equivalent_fingerprint: None,
                        extra: BTreeMap::new(),
                    },
                    points: Vec::new(),
                    structure: None,
                    extra: BTreeMap::new(),
                }],
            })
            .expect("save initial runs");

        write_raw_log(
            &temp.path,
            &[
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.635d,-62.0d,-42.875d] Motion: [1.0d,-0.04d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [1.635d,-62.0d,-42.875d] Motion: [0.5880000591278076d,0.0d,0.0d]",
                "[12:00:01] [Render thread/INFO]: [CHAT] Pos: [2.2230000591278074d,-62.0d,-42.875d] Motion: [0.5507152831981685d,0.0d,0.0d]",
            ],
        );

        let config =
            load_config_from_path(&config_path, Some(viewer_data.clone())).expect("load config");
        let summary = analyze(&config).expect("analyze pipeline");
        assert_eq!(summary.runs, 1);
        assert_eq!(summary.active_runs, 1);

        let merged = store.load_runs().expect("load merged runs");
        assert_eq!(merged.runs.len(), 2);
        assert!(merged
            .runs
            .iter()
            .any(|run| run.summary.source.as_deref() == Some("reachability-search")));
        let pipeline_run = merged
            .runs
            .iter()
            .find(|run| run.summary.source.as_deref() == Some(PIPELINE_SOURCE))
            .expect("pipeline run");
        assert_eq!(pipeline_run.run_id, Some(930001));
        assert_eq!(
            pipeline_run
                .summary
                .extra
                .get("pipeline_run_id")
                .and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(pipeline_run.points.len(), 4);
        assert_eq!(
            pipeline_run
                .summary
                .extra
                .get("sample_count")
                .and_then(Value::as_u64),
            Some(4)
        );
    }

    #[test]
    fn analyze_keeps_single_run_across_delayed_latest_log_flush() {
        let temp = TestDir::new();
        let config_path = write_config(&temp.path);
        let viewer_data = temp.path.join("shared-viewer-data");
        fs::create_dir_all(&viewer_data).expect("create shared viewer data");
        let store = RunStore::new(&viewer_data);

        write_raw_log(
            &temp.path,
            &[
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:25] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:26] [Render thread/INFO]: [CHAT] Pos: [0.635d,-62.0d,-42.875d] Motion: [1.0d,-0.04d,0.0d]",
                "[12:00:27] [Render thread/INFO]: [CHAT] Pos: [1.635d,-62.0d,-42.875d] Motion: [0.5880000591278076d,0.0d,0.0d]",
                "[12:00:52] [Render thread/INFO]: [CHAT] Pos: [2.2230000591278074d,-62.0d,-42.875d] Motion: [0.5507152831981685d,0.0d,0.0d]",
            ],
        );

        let config =
            load_config_from_path(&config_path, Some(viewer_data.clone())).expect("load config");
        let summary = analyze(&config).expect("analyze pipeline");
        assert_eq!(summary.runs, 1);
        assert_eq!(summary.active_runs, 1);

        let merged = store.load_runs().expect("load merged runs");
        let pipeline_run = merged
            .runs
            .iter()
            .find(|run| run.summary.source.as_deref() == Some(PIPELINE_SOURCE))
            .expect("pipeline run");
        assert_eq!(pipeline_run.run_id, Some(930001));
        assert_eq!(pipeline_run.points.len(), 4);
        assert_eq!(
            pipeline_run
                .summary
                .extra
                .get("sample_count")
                .and_then(Value::as_u64),
            Some(4)
        );
    }

    #[test]
    fn delete_and_restore_pipeline_run_round_trip_through_merged_viewer_store() {
        let temp = TestDir::new();
        let config_path = write_config(&temp.path);
        let data_root = temp.path.join("data");
        let viewer_data = data_root.join("viewer_data");
        fs::create_dir_all(&viewer_data).expect("create shared viewer data");
        let store = RunStore::new(&viewer_data);
        store
            .save_runs(&ViewerRunsPayload {
                updated_at: None,
                latest_run_id: None,
                run_count: 0,
                runs: vec![ViewerRun {
                    run_id: Some(920001),
                    label: Some("search".to_string()),
                    display_label: Some("search".to_string()),
                    summary: ViewerRunSummary {
                        source: Some("reachability-search".to_string()),
                        model_engine: None,
                        structure: Some("demo".to_string()),
                        structure_count: Some(1),
                        deleted: false,
                        launch_mode: None,
                        equivalent_fingerprint: None,
                        extra: BTreeMap::new(),
                    },
                    points: Vec::new(),
                    structure: None,
                    extra: BTreeMap::new(),
                }],
            })
            .expect("save initial runs");

        write_raw_log(
            &temp.path,
            &[
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.125d,-62.0d,-42.875d] Motion: [0.0d,0.0d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [0.635d,-62.0d,-42.875d] Motion: [1.0d,-0.04d,0.0d]",
                "[12:00:00] [Render thread/INFO]: [CHAT] Pos: [1.635d,-62.0d,-42.875d] Motion: [0.5880000591278076d,0.0d,0.0d]",
                "[12:00:01] [Render thread/INFO]: [CHAT] Pos: [2.2230000591278074d,-62.0d,-42.875d] Motion: [0.5507152831981685d,0.0d,0.0d]",
            ],
        );

        let config =
            load_config_from_path(&config_path, Some(viewer_data.clone())).expect("load config");
        assert_eq!(config.merged_viewer_data_dir, viewer_data);
        let configured_viewer_data_dir = temp.path.join("artifacts").join("viewer_data");
        assert_ne!(configured_viewer_data_dir, config.merged_viewer_data_dir);
        let summary = analyze(&config).expect("analyze pipeline");
        assert_eq!(summary.active_runs, 1);
        assert!(
            !configured_viewer_data_dir.exists(),
            "legacy configured viewer_data_dir should not be created when merged viewer data dir differs"
        );

        let before_delete = store.load_runs().expect("load runs before delete");
        let before_pipeline = before_delete
            .runs
            .iter()
            .find(|run| run.run_id == Some(930001))
            .expect("pipeline run before delete");
        assert!(!before_pipeline.summary.deleted);

        cmd_delete_run_with_config(1, &config).expect("delete pipeline run");
        let deleted_runs = read_deleted_runs_any(&config.deleted_runs_path).expect("read deleted runs");
        assert!(deleted_runs.contains(&1));
        let after_delete = store.load_runs().expect("load runs after delete");
        let deleted_pipeline = after_delete
            .runs
            .iter()
            .find(|run| run.run_id == Some(930001))
            .expect("pipeline run after delete");
        assert!(deleted_pipeline.summary.deleted);

        cmd_restore_run_with_config(1, &config).expect("restore pipeline run");
        let deleted_runs = read_deleted_runs_any(&config.deleted_runs_path).expect("read deleted runs after restore");
        assert!(!deleted_runs.contains(&1));
        let after_restore = store.load_runs().expect("load runs after restore");
        let restored_pipeline = after_restore
            .runs
            .iter()
            .find(|run| run.run_id == Some(930001))
            .expect("pipeline run after restore");
        assert!(!restored_pipeline.summary.deleted);
    }

    #[test]
    fn load_state_accepts_legacy_snake_case_analysis_summary() {
        let temp = TestDir::new();
        let state_path = temp.path.join("pipeline_state.json");
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&json!({
                "status": "running",
                "updated_at": "2026-05-31T02:42:48+08:00",
                "last_pid": 22996,
                "analysis": {
                    "raw_records": 19309,
                    "merged_records": 15365,
                    "parsed_samples": 12151,
                    "runs": 22,
                    "active_runs": 17,
                    "latest_run": {
                        "run_id": 86,
                        "sample_count": 6005,
                        "start_x_raw": 999.125,
                        "end_x_raw": 4002.6727184476185,
                        "start_x": 0.0,
                        "end_x": 3003.5477184476185,
                        "duration_gt": 6004,
                        "avg_logged_speed": 0.49519267722762056,
                        "avg_derived_speed": 0.5002577812204562,
                        "avg_speed_x_gt_3": 0.4950423693485527,
                        "two_gt_hit_rate": 0.7932700316508412,
                        "two_gt_hit_rate_x_gt_3": 0.7937989664944157,
                        "deleted": false
                    },
                    "plotting_available": true,
                    "candidate_lines": 0
                },
                "last_visible_lines": ["example line"]
            }))
            .expect("serialize legacy pipeline state"),
        )
        .expect("write legacy pipeline state");

        let state = load_state(&state_path).expect("load legacy pipeline state");
        assert_eq!(state.status.as_deref(), Some("running"));
        let analysis = state.analysis.expect("analysis");
        assert_eq!(analysis.raw_records, 19309);
        assert_eq!(analysis.active_runs, 17);
        let latest_run = analysis.latest_run.expect("latest run");
        assert_eq!(latest_run.run_id, 86);
        assert_eq!(latest_run.target_speed, DEFAULT_TARGET_SPEED);
        assert_eq!(latest_run.target_dwell_ticks, DEFAULT_TARGET_DWELL_TICKS);
        assert_eq!(latest_run.dwell_blocks, 0);
        assert_eq!(latest_run.log_path, "");
    }
}
