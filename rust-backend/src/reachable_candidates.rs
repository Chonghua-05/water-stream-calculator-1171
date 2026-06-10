use crate::reachability::{self, CandidateCatalog, PreparedSearch, SelectedCandidate};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const DEFAULT_TARGET_VXS: [f64; 11] = [0.1, 0.2, 0.3, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 0.9, 1.0];
pub(crate) const SEARCH_CANCELLED: &str = "search task cancelled";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CliArgs {
    out: PathBuf,
    run_name: String,
    modes: Vec<String>,
    cycles: Vec<String>,
    max_prefix_cells: usize,
    beam_width: usize,
    bucket_keep: usize,
    frontier_structure_keep: usize,
    bridge_beam_share: f64,
    proven_cycle_beam_weight: f64,
    workers: usize,
    worker_min_frontier_states: usize,
    short_ticks: usize,
    long_ticks: usize,
    short_pairs: usize,
    long_pairs: usize,
    cadence_tolerance: f64,
    target_phase_samples: usize,
    target_vxs: Vec<f64>,
    target_speed: f64,
    target_dwell_ticks: usize,
    steady_dwell_mode: String,
    entity_mods: Vec<usize>,
    initial_tick_mods: Vec<usize>,
    top_candidates: usize,
    long_limit: usize,
    min_short_hit_rate_for_long: f64,
    entry_tick_pad: usize,
    entry_ticks_per_cell: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    solver_path: Option<String>,
    requested_generator_engine: String,
    debug_generator: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_overrides: Option<StartOverrides>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    start_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_vy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_on_ground: Option<bool>,
}

#[derive(Clone, Debug)]
struct ModeStart {
    start_x: f64,
    start_y: f64,
    start_vx: f64,
    start_vy: f64,
    start_on_ground: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleInfo {
    name: String,
    role: String,
    length: usize,
    cost: f64,
    stage: usize,
}

#[derive(Clone)]
struct CandidateState {
    mode: String,
    candidate_id: String,
    early_score: f64,
    selection_score: f64,
    transient_shape_mae16: Option<f64>,
    transient_shape_mae24: Option<f64>,
    entity_id_mod4: usize,
    initial_tick_mod4: usize,
    cycle_name: String,
    cycle_proven: bool,
    cycle_cells: Vec<crate::Cell>,
    prefix_label: String,
    prefix_cells: Vec<crate::Cell>,
    complexity: f64,
    entry_reached: bool,
    entry_tick: Option<usize>,
    x: f64,
    x_phase: f64,
    vx: f64,
    y: f64,
    vy: f64,
    on_ground: bool,
    tick_mod4: usize,
    target: TargetDistance,
    trend: TrendSummary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetDistance {
    distance: f64,
    nearest_category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nearest_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nearest_phase: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase_distance: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrendSummary {
    avg_early_vx: f64,
    avg_recent_vx: f64,
    avg_recent_front_vx: f64,
    avg_recent_back_vx: f64,
    recent_velocity_slope: f64,
    recent_error_improvement: f64,
    macro_average_speed: f64,
    speed_error_early: f64,
    speed_error_recent: f64,
    speed_error_improvement: f64,
    avg_two_gt_distance: f64,
    two_gt_error: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_two_gt_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_two_gt_error: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailurePoint {
    tick: usize,
    x: f64,
    block: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    vx: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DwellMetrics {
    min_block: i64,
    max_block: i64,
    min_start_tick: usize,
    blocks: usize,
    exact2: usize,
    failures: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict_hit_rate: Option<f64>,
    target_dwell_ticks: usize,
    target_exact: usize,
    target_failures: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_hit_rate: Option<f64>,
    count_dist: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_failure: Option<Vec<FailurePoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_target_failure: Option<Vec<FailurePoint>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StableTick {
    score: f64,
    tick: usize,
    stable_like: bool,
    block_hit_rate: f64,
    average_speed: f64,
    mean_abs_distance_error: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DwellWindow {
    mode: String,
    min_block: i64,
    max_block: i64,
    include_final_group: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_start_tick: Option<usize>,
    min_start_tick: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_block_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_average_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_mean_abs_distance_error: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FullCadenceCompat {
    full_cadence_start_tick: usize,
    full_cadence_pairs: usize,
    full_cadence_mean_abs_distance_error: f64,
    full_cadence_mean_signed_distance_error: f64,
    full_cadence_max_abs_distance_error: f64,
    full_cadence_block_hit_rate: f64,
    full_cadence_within_tolerance_rate: f64,
    full_cadence_longest_hit_run: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_cadence_first_miss: Option<crate::EarlyCadenceSample>,
    full_cadence_min_hit_margin: f64,
    full_cadence_mean_hit_margin: f64,
    full_cadence_min_endpoint_boundary_margin: f64,
    full_cadence_mean_endpoint_boundary_margin: f64,
    full_cadence_samples: Vec<crate::FullCadenceSample>,
    full_cadence_distance: f64,
    full_cadence_average_speed: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationRow {
    id: String,
    mode: String,
    cycle: String,
    prefix_label: String,
    prefix_length: usize,
    complexity: f64,
    score: f64,
    early_score: f64,
    entity_id_mod4: usize,
    initial_tick_mod4: usize,
    ticks: usize,
    dwell: DwellMetrics,
    raw_dwell: DwellMetrics,
    dwell_window: DwellWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    cadence: Option<FullCadenceCompat>,
    average_speed: f64,
    average_speed_error: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    transient_shape_mae16: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transient_shape_mae24: Option<f64>,
    terminal_position_fit: Option<f64>,
    acceleration_ticks: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_stable_tick: Option<StableTick>,
    overshoot: bool,
    max_vx: f64,
    entry_tick: Option<usize>,
    target: TargetDistance,
    first_ticks: Vec<Value>,
    prefix_cells: Vec<crate::CellDescription>,
    cycle_cells: Vec<crate::CellDescription>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetSample {
    cycle: String,
    category: String,
    start_x: f64,
    x_phase: f64,
    vx: f64,
    y: f64,
    vy: f64,
    on_ground: bool,
    entity_id_mod4: usize,
    initial_tick_mod4: usize,
    strict_hit_rate: f64,
    target_hit_rate: f64,
    block_hit_rate: f64,
    avg_speed: f64,
    trend_improvement: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RangeSummary {
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_phase_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_phase_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vx_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vx_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_speed_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_speed_max: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tick_mods: Vec<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    entity_mods: Vec<usize>,
    examples: Vec<TargetSample>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetWindowSummary {
    cycle: String,
    period: usize,
    evaluated: usize,
    stable_window: RangeSummary,
    near_stable_window: RangeSummary,
    accelerating_trend_window: RangeSummary,
    scoring_samples: Vec<TargetSample>,
}

#[derive(Clone)]
struct TargetMetric {
    full: Option<crate::FullCadence>,
    dwell: DwellMetrics,
    average_speed: f64,
    avg_end: f64,
    trend_improvement: f64,
    overrun: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceSearchRequest {
    pub(crate) out_root: PathBuf,
    pub(crate) run_name: String,
    pub(crate) modes: Vec<String>,
    pub(crate) cycles: Vec<String>,
    pub(crate) max_prefix_cells: usize,
    pub(crate) workers: usize,
    pub(crate) short_ticks: usize,
    pub(crate) target_speed: f64,
    pub(crate) target_dwell_ticks: usize,
    pub(crate) top_candidates: usize,
    pub(crate) long_limit: usize,
    pub(crate) min_short_hit_rate_for_long: f64,
    pub(crate) entity_mods: Vec<usize>,
    pub(crate) initial_tick_mods: Vec<usize>,
    pub(crate) start_x: f64,
    pub(crate) start_y: f64,
    pub(crate) start_vx: f64,
    pub(crate) start_vy: f64,
    pub(crate) start_on_ground: bool,
    pub(crate) debug_generator: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceCandidateRow {
    pub(crate) id: String,
    pub(crate) mode: String,
    pub(crate) cycle: String,
    pub(crate) prefix_label: String,
    pub(crate) score: f64,
    pub(crate) entity_id_mod4: usize,
    pub(crate) initial_tick_mod4: usize,
    pub(crate) strict_hit_rate: Option<f64>,
    pub(crate) target_hit_rate: Option<f64>,
    pub(crate) raw_short_hit_rate: Option<f64>,
    pub(crate) average_speed: f64,
    pub(crate) dwell_window: Value,
    pub(crate) prefix_cells: Vec<crate::CellDescription>,
    pub(crate) cycle_cells: Vec<crate::CellDescription>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceSearchResult {
    pub(crate) out_dir: PathBuf,
    pub(crate) generator_payload: Value,
    pub(crate) short_verified: Vec<ServiceCandidateRow>,
    pub(crate) short_passing_for_long: usize,
    pub(crate) long_verified: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceProgressUpdate {
    pub(crate) stage: String,
    pub(crate) message: String,
    pub(crate) checked: Option<u64>,
    pub(crate) total: Option<u64>,
    pub(crate) percent: Option<f64>,
    pub(crate) candidate_count: Option<u64>,
    pub(crate) unique_count: Option<u64>,
    pub(crate) expanded_states: Option<u64>,
    pub(crate) bucket_count: Option<u64>,
}

#[derive(Clone)]
struct RunArtifacts {
    out_dir: PathBuf,
    stdout_payload: Value,
    short_verified: Vec<VerificationRow>,
    short_passing_for_long: usize,
    long_verified: Vec<VerificationRow>,
}

pub(super) fn usage() -> String {
    "Usage: item-waterway-solver reachable-candidates [--out <dir>] [--run-name <name>] [--modes launch-fast,water-accelerate] [--cycles W3-I_D3-B,W2-I_D2-B] [--max-prefix-cells 8] [--short-ticks 400] [--long-ticks 20000] [--top-candidates 80]".to_string()
}

pub(super) fn main_cli(argv: &[String]) -> Result<(), String> {
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return Ok(());
    }

    let args = parse_args(argv)?;
    let payload = run(&args)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("Failed to encode reachable-candidates stdout JSON: {error}"))?
    );
    Ok(())
}

fn parse_args(argv: &[String]) -> Result<CliArgs, String> {
    let mut args = default_args();
    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        let next = |cursor: &mut usize, flag: &str| -> Result<&str, String> {
            *cursor += 1;
            argv.get(*cursor)
                .map(|value| value.as_str())
                .ok_or_else(|| format!("Missing value for {flag}"))
        };
        match arg.as_str() {
            "--out" => args.out = PathBuf::from(next(&mut index, "--out")?),
            "--run-name" => args.run_name = next(&mut index, "--run-name")?.to_string(),
            "--modes" => args.modes = crate::parse_string_list(next(&mut index, "--modes")?),
            "--cycles" => args.cycles = crate::parse_string_list(next(&mut index, "--cycles")?),
            "--max-prefix-cells" => {
                args.max_prefix_cells = crate::parse_usize(next(&mut index, "--max-prefix-cells")?, "--max-prefix-cells")?
            }
            "--beam-width" => {
                args.beam_width = crate::parse_usize(next(&mut index, "--beam-width")?, "--beam-width")?
            }
            "--bucket-keep" => {
                args.bucket_keep = crate::parse_usize(next(&mut index, "--bucket-keep")?, "--bucket-keep")?
            }
            "--frontier-structure-keep" => {
                args.frontier_structure_keep =
                    crate::parse_usize(next(&mut index, "--frontier-structure-keep")?, "--frontier-structure-keep")?
            }
            "--bridge-beam-share" => {
                args.bridge_beam_share = crate::parse_f64(next(&mut index, "--bridge-beam-share")?, "--bridge-beam-share")?
            }
            "--proven-cycle-beam-weight" => {
                args.proven_cycle_beam_weight =
                    crate::parse_f64(next(&mut index, "--proven-cycle-beam-weight")?, "--proven-cycle-beam-weight")?
            }
            "--workers" => args.workers = crate::parse_usize(next(&mut index, "--workers")?, "--workers")?,
            "--worker-min-frontier-states" => {
                args.worker_min_frontier_states =
                    crate::parse_usize(next(&mut index, "--worker-min-frontier-states")?, "--worker-min-frontier-states")?
            }
            "--short-ticks" => args.short_ticks = crate::parse_usize(next(&mut index, "--short-ticks")?, "--short-ticks")?,
            "--long-ticks" => args.long_ticks = crate::parse_usize(next(&mut index, "--long-ticks")?, "--long-ticks")?,
            "--short-pairs" => args.short_pairs = crate::parse_usize(next(&mut index, "--short-pairs")?, "--short-pairs")?,
            "--long-pairs" => args.long_pairs = crate::parse_usize(next(&mut index, "--long-pairs")?, "--long-pairs")?,
            "--cadence-tolerance" => {
                args.cadence_tolerance = crate::parse_f64(next(&mut index, "--cadence-tolerance")?, "--cadence-tolerance")?
            }
            "--target-phase-samples" => {
                args.target_phase_samples =
                    crate::parse_usize(next(&mut index, "--target-phase-samples")?, "--target-phase-samples")?
            }
            "--target-vxs" => {
                args.target_vxs = crate::parse_number_list(next(&mut index, "--target-vxs")?);
            }
            "--target-speed" => {
                args.target_speed = crate::parse_f64(next(&mut index, "--target-speed")?, "--target-speed")?
            }
            "--target-dwell-ticks" => {
                args.target_dwell_ticks =
                    crate::parse_usize(next(&mut index, "--target-dwell-ticks")?, "--target-dwell-ticks")?
            }
            "--steady-dwell-mode" => args.steady_dwell_mode = next(&mut index, "--steady-dwell-mode")?.to_string(),
            "--entity-mods" | "--entity-id-mods" => {
                args.entity_mods = crate::parse_usize_list(next(&mut index, arg)?, arg)?
            }
            "--initial-tick-mods" | "--initial-tick-counts" => {
                args.initial_tick_mods = crate::parse_usize_list(next(&mut index, arg)?, arg)?
            }
            "--top-candidates" => {
                args.top_candidates = crate::parse_usize(next(&mut index, "--top-candidates")?, "--top-candidates")?
            }
            "--long-limit" => args.long_limit = crate::parse_usize(next(&mut index, "--long-limit")?, "--long-limit")?,
            "--min-short-hit-rate-for-long" => {
                args.min_short_hit_rate_for_long =
                    crate::parse_f64(next(&mut index, "--min-short-hit-rate-for-long")?, "--min-short-hit-rate-for-long")?
            }
            "--entry-tick-pad" => {
                args.entry_tick_pad = crate::parse_usize(next(&mut index, "--entry-tick-pad")?, "--entry-tick-pad")?
            }
            "--entry-ticks-per-cell" => {
                args.entry_ticks_per_cell =
                    crate::parse_usize(next(&mut index, "--entry-ticks-per-cell")?, "--entry-ticks-per-cell")?
            }
            "--solver" => args.solver_path = Some(next(&mut index, "--solver")?.to_string()),
            "--generator-engine" => {
                args.requested_generator_engine = next(&mut index, "--generator-engine")?.to_string()
            }
            "--debug-generator" => args.debug_generator = true,
            "--start-x" => {
                args.start_overrides.get_or_insert_with(Default::default).start_x =
                    Some(crate::parse_f64(next(&mut index, "--start-x")?, "--start-x")?)
            }
            "--start-y" => {
                args.start_overrides.get_or_insert_with(Default::default).start_y =
                    Some(crate::parse_f64(next(&mut index, "--start-y")?, "--start-y")?)
            }
            "--start-vx" => {
                args.start_overrides.get_or_insert_with(Default::default).start_vx =
                    Some(crate::parse_f64(next(&mut index, "--start-vx")?, "--start-vx")?)
            }
            "--start-vy" => {
                args.start_overrides.get_or_insert_with(Default::default).start_vy =
                    Some(crate::parse_f64(next(&mut index, "--start-vy")?, "--start-vy")?)
            }
            "--start-on-ground" => {
                let value = next(&mut index, "--start-on-ground")?;
                let parsed = match value {
                    "true" | "TRUE" | "True" => true,
                    "false" | "FALSE" | "False" => false,
                    _ => return Err("--start-on-ground must be true or false.".to_string()),
                };
                args.start_overrides.get_or_insert_with(Default::default).start_on_ground = Some(parsed);
            }
            "--rust-batch-size"
            | "--legal-period"
            | "--legal-limit"
            | "--legal-max-sources"
            | "--legal-search-cycles"
            | "--phase-bucket-denom"
            | "--vx-bucket-size"
            | "--y-bucket-size"
            | "--vy-bucket-size"
            | "--max-enter-gt"
            | "--max-complexity"
            | "--examples-per-prune" => {
                let _ = next(&mut index, arg)?;
            }
            "--hard-max-enter-gt" | "--use-known-rust-legal-seeds" => {}
            other => return Err(format!("Unknown argument: {other}")),
        }
        index += 1;
    }

    validate_args(&mut args)?;
    Ok(args)
}

fn service_short_pairs(short_ticks: usize) -> usize {
    short_ticks
        .saturating_sub(9)
        .saturating_div(2)
        .max(4)
}

fn default_args() -> CliArgs {
    CliArgs {
        out: PathBuf::from("artifacts").join("reachability-candidate-generator"),
        run_name: format!("smoke-{}", unix_millis()),
        modes: vec!["launch-fast".to_string(), "water-accelerate".to_string()],
        cycles: vec!["W3-I_D3-B".to_string(), "W2-I_D2-B".to_string()],
        max_prefix_cells: 8,
        beam_width: 2000,
        bucket_keep: 2,
        frontier_structure_keep: 16,
        bridge_beam_share: 0.25,
        proven_cycle_beam_weight: 3.0,
        workers: 8,
        worker_min_frontier_states: 128,
        short_ticks: 400,
        long_ticks: 20_000,
        short_pairs: 190,
        long_pairs: 10_000,
        cadence_tolerance: 0.05,
        target_phase_samples: 12,
        target_vxs: DEFAULT_TARGET_VXS.to_vec(),
        target_speed: 0.5,
        target_dwell_ticks: 2,
        steady_dwell_mode: "auto".to_string(),
        entity_mods: vec![0, 1, 2, 3],
        initial_tick_mods: vec![0, 1, 2, 3],
        top_candidates: 80,
        long_limit: 5,
        min_short_hit_rate_for_long: 0.98,
        entry_tick_pad: 80,
        entry_ticks_per_cell: 30,
        solver_path: None,
        requested_generator_engine: "rust".to_string(),
        debug_generator: false,
        start_overrides: None,
    }
}

fn validate_args(args: &mut CliArgs) -> Result<(), String> {
    if args.modes.is_empty() {
        return Err("--modes must name at least one mode.".to_string());
    }
    for mode in &args.modes {
        mode_start(mode, args.start_overrides.as_ref())?;
    }
    if args.cycles.is_empty() {
        return Err("--cycles must name at least one cycle.".to_string());
    }
    let known_cycles = crate::backbone_cycles()
        .into_iter()
        .map(|cycle| cycle.name)
        .collect::<BTreeSet<_>>();
    for cycle in &args.cycles {
        if !known_cycles.contains(cycle) {
            return Err(format!(
                "Unknown cycle '{cycle}'. Known cycles: {}",
                known_cycles.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if args.max_prefix_cells > 64 {
        return Err("--max-prefix-cells must be in [0, 64].".to_string());
    }
    args.beam_width = args.beam_width.max(1);
    args.bucket_keep = args.bucket_keep.max(1);
    args.frontier_structure_keep = args.frontier_structure_keep.clamp(1, 64);
    if !(0.0..=1.0).contains(&args.bridge_beam_share) {
        return Err("--bridge-beam-share must be in [0, 1].".to_string());
    }
    args.proven_cycle_beam_weight = args.proven_cycle_beam_weight.max(1.0);
    args.workers = args.workers.clamp(1, 64);
    args.worker_min_frontier_states = args.worker_min_frontier_states.max(1);
    args.top_candidates = args.top_candidates.max(1);
    args.short_pairs = args.short_pairs.min(args.short_ticks.saturating_div(2).saturating_sub(1).max(1));
    args.long_pairs = args.long_pairs.min(args.long_ticks.saturating_div(2).saturating_sub(1).max(1));
    if !(args.target_speed > 0.0 && args.target_speed <= 1.25) {
        return Err("--target-speed must be in (0, 1.25].".to_string());
    }
    if !matches!(args.steady_dwell_mode.as_str(), "auto" | "cycle" | "stable") {
        return Err("--steady-dwell-mode must be auto, cycle, or stable.".to_string());
    }
    if args.target_vxs.is_empty() {
        return Err("--target-vxs must contain at least one finite number.".to_string());
    }
    args.target_dwell_ticks = args.target_dwell_ticks.max(1);
    if args.entity_mods.is_empty() || args.initial_tick_mods.is_empty() {
        return Err("--entity-mods and --initial-tick-mods must be non-empty.".to_string());
    }
    if args
        .entity_mods
        .iter()
        .chain(args.initial_tick_mods.iter())
        .any(|value| *value >= 4)
    {
        return Err("entity/tick mod values must be in [0, 3].".to_string());
    }
    Ok(())
}

pub(crate) fn is_cancelled_error(error: &str) -> bool {
    error == SEARCH_CANCELLED
}

pub(crate) fn run_service(
    request: &ServiceSearchRequest,
    cancel: Option<&AtomicBool>,
    progress: Option<&mut dyn FnMut(ServiceProgressUpdate)>,
) -> Result<ServiceSearchResult, String> {
    let mut args = default_args();
    args.out = request.out_root.clone();
    args.run_name = request.run_name.clone();
    args.modes = request.modes.clone();
    args.cycles = request.cycles.clone();
    args.max_prefix_cells = request.max_prefix_cells;
    args.workers = request.workers;
    args.short_ticks = request.short_ticks;
    args.long_ticks = request.short_ticks;
    args.short_pairs = service_short_pairs(request.short_ticks);
    args.long_pairs = args.short_pairs;
    args.target_speed = request.target_speed;
    args.target_dwell_ticks = request.target_dwell_ticks;
    args.entity_mods = request.entity_mods.clone();
    args.initial_tick_mods = request.initial_tick_mods.clone();
    args.top_candidates = request.top_candidates;
    args.long_limit = request.long_limit;
    args.min_short_hit_rate_for_long = request.min_short_hit_rate_for_long;
    args.debug_generator = request.debug_generator;
    args.start_overrides = Some(StartOverrides {
        start_x: Some(request.start_x),
        start_y: Some(request.start_y),
        start_vx: Some(request.start_vx),
        start_vy: Some(request.start_vy),
        start_on_ground: Some(request.start_on_ground),
    });
    validate_args(&mut args)?;
    let artifacts = execute(&args, cancel, progress)?;
    Ok(ServiceSearchResult {
        out_dir: artifacts.out_dir,
        generator_payload: artifacts.stdout_payload,
        short_passing_for_long: artifacts.short_passing_for_long,
        long_verified: artifacts.long_verified.len(),
        short_verified: artifacts
            .short_verified
            .into_iter()
            .map(|row| ServiceCandidateRow {
                id: row.id,
                mode: row.mode,
                cycle: row.cycle,
                prefix_label: row.prefix_label,
                score: row.score,
                entity_id_mod4: row.entity_id_mod4,
                initial_tick_mod4: row.initial_tick_mod4,
                strict_hit_rate: row.dwell.strict_hit_rate,
                target_hit_rate: row.dwell.target_hit_rate,
                raw_short_hit_rate: row.raw_dwell.target_hit_rate.or(row.raw_dwell.strict_hit_rate),
                average_speed: row.average_speed,
                dwell_window: serde_json::to_value(&row.dwell_window)
                    .expect("dwell window should serialize"),
                prefix_cells: row.prefix_cells,
                cycle_cells: row.cycle_cells,
            })
            .collect(),
    })
}

fn run(args: &CliArgs) -> Result<Value, String> {
    execute(args, None, None).map(|artifacts| artifacts.stdout_payload)
}

fn execute(
    args: &CliArgs,
    cancel: Option<&AtomicBool>,
    mut progress: Option<&mut dyn FnMut(ServiceProgressUpdate)>,
) -> Result<RunArtifacts, String> {
    let out_dir = args.out.join(&args.run_name);
    fs::create_dir_all(&out_dir)
        .map_err(|error| format!("Failed to create output dir {}: {error}", out_dir.display()))?;

    let generator_engine = json!({
        "generatedAt": timestamp_now_string()?,
        "generatorEngine": "rust",
        "requestedGeneratorEngine": args.requested_generator_engine,
        "selectedPath": "rust reachability search",
        "solverPath": args.solver_path,
        "script": "item-waterway-solver reachable-candidates",
    });
    write_json(&out_dir.join("generator-engine.json"), &generator_engine)?;

    if args.debug_generator {
        println!(
            "{}",
            json!({
                "event": "generator-debug",
                "generatorEngine": "rust",
                "requestedGeneratorEngine": args.requested_generator_engine,
                "selectedPath": "rust reachability search",
                "solverPath": args.solver_path,
                "runDir": out_dir.display().to_string(),
            })
        );
    }

    check_cancelled(cancel)?;
    let cycles = requested_cycles(&args.cycles)?;
    report_progress(
        &mut progress,
        ServiceProgressUpdate {
            stage: "searching".to_string(),
            message: "scanning target windows".to_string(),
            checked: Some(0),
            total: Some(cycles.len() as u64),
            percent: Some(5.0),
            candidate_count: Some(0),
            unique_count: Some(0),
            expanded_states: Some(0),
            bucket_count: Some(0),
        },
    );
    let target_windows = scan_target_windows(&cycles, args, cancel, &mut progress);
    check_cancelled(cancel)?;
    write_json(&out_dir.join("target-windows.json"), &target_windows)?;

    let mode_limits = per_mode_limits(args.top_candidates, args.modes.len());
    let mut total_expanded = 0_usize;
    let mut frontier_stats = Vec::new();
    let mut candidate_prefix_rows = Vec::new();
    let mut prune_stats = BTreeMap::new();
    let mut kept_total = 0_usize;
    let mut deduped_total = 0_usize;
    let mut short_verified = Vec::new();
    let mut short_passing_for_long = 0_usize;
    let mut long_candidate_states = Vec::new();
    let total_layers = (args.modes.len() * args.max_prefix_cells.max(1)).max(1) as f64;

    for (index, mode) in args.modes.iter().enumerate() {
        check_cancelled(cancel)?;
        let candidate_count = mode_limits[index];
        let mut layer_snapshots = Vec::new();
        let mut mode_progress = |state: crate::reachability::PrepareProgress| {
            layer_snapshots.push(state.clone());
            let layer_base = index * args.max_prefix_cells.max(1) + state.layer.saturating_sub(1);
            let layer_fraction = if state.expanded_prefixes > 0 {
                state.processed_prefixes as f64 / state.expanded_prefixes as f64
            } else {
                0.0
            };
            let completed_layers = layer_base as f64 + layer_fraction.clamp(0.0, 1.0);
            let search_percent = 15.0 + ((completed_layers / total_layers) * 70.0);
            let message = if state.expanded_prefixes > 0 {
                format!(
                    "searching {mode} layer {}/{} ({}/{})",
                    state.layer,
                    args.max_prefix_cells,
                    state.processed_prefixes,
                    state.expanded_prefixes
                )
            } else {
                format!("searching {mode} layer {}/{}", state.layer, args.max_prefix_cells)
            };
            let frontier_count = (state.processed_prefixes == state.expanded_prefixes)
                .then_some(state.frontier_out as u64);
            report_progress(
                &mut progress,
                ServiceProgressUpdate {
                    stage: "searching".to_string(),
                    message,
                    checked: Some(state.processed_prefixes as u64),
                    total: (state.expanded_prefixes > 0).then_some(state.expanded_prefixes as u64),
                    percent: Some(search_percent.clamp(15.0, 85.0)),
                    candidate_count: Some(state.kept_total as u64),
                    unique_count: frontier_count,
                    expanded_states: Some(state.evaluated_total as u64),
                    bucket_count: frontier_count,
                },
            );
        };
        let prepared_mode = prepare_mode(mode, args, cancel, Some(&mut mode_progress))?;
        let bucket_count = unique_prefix_count(&prepared_mode.ranked_early_candidates, &prepared_mode.catalog);
        let mode_candidates =
            build_mode_candidates(mode, &prepared_mode, args, &target_windows, candidate_count, cancel)?;
        if layer_snapshots.is_empty() {
            frontier_stats.push(json!({
                "layer": index + 1,
                "mode": mode,
                "attempted": prepared_mode.selected.evaluated,
                "generated": prepared_mode.selected.kept,
                "beforeBeam": prepared_mode.ranked_early_candidates.len(),
                "kept": mode_candidates.len(),
                "bucketCount": bucket_count,
                "elapsedMs": 0,
            }));
        } else {
            for snapshot in layer_snapshots {
                frontier_stats.push(json!({
                    "layer": snapshot.layer,
                    "mode": mode,
                    "attempted": snapshot.evaluated_total,
                    "generated": snapshot.kept_total,
                    "beforeBeam": snapshot.expanded_prefixes,
                    "kept": snapshot.frontier_out,
                    "bucketCount": snapshot.frontier_out,
                    "frontierIn": snapshot.frontier_in,
                    "elapsedMs": 0,
                }));
            }
        }
        total_expanded += prepared_mode.selected.evaluated;
        kept_total += prepared_mode.selected.kept;
        deduped_total += prepared_mode.ranked_early_candidates.len();
        for state in mode_candidates {
            check_cancelled(cancel)?;
            candidate_prefix_rows.push(candidate_state_to_value(&state));
            let row = verify_row(&state, args.short_ticks, args.short_pairs, args);
            let passes_short = row
                .dwell
                .target_hit_rate
                .unwrap_or(row.dwell.strict_hit_rate.unwrap_or(0.0))
                >= args.min_short_hit_rate_for_long;
            if passes_short {
                short_passing_for_long += 1;
                if long_candidate_states.len() < args.long_limit {
                    long_candidate_states.push(state);
                }
            }
            short_verified.push(row);
        }
    }
    sort_verification_rows(&mut short_verified);

    prune_stats.insert(
        "filteredEarly".to_string(),
        json!({
            "count": total_expanded.saturating_sub(kept_total),
            "examples": [],
        }),
    );
    prune_stats.insert(
        "modeQuota".to_string(),
        json!({
            "count": deduped_total.saturating_sub(candidate_prefix_rows.len()),
            "examples": [],
        }),
    );

    let candidate_prefixes = json!({
        "generatedAt": timestamp_now_string()?,
        "generatorEngine": "rust",
        "totalExpanded": total_expanded,
        "candidates": candidate_prefix_rows,
    });
    write_json(&out_dir.join("candidate-prefixes.json"), &candidate_prefixes)?;
    write_json(&out_dir.join("frontier-stats.json"), &frontier_stats)?;
    write_json(&out_dir.join("prune-stats.json"), &prune_stats)?;

    check_cancelled(cancel)?;
    report_progress(
        &mut progress,
        ServiceProgressUpdate {
            stage: "verifying".to_string(),
            message: "verifying shortlisted candidates".to_string(),
            checked: Some(short_verified.len() as u64),
            total: Some(short_verified.len().max(1) as u64),
            percent: Some(90.0),
            candidate_count: Some(short_verified.len() as u64),
            unique_count: Some(candidate_prefix_rows.len() as u64),
            expanded_states: Some(total_expanded as u64),
            bucket_count: Some(deduped_total as u64),
        },
    );
    let long_verified = verify_rows(&long_candidate_states, args.long_ticks, args.long_pairs, args, cancel);
    check_cancelled(cancel)?;

    write_json(&out_dir.join("short-verified.json"), &short_verified)?;
    write_json(&out_dir.join("long-verified.json"), &long_verified)?;

    let config = json!({
        "generatedAt": timestamp_now_string()?,
        "script": "item-waterway-solver reachable-candidates",
        "args": args,
        "cycles": cycles.iter().map(|cycle| json!({
            "name": cycle.name,
            "period": cycle.cells.len(),
            "proven": cycle.proven,
        })).collect::<Vec<_>>(),
        "note": "Rust reachability candidate generator compatibility artifact set.",
    });
    write_json(&out_dir.join("generator-config.json"), &config)?;

    let summary = render_summary(
        args,
        total_expanded,
        &target_windows,
        &frontier_stats,
        &prune_stats,
        &short_verified,
        &long_verified,
    );
    write_text(&out_dir.join("summary.md"), &summary)?;

    Ok(RunArtifacts {
        out_dir: out_dir.clone(),
        short_verified: short_verified.clone(),
        short_passing_for_long,
        long_verified: long_verified.clone(),
        stdout_payload: json!({
            "outDir": out_dir.display().to_string(),
            "elapsedSeconds": 0.0,
            "generatorEngine": "rust",
            "requestedGeneratorEngine": args.requested_generator_engine,
            "verificationStatus": "short-long-solver-verified",
            "targetWindows": target_windows.iter().map(|(name, window)| {
                (name.clone(), json!({
                    "stable": window.stable_window.count,
                    "near": window.near_stable_window.count,
                    "accelerating": window.accelerating_trend_window.count,
                }))
            }).collect::<BTreeMap<_, _>>(),
            "expanded": total_expanded,
            "selectedCandidates": candidate_prefixes
                .get("candidates")
                .and_then(Value::as_array)
                .map(|rows| rows.len())
                .unwrap_or(0),
            "shortVerified": short_verified.len(),
            "shortPassingForLong": short_passing_for_long,
            "longVerified": long_verified.len(),
            "bestShort": short_verified.first().map(|row| {
                json!({
                    "mode": row.mode,
                    "cycle": row.cycle,
                    "prefixLabel": row.prefix_label,
                    "hitRate": row.dwell.strict_hit_rate,
                })
            }),
        }),
    })
}

fn report_progress(
    progress: &mut Option<&mut dyn FnMut(ServiceProgressUpdate)>,
    update: ServiceProgressUpdate,
) {
    if let Some(callback) = progress.as_deref_mut() {
        callback(update);
    }
}

fn prepare_mode(
    mode: &str,
    args: &CliArgs,
    cancel: Option<&AtomicBool>,
    progress: Option<&mut dyn FnMut(crate::reachability::PrepareProgress)>,
) -> Result<PreparedSearch, String> {
    let mode_start = mode_start(mode, args.start_overrides.as_ref())?;
    let search_args = crate::Args {
        out: PathBuf::from("artifacts").join("rust-reachable-candidates"),
        mode: crate::Mode::Early,
        ticks: args.short_ticks.max(32),
        top: args.top_candidates,
        max_prefix: args.max_prefix_cells,
        beam_width: args.beam_width,
        bucket_keep: args.bucket_keep,
        frontier_structure_keep: args.frontier_structure_keep,
        workers: args.workers,
        cadence_pairs: args.short_pairs.min(20).max(1),
        cadence_tolerance: args.cadence_tolerance,
        long_window: args.short_ticks.saturating_sub(10).clamp(10, 200),
        start_samples: 2,
        keep_weak: true,
        min_early_block_hit_rate: 0.0,
        early_limit: args.top_candidates,
        long_limit: args.long_limit,
        dedupe_long: true,
        full_cadence_pairs: args.long_pairs.min(args.long_ticks.saturating_div(2).saturating_sub(1).max(1)),
        full_cadence_tolerance: args.cadence_tolerance,
        fixed_start_offsets: Some(vec![mode_start.start_x]),
        cycle_names: Some(args.cycles.clone()),
        entity_id_mods: args.entity_mods.clone(),
        initial_tick_counts: args.initial_tick_mods.clone(),
        start_y: mode_start.start_y,
        start_vx: mode_start.start_vx,
        start_vy: mode_start.start_vy,
        start_on_ground: mode_start.start_on_ground,
    };
    reachability::prepare_cancelable(&search_args, cancel, progress)
}

fn requested_cycles(names: &[String]) -> Result<Vec<crate::CycleSpec>, String> {
    let by_name = crate::backbone_cycles()
        .into_iter()
        .map(|cycle| (cycle.name.clone(), cycle))
        .collect::<BTreeMap<_, _>>();
    names.iter()
        .map(|name| {
            by_name
                .get(name)
                .cloned()
                .ok_or_else(|| format!("Unknown cycle '{name}'"))
        })
        .collect()
}

fn build_mode_candidates(
    mode: &str,
    prepared_mode: &PreparedSearch,
    args: &CliArgs,
    target_windows: &BTreeMap<String, TargetWindowSummary>,
    limit: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<CandidateState>, String> {
    let mode_start = mode_start(mode, args.start_overrides.as_ref())?;
    let scan_limit = shortlist_scan_limit(limit, prepared_mode.ranked_early_candidates.len(), args);
    let mut states = Vec::new();
    let scan_candidates = select_scan_candidates_by_cycle(
        &prepared_mode.ranked_early_candidates,
        &prepared_mode.catalog,
        scan_limit,
        args,
    );
    for candidate in scan_candidates {
        check_cancelled(cancel)?;
        states.push(candidate_state(
            mode,
            candidate,
            &prepared_mode.catalog,
            &mode_start,
            args,
            target_windows,
        ));
    }
    Ok(select_shortlist_states(states, limit.max(1), args))
}

fn select_scan_candidates_by_cycle<'a>(
    candidates: &'a [SelectedCandidate],
    catalog: &CandidateCatalog,
    scan_limit: usize,
    args: &CliArgs,
) -> Vec<&'a SelectedCandidate> {
    if candidates.len() <= scan_limit {
        return candidates.iter().collect();
    }

    let weights = catalog
        .cycles
        .iter()
        .map(|cycle| {
            if cycle.proven {
                args.proven_cycle_beam_weight.max(1.0)
            } else {
                1.0
            }
        })
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f64>().max(1.0);
    let mut quotas = weights
        .iter()
        .map(|weight| {
            ((scan_limit as f64) * (*weight / total_weight))
                .round()
                .max(1.0) as usize
        })
        .collect::<Vec<_>>();
    let mut quota_total = quotas.iter().sum::<usize>();
    while quota_total > scan_limit {
        if let Some((index, _)) = quotas
            .iter()
            .enumerate()
            .filter(|(_, quota)| **quota > 1)
            .max_by_key(|(_, quota)| **quota)
        {
            quotas[index] -= 1;
            quota_total -= 1;
        } else {
            break;
        }
    }

    let mut selected = Vec::with_capacity(scan_limit.min(candidates.len()));
    let mut counts = vec![0_usize; catalog.cycles.len()];
    let mut selected_ids = BTreeSet::new();

    for candidate in candidates {
        let cycle_index = candidate.address.cycle_index;
        if counts[cycle_index] >= quotas[cycle_index] {
            continue;
        }
        if selected_ids.insert(candidate.id.clone()) {
            selected.push(candidate);
            counts[cycle_index] += 1;
            if selected.len() >= scan_limit {
                return selected;
            }
        }
    }

    for candidate in candidates {
        if selected_ids.insert(candidate.id.clone()) {
            selected.push(candidate);
            if selected.len() >= scan_limit {
                break;
            }
        }
    }

    selected
}

fn candidate_state(
    mode: &str,
    candidate: &SelectedCandidate,
    catalog: &CandidateCatalog,
    mode_start: &ModeStart,
    args: &CliArgs,
    target_windows: &BTreeMap<String, TargetWindowSummary>,
) -> CandidateState {
    let prefix = catalog.prefix(candidate);
    let cycle = catalog.cycle(candidate);
    let layout = catalog.layout(candidate);
    let verify_ticks = verification_ticks(prefix.cells.len(), args);
    let sim = crate::simulate(
        &layout,
        &crate::SimConfig {
            ticks: verify_ticks,
            start_x: mode_start.start_x,
            start_y: mode_start.start_y,
            start_vx: mode_start.start_vx,
            start_vy: mode_start.start_vy,
            entity_id_mod4: candidate.entity_id_mod4,
            initial_tick_count: candidate.initial_tick_count,
            start_on_ground: Some(mode_start.start_on_ground),
        },
    );
    let entry_tick = entry_tick(&sim, prefix.cells.len());
    let state_tick = entry_tick.unwrap_or_else(|| sim.xs.len().saturating_sub(1));
    let trend = state_trend(&sim, state_tick, args);
    let tick_mod4 = (candidate.initial_tick_count + state_tick) % 4;
    let complexity = modules_for_prefix(&prefix.label).iter().map(|module| module.cost).sum();
    let mut state = CandidateState {
        mode: mode.to_string(),
        candidate_id: candidate.id.clone(),
        early_score: candidate.early_score,
        selection_score: candidate.early_score,
        transient_shape_mae16: candidate.transient_shape_mae16,
        transient_shape_mae24: candidate.transient_shape_mae24,
        entity_id_mod4: candidate.entity_id_mod4,
        initial_tick_mod4: candidate.initial_tick_count,
        cycle_name: cycle.name.clone(),
        cycle_proven: cycle.proven,
        cycle_cells: cycle.cells.clone(),
        prefix_label: prefix.label.clone(),
        prefix_cells: prefix.cells.clone(),
        complexity,
        entry_reached: entry_tick.is_some(),
        entry_tick,
        x: sim.xs[state_tick],
        x_phase: fraction(sim.xs[state_tick]),
        vx: sim.vxs[state_tick],
        y: sim.ys[state_tick],
        vy: sim.vys[state_tick],
        on_ground: sim.on_grounds[state_tick] != 0,
        tick_mod4,
        target: TargetDistance {
            distance: 0.0,
            nearest_category: String::new(),
            nearest_vx: None,
            nearest_phase: None,
            phase_distance: None,
        },
        trend,
    };
    state.target = target_distance(&state, target_windows, args);
    state.selection_score = candidate_selection_score(&state, args);
    state
}

fn scan_target_windows(
    cycles: &[crate::CycleSpec],
    args: &CliArgs,
    cancel: Option<&AtomicBool>,
    progress: &mut Option<&mut dyn FnMut(ServiceProgressUpdate)>,
) -> BTreeMap<String, TargetWindowSummary> {
    let phases = (0..args.target_phase_samples)
        .map(|index| (index as f64 + 0.5) / args.target_phase_samples as f64)
        .collect::<Vec<_>>();
    let mut by_cycle = BTreeMap::new();
    let requests_per_cycle = phases.len()
        * args.target_vxs.len()
        * args.entity_mods.len()
        * args.initial_tick_mods.len();
    let total_requests = (cycles.len() * requests_per_cycle).max(1);
    let mut completed_requests = 0_usize;

    for cycle in cycles {
        if is_cancelled(cancel) {
            break;
        }
        let layout = crate::Layout::new(&[], &cycle.cells);
        let mut stable = Vec::new();
        let mut near = Vec::new();
        let mut accelerating = Vec::new();
        let mut requests = 0_usize;

        for phase in &phases {
            for vx in &args.target_vxs {
                for &entity_id_mod4 in &args.entity_mods {
                    for &initial_tick_mod4 in &args.initial_tick_mods {
                        if is_cancelled(cancel) {
                            return by_cycle;
                        }
                        requests += 1;
                        let sim = crate::simulate(
                            &layout,
                            &crate::SimConfig {
                                ticks: args.short_ticks,
                                start_x: *phase,
                                start_y: 0.0,
                                start_vx: *vx,
                                start_vy: 0.0,
                                entity_id_mod4,
                                initial_tick_count: initial_tick_mod4,
                                start_on_ground: Some(true),
                            },
                        );
                        let metric = target_sample_metric(&sim, args);
                        let block_hit_rate = metric
                            .full
                            .as_ref()
                            .map(|full| full.full_cadence_block_hit_rate)
                            .unwrap_or(0.0);
                        let strict_hit_rate = metric.dwell.strict_hit_rate.unwrap_or(0.0);
                        let target_hit_rate = metric.dwell.target_hit_rate.unwrap_or(0.0);
                        let avg_speed_error = (metric.average_speed - args.target_speed).abs();
                        let sample = TargetSample {
                            cycle: cycle.name.clone(),
                            category: String::new(),
                            start_x: *phase,
                            x_phase: *phase,
                            vx: *vx,
                            y: 0.0,
                            vy: 0.0,
                            on_ground: true,
                            entity_id_mod4,
                            initial_tick_mod4,
                            strict_hit_rate,
                            target_hit_rate,
                            block_hit_rate,
                            avg_speed: metric.average_speed,
                            trend_improvement: metric.trend_improvement,
                        };
                        if target_hit_rate >= 0.985 && avg_speed_error <= 0.01 {
                            let mut copy = sample.clone();
                            copy.category = "stableWindow".to_string();
                            stable.push(copy);
                        } else if target_hit_rate >= 0.9
                            || strict_hit_rate >= 0.9
                            || block_hit_rate >= 0.9
                            || (avg_speed_error <= 0.04 && !metric.overrun)
                        {
                            let mut copy = sample.clone();
                            copy.category = "nearStableWindow".to_string();
                            near.push(copy);
                        } else if *vx < args.target_speed
                            && metric.trend_improvement > 0.01
                            && metric.avg_end <= args.target_speed + 0.12
                            && !metric.overrun
                        {
                            let mut copy = sample.clone();
                            copy.category = "acceleratingTrendWindow".to_string();
                            accelerating.push(copy);
                        }
                    }
                }
            }
        }

        stable.sort_by(|left, right| {
            right
                .target_hit_rate
                .total_cmp(&left.target_hit_rate)
                .then_with(|| {
                    (left.avg_speed - args.target_speed)
                        .abs()
                        .total_cmp(&(right.avg_speed - args.target_speed).abs())
                })
        });
        near.sort_by(|left, right| {
            right
                .target_hit_rate
                .total_cmp(&left.target_hit_rate)
                .then_with(|| right.block_hit_rate.total_cmp(&left.block_hit_rate))
                .then_with(|| {
                    (left.avg_speed - args.target_speed)
                        .abs()
                        .total_cmp(&(right.avg_speed - args.target_speed).abs())
                })
        });
        accelerating.sort_by(|left, right| {
            right
                .trend_improvement
                .total_cmp(&left.trend_improvement)
        });
        let scoring_samples = stable
            .iter()
            .take(80)
            .chain(near.iter().take(80))
            .chain(accelerating.iter().take(80))
            .cloned()
            .collect::<Vec<_>>();
        by_cycle.insert(
            cycle.name.clone(),
            TargetWindowSummary {
                cycle: cycle.name.clone(),
                period: cycle.cells.len(),
                evaluated: requests,
                stable_window: range_summary(&stable),
                near_stable_window: range_summary(&near),
                accelerating_trend_window: range_summary(&accelerating),
                scoring_samples,
            },
        );
        completed_requests += requests;
        let percent = 5.0 + (completed_requests as f64 / total_requests as f64) * 10.0;
        report_progress(
            progress,
            ServiceProgressUpdate {
                stage: "searching".to_string(),
                message: format!("scanned target window for {}", cycle.name),
                checked: Some(completed_requests as u64),
                total: Some(total_requests as u64),
                percent: Some(percent.clamp(5.0, 15.0)),
                candidate_count: Some(0),
                unique_count: Some(by_cycle.len() as u64),
                expanded_states: Some(completed_requests as u64),
                bucket_count: Some(by_cycle.len() as u64),
            },
        );
    }

    by_cycle
}

fn target_sample_metric(sim: &crate::Simulation, args: &CliArgs) -> TargetMetric {
    let max_pairs = sim.xs.len().saturating_sub(1).saturating_div(2).saturating_sub(1);
    let pairs = args.short_pairs.min(max_pairs).max(1);
    let full = crate::full_cadence_metrics(sim, 0, pairs, args.cadence_tolerance);
    let dwell = dwell_metrics(sim, 0, 220, args.target_dwell_ticks, true, 0);
    let avg_start = average_speed(sim, 0, sim.xs.len().saturating_sub(1).min(40));
    let avg_end = average_speed(sim, sim.xs.len().saturating_sub(81), sim.xs.len().saturating_sub(1));
    let average_speed = average_speed(sim, 0, sim.xs.len().saturating_sub(1));
    let start_err = (avg_start - args.target_speed).abs();
    let end_err = (avg_end - args.target_speed).abs();
    let overrun = sim
        .vxs
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        > args.target_speed + 0.22;
    TargetMetric {
        full,
        dwell,
        average_speed,
        avg_end,
        trend_improvement: start_err - end_err,
        overrun,
    }
}

fn range_summary(samples: &[TargetSample]) -> RangeSummary {
    if samples.is_empty() {
        return RangeSummary {
            count: 0,
            x_phase_min: None,
            x_phase_max: None,
            vx_min: None,
            vx_max: None,
            avg_speed_min: None,
            avg_speed_max: None,
            tick_mods: Vec::new(),
            entity_mods: Vec::new(),
            examples: Vec::new(),
        };
    }

    let x_phase_values = samples.iter().map(|sample| sample.x_phase).collect::<Vec<_>>();
    let vx_values = samples.iter().map(|sample| sample.vx).collect::<Vec<_>>();
    let avg_speed_values = samples.iter().map(|sample| sample.avg_speed).collect::<Vec<_>>();

    RangeSummary {
        count: samples.len(),
        x_phase_min: x_phase_values.iter().copied().reduce(f64::min),
        x_phase_max: x_phase_values.iter().copied().reduce(f64::max),
        vx_min: vx_values.iter().copied().reduce(f64::min),
        vx_max: vx_values.iter().copied().reduce(f64::max),
        avg_speed_min: avg_speed_values.iter().copied().reduce(f64::min),
        avg_speed_max: avg_speed_values.iter().copied().reduce(f64::max),
        tick_mods: samples
            .iter()
            .map(|sample| sample.initial_tick_mod4)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        entity_mods: samples
            .iter()
            .map(|sample| sample.entity_id_mod4)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        examples: samples.iter().take(10).cloned().collect(),
    }
}

fn verify_rows(
    states: &[CandidateState],
    ticks: usize,
    pairs: usize,
    args: &CliArgs,
    cancel: Option<&AtomicBool>,
) -> Vec<VerificationRow> {
    let mut rows = Vec::with_capacity(states.len());
    for state in states {
        if is_cancelled(cancel) {
            break;
        }
        rows.push(verify_row(state, ticks, pairs, args));
    }
    sort_verification_rows(&mut rows);
    rows
}

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if is_cancelled(cancel) {
        Err(SEARCH_CANCELLED.to_string())
    } else {
        Ok(())
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn sort_verification_rows(rows: &mut [VerificationRow]) {
    rows.sort_by(|left, right| {
        let left_within_tolerance = left
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_within_tolerance_rate)
            .unwrap_or(0.0);
        let right_within_tolerance = right
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_within_tolerance_rate)
            .unwrap_or(0.0);
        let left_mean_abs = left
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_mean_abs_distance_error)
            .unwrap_or(f64::INFINITY);
        let right_mean_abs = right
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_mean_abs_distance_error)
            .unwrap_or(f64::INFINITY);
        let left_max_abs = left
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_max_abs_distance_error)
            .unwrap_or(f64::INFINITY);
        let right_max_abs = right
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_max_abs_distance_error)
            .unwrap_or(f64::INFINITY);
        let left_min_margin = left
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_min_hit_margin)
            .unwrap_or(f64::NEG_INFINITY);
        let right_min_margin = right
            .cadence
            .as_ref()
            .map(|value| value.full_cadence_min_hit_margin)
            .unwrap_or(f64::NEG_INFINITY);
        let left_transient16 = left.transient_shape_mae16.unwrap_or(f64::INFINITY);
        let right_transient16 = right.transient_shape_mae16.unwrap_or(f64::INFINITY);
        let left_transient24 = left.transient_shape_mae24.unwrap_or(f64::INFINITY);
        let right_transient24 = right.transient_shape_mae24.unwrap_or(f64::INFINITY);
        right
            .dwell
            .target_hit_rate
            .unwrap_or(right.dwell.strict_hit_rate.unwrap_or(0.0))
            .total_cmp(&left.dwell.target_hit_rate.unwrap_or(left.dwell.strict_hit_rate.unwrap_or(0.0)))
            .then_with(|| left.dwell.target_failures.cmp(&right.dwell.target_failures))
            .then_with(|| right_within_tolerance.total_cmp(&left_within_tolerance))
            .then_with(|| left_mean_abs.total_cmp(&right_mean_abs))
            .then_with(|| left_max_abs.total_cmp(&right_max_abs))
            .then_with(|| right_min_margin.total_cmp(&left_min_margin))
            .then_with(|| left_transient16.total_cmp(&right_transient16))
            .then_with(|| left_transient24.total_cmp(&right_transient24))
            .then_with(|| left.average_speed_error.total_cmp(&right.average_speed_error))
            .then_with(|| left.score.total_cmp(&right.score))
            .then_with(|| left.early_score.total_cmp(&right.early_score))
    });
}

fn verify_row(
    state: &CandidateState,
    ticks: usize,
    pairs: usize,
    args: &CliArgs,
) -> VerificationRow {
    let mode_start = mode_start(&state.mode, args.start_overrides.as_ref())
        .expect("validated mode start should always exist");
    let layout = crate::Layout::new(&state.prefix_cells, &state.cycle_cells);
    let sim = crate::simulate(
        &layout,
        &crate::SimConfig {
            ticks,
            start_x: mode_start.start_x,
            start_y: mode_start.start_y,
            start_vx: mode_start.start_vx,
            start_vy: mode_start.start_vy,
            entity_id_mod4: state.entity_id_mod4,
            initial_tick_count: state.initial_tick_mod4,
            start_on_ground: Some(mode_start.start_on_ground),
        },
    );

    let stable = first_stable_tick(&sim, args);
    let (dwell, raw_dwell, dwell_window) =
        candidate_dwell_window(&sim, state, stable.as_ref(), ticks, pairs, args);
    let cadence_start = stable
        .as_ref()
        .map(|value| value.tick)
        .unwrap_or_else(|| state.entry_tick.unwrap_or(0));
    let usable_pairs = pairs.min(sim.xs.len().saturating_sub(cadence_start + 1) / 2);
    let cadence = if usable_pairs > 0 {
        crate::full_cadence_metrics(&sim, cadence_start, usable_pairs, args.cadence_tolerance)
            .map(full_cadence_compat)
    } else {
        None
    };
    let average_speed = average_speed(&sim, 0, sim.xs.len().saturating_sub(1));
    let ideal_dx = sim.xs.len().saturating_sub(1) as f64 * args.target_speed;
    let actual_dx = sim.xs[sim.xs.len().saturating_sub(1)] - sim.xs[0];
    let terminal_position_fit = if ideal_dx > 0.0 {
        Some((1.0 - (actual_dx - ideal_dx).abs() / ideal_dx).max(0.0))
    } else {
        None
    };
    let max_vx = sim
        .vxs
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let transient_shape_mae16 = crate::uses_post_piston_game2_reference(
        mode_start.start_x,
        mode_start.start_vx,
        mode_start.start_on_ground,
    )
    .then(|| crate::post_piston_game2_transient_mae(&sim.xs, crate::TRANSIENT_SHAPE_WINDOW_SHORT))
    .flatten();
    let transient_shape_mae24 = crate::uses_post_piston_game2_reference(
        mode_start.start_x,
        mode_start.start_vx,
        mode_start.start_on_ground,
    )
    .then(|| crate::post_piston_game2_transient_mae(&sim.xs, crate::TRANSIENT_SHAPE_WINDOW_LONG))
    .flatten();
    VerificationRow {
        id: state.candidate_id.clone(),
        mode: state.mode.clone(),
        cycle: state.cycle_name.clone(),
        prefix_label: state.prefix_label.clone(),
        prefix_length: state.prefix_cells.len(),
        complexity: state.complexity,
        score: state.selection_score,
        early_score: state.early_score,
        entity_id_mod4: state.entity_id_mod4,
        initial_tick_mod4: state.initial_tick_mod4,
        ticks,
        dwell,
        raw_dwell,
        dwell_window,
        cadence,
        average_speed,
        average_speed_error: (average_speed - args.target_speed).abs(),
        transient_shape_mae16,
        transient_shape_mae24,
        terminal_position_fit,
        acceleration_ticks: stable.as_ref().filter(|value| value.stable_like).map(|value| value.tick),
        first_stable_tick: stable,
        overshoot: max_vx > args.target_speed + 0.22,
        max_vx,
        entry_tick: state.entry_tick,
        target: state.target.clone(),
        first_ticks: first_ticks(&sim),
        prefix_cells: cell_descriptions(&state.prefix_cells),
        cycle_cells: cell_descriptions(&state.cycle_cells),
    }
}

fn candidate_dwell_window(
    sim: &crate::Simulation,
    state: &CandidateState,
    stable: Option<&StableTick>,
    ticks: usize,
    pairs: usize,
    args: &CliArgs,
) -> (DwellMetrics, DwellMetrics, DwellWindow) {
    let cycle_start = state.prefix_cells.len() as i64;
    let raw_target_blocks = (ticks / 2).min(pairs) as i64;
    let raw_dwell = dwell_metrics(
        sim,
        cycle_start,
        cycle_start + raw_target_blocks.saturating_sub(1),
        args.target_dwell_ticks,
        false,
        0,
    );
    let use_stable = should_use_stable_dwell(&state.mode, stable, args);
    if !use_stable {
        let window = DwellWindow {
            mode: "cycle".to_string(),
            min_block: raw_dwell.min_block,
            max_block: raw_dwell.max_block,
            include_final_group: false,
            stable_start_tick: None,
            min_start_tick: raw_dwell.min_start_tick,
            stable_block_hit_rate: None,
            stable_average_speed: None,
            stable_mean_abs_distance_error: None,
        };
        return (raw_dwell.clone(), raw_dwell, window);
    }

    let stable = stable.expect("stable dwell window only when stable tick exists");
    let stable_min_block = (sim.xs[stable.tick].floor() as i64).max(cycle_start);
    let stable_max_block = sim.xs[sim.xs.len().saturating_sub(1)].floor() as i64;
    if stable_min_block > stable_max_block {
        let window = DwellWindow {
            mode: "cycle-fallback".to_string(),
            min_block: raw_dwell.min_block,
            max_block: raw_dwell.max_block,
            include_final_group: false,
            stable_start_tick: Some(stable.tick),
            min_start_tick: raw_dwell.min_start_tick,
            stable_block_hit_rate: None,
            stable_average_speed: None,
            stable_mean_abs_distance_error: None,
        };
        return (raw_dwell.clone(), raw_dwell, window);
    }

    let stable_dwell = dwell_metrics(
        sim,
        stable_min_block,
        stable_max_block,
        args.target_dwell_ticks,
        false,
        stable.tick,
    );
    let window = DwellWindow {
        mode: "stable".to_string(),
        min_block: stable_dwell.min_block,
        max_block: stable_dwell.max_block,
        include_final_group: false,
        stable_start_tick: Some(stable.tick),
        min_start_tick: stable_dwell.min_start_tick,
        stable_block_hit_rate: Some(stable.block_hit_rate),
        stable_average_speed: Some(stable.average_speed),
        stable_mean_abs_distance_error: Some(stable.mean_abs_distance_error),
    };
    (stable_dwell, raw_dwell, window)
}

fn should_use_stable_dwell(mode: &str, stable: Option<&StableTick>, args: &CliArgs) -> bool {
    let Some(stable) = stable else {
        return false;
    };
    if !stable.stable_like {
        return false;
    }
    match args.steady_dwell_mode.as_str() {
        "stable" => true,
        "cycle" => false,
        _ => matches!(mode, "water-accelerate" | "hybrid"),
    }
}

fn first_stable_tick(sim: &crate::Simulation, args: &CliArgs) -> Option<StableTick> {
    let pairs = sim.xs.len().saturating_sub(1).saturating_div(2).saturating_sub(1).min(20);
    if pairs == 0 {
        return None;
    }
    let mut best: Option<StableTick> = None;
    for start_tick in 0..sim.xs.len() {
        if start_tick + pairs * 2 >= sim.xs.len() {
            break;
        }
        let Some(full) = crate::full_cadence_metrics(sim, start_tick, pairs, args.cadence_tolerance) else {
            continue;
        };
        let score = (1.0 - full.full_cadence_block_hit_rate) * 100.0
            + full.full_cadence_mean_abs_distance_error * 10.0
            + (full.full_cadence_average_speed - args.target_speed).abs() * 20.0
            + start_tick as f64 * 0.01;
        let current = StableTick {
            score,
            tick: start_tick,
            stable_like: false,
            block_hit_rate: full.full_cadence_block_hit_rate,
            average_speed: full.full_cadence_average_speed,
            mean_abs_distance_error: full.full_cadence_mean_abs_distance_error,
        };
        if best
            .as_ref()
            .map(|value| score < value.score)
            .unwrap_or(true)
        {
            best = Some(current.clone());
        }
        if full.full_cadence_block_hit_rate >= 0.98
            && full.full_cadence_mean_abs_distance_error <= 0.05
            && (full.full_cadence_average_speed - args.target_speed).abs() <= 0.02
        {
            return Some(StableTick {
                stable_like: true,
                ..current
            });
        }
    }
    best
}

fn dwell_metrics(
    sim: &crate::Simulation,
    min_block: i64,
    max_block: i64,
    target_dwell_ticks: usize,
    include_final_group: bool,
    min_start_tick: usize,
) -> DwellMetrics {
    let mut groups: Vec<Vec<FailurePoint>> = Vec::new();
    let mut current: Vec<FailurePoint> = Vec::new();
    for tick in 0..sim.xs.len() {
        let x = sim.xs[tick];
        let block = x.floor() as i64;
        let item = FailurePoint {
            tick,
            x,
            block,
            vx: sim.vxs.get(tick).copied(),
        };
        if current.is_empty() || current.last().map(|value| value.block == block).unwrap_or(false) {
            current.push(item);
        } else {
            groups.push(current);
            current = vec![item];
        }
    }
    if include_final_group && !current.is_empty() {
        groups.push(current);
    }

    let eligible = groups
        .into_iter()
        .filter(|group| {
            group.first().map(|item| item.tick >= min_start_tick).unwrap_or(false)
                && group.first().map(|item| item.block >= min_block).unwrap_or(false)
                && group.first().map(|item| item.block <= max_block).unwrap_or(false)
        })
        .collect::<Vec<_>>();

    let mut count_dist = BTreeMap::new();
    let mut failures = Vec::new();
    let mut target_failures = Vec::new();
    for group in &eligible {
        *count_dist.entry(group.len().to_string()).or_insert(0) += 1;
        if group.len() != 2 {
            failures.push(group.clone());
        }
        if group.len() != target_dwell_ticks {
            target_failures.push(group.clone());
        }
    }
    let exact2 = eligible.len().saturating_sub(failures.len());
    let target_exact = eligible.len().saturating_sub(target_failures.len());
    DwellMetrics {
        min_block,
        max_block,
        min_start_tick,
        blocks: eligible.len(),
        exact2,
        failures: failures.len(),
        strict_hit_rate: (!eligible.is_empty()).then_some(exact2 as f64 / eligible.len() as f64),
        target_dwell_ticks,
        target_exact,
        target_failures: target_failures.len(),
        target_hit_rate: (!eligible.is_empty()).then_some(target_exact as f64 / eligible.len() as f64),
        count_dist,
        first_failure: failures.first().cloned(),
        first_target_failure: target_failures.first().cloned(),
    }
}

fn target_distance(
    state: &CandidateState,
    target_windows: &BTreeMap<String, TargetWindowSummary>,
    args: &CliArgs,
) -> TargetDistance {
    let Some(windows) = target_windows.get(&state.cycle_name) else {
        return TargetDistance {
            distance: 2.0 + (state.vx - args.target_speed).abs() / 0.1 + state.trend.two_gt_error,
            nearest_category: "noTargetWindowFallback".to_string(),
            nearest_vx: None,
            nearest_phase: None,
            phase_distance: None,
        };
    };
    if windows.scoring_samples.is_empty() {
        return TargetDistance {
            distance: 2.0 + (state.vx - args.target_speed).abs() / 0.1 + state.trend.two_gt_error,
            nearest_category: "noTargetWindowFallback".to_string(),
            nearest_vx: None,
            nearest_phase: None,
            phase_distance: None,
        };
    }
    let pool = windows
        .scoring_samples
        .iter()
        .filter(|sample| {
            if state.mode == "water-accelerate" {
                true
            } else if state.mode == "launch-fast" {
                sample.category != "acceleratingTrendWindow"
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    let usable = if pool.is_empty() {
        windows.scoring_samples.iter().collect::<Vec<_>>()
    } else {
        pool
    };
    let mut best: Option<TargetDistance> = None;
    for sample in usable {
        let pd = phase_distance(state.x_phase, sample.x_phase);
        let tick_penalty = if state.tick_mod4 == sample.initial_tick_mod4 { 0.0 } else { 0.3 };
        let entity_penalty = if state.entity_id_mod4 == sample.entity_id_mod4 {
            0.0
        } else {
            0.15
        };
        let value = if state.mode == "water-accelerate" && sample.category == "acceleratingTrendWindow" {
            let macro_speed = state.trend.avg_recent_vx;
            let speed_delta = (macro_speed - sample.avg_speed).abs();
            let trend_delta =
                state.trend.speed_error_improvement.max(0.0) - sample.trend_improvement.max(0.0);
            let two_gt_delta = (state.trend.avg_two_gt_distance
                - (sample.avg_speed * args.target_dwell_ticks as f64))
                .abs();
            pd / 0.08
                + speed_delta / 0.04
                + trend_delta.abs() / 0.08
                + two_gt_delta / 0.16
                + tick_penalty * 0.75
                + entity_penalty * 0.75
        } else {
            let vd = (state.vx - sample.vx).abs();
            pd / 0.08 + vd / 0.05 + tick_penalty + entity_penalty
        };
        if best.as_ref().map(|current| value < current.distance).unwrap_or(true) {
            best = Some(TargetDistance {
                distance: value,
                nearest_category: sample.category.clone(),
                nearest_vx: Some(sample.vx),
                nearest_phase: Some(sample.x_phase),
                phase_distance: Some(pd),
            });
        }
    }
    best.unwrap_or(TargetDistance {
        distance: 2.0 + (state.vx - args.target_speed).abs() / 0.1 + state.trend.two_gt_error,
        nearest_category: "noTargetWindowFallback".to_string(),
        nearest_vx: None,
        nearest_phase: None,
        phase_distance: None,
    })
}

fn state_trend(sim: &crate::Simulation, tick: usize, args: &CliArgs) -> TrendSummary {
    let target_speed = args.target_speed;
    let target_distance = args.target_speed * args.target_dwell_ticks as f64;
    let recent_start = tick.saturating_sub(10);
    let macro_start = tick.saturating_sub(24);
    let early_end = tick.min(10);
    let early = (0..=early_end).map(|t| sim.vxs[t]).collect::<Vec<_>>();
    let recent = (recent_start..=tick).map(|t| sim.vxs[t]).collect::<Vec<_>>();
    let two_gt = recent_two_gt_distances(sim, tick, 8);
    let first_two_gt = two_gt[..(two_gt.len() + 1) / 2].to_vec();
    let last_two_gt = two_gt[two_gt.len() / 2..].to_vec();
    let avg_early_vx = average(&early).unwrap_or(sim.vxs[0]);
    let avg_recent_vx = average(&recent).unwrap_or(sim.vxs[tick]);
    let recent_mid = recent.len().max(1) / 2;
    let avg_recent_front_vx = average(&recent[..recent_mid]).unwrap_or(avg_recent_vx);
    let avg_recent_back_vx = average(&recent[recent_mid..]).unwrap_or(avg_recent_vx);
    let avg_two_gt_distance = average(&two_gt).unwrap_or(target_distance);
    TrendSummary {
        avg_early_vx,
        avg_recent_vx,
        avg_recent_front_vx,
        avg_recent_back_vx,
        recent_velocity_slope: avg_recent_back_vx - avg_recent_front_vx,
        recent_error_improvement: (avg_recent_front_vx - target_speed).abs()
            - (avg_recent_back_vx - target_speed).abs(),
        macro_average_speed: average_speed(sim, macro_start, tick),
        speed_error_early: (avg_early_vx - target_speed).abs(),
        speed_error_recent: (avg_recent_vx - target_speed).abs(),
        speed_error_improvement: (avg_early_vx - target_speed).abs()
            - (avg_recent_vx - target_speed).abs(),
        avg_two_gt_distance,
        two_gt_error: (avg_two_gt_distance - target_distance).abs(),
        first_two_gt_error: (!first_two_gt.is_empty())
            .then(|| (average(&first_two_gt).unwrap_or(0.0) - target_distance).abs()),
        last_two_gt_error: (!last_two_gt.is_empty())
            .then(|| (average(&last_two_gt).unwrap_or(0.0) - target_distance).abs()),
    }
}

fn average_speed(sim: &crate::Simulation, start_tick: usize, end_tick: usize) -> f64 {
    let start = start_tick.min(sim.xs.len().saturating_sub(1));
    let end = end_tick.clamp(start + 1, sim.xs.len().saturating_sub(1));
    (sim.xs[end] - sim.xs[start]) / (end - start) as f64
}

fn recent_two_gt_distances(sim: &crate::Simulation, tick: usize, count: usize) -> Vec<f64> {
    let start = tick.saturating_sub(count * 2).max(2);
    let mut values = Vec::new();
    let mut t = start;
    while t <= tick {
        if t >= 2 && t < sim.xs.len() {
            values.push(sim.xs[t] - sim.xs[t - 2]);
        }
        t += 2;
    }
    values
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then_some(values.iter().sum::<f64>() / values.len() as f64)
}

fn entry_tick(sim: &crate::Simulation, prefix_length: usize) -> Option<usize> {
    let target_x = prefix_length as f64;
    sim.xs.iter().position(|x| *x >= target_x)
}

fn verification_ticks(prefix_length: usize, args: &CliArgs) -> usize {
    args.entry_tick_pad
        .max(args.entry_tick_pad + (prefix_length as f64 * args.entry_ticks_per_cell as f64).ceil() as usize)
}

fn mode_start(mode: &str, overrides: Option<&StartOverrides>) -> Result<ModeStart, String> {
    let mut start = match mode {
        "launch-fast" => ModeStart {
            start_x: 0.125,
            start_y: 0.0,
            start_vx: 1.0,
            start_vy: 0.0,
            start_on_ground: false,
        },
        "water-accelerate" => ModeStart {
            start_x: 0.125,
            start_y: 0.0,
            start_vx: 0.0,
            start_vy: 0.0,
            start_on_ground: true,
        },
        "hybrid" => ModeStart {
            start_x: 0.125,
            start_y: 0.0,
            start_vx: 0.35,
            start_vy: 0.0,
            start_on_ground: true,
        },
        other => {
            return Err(format!(
                "Unknown mode '{other}'. Expected launch-fast, water-accelerate, or hybrid."
            ))
        }
    };
    if let Some(overrides) = overrides {
        if let Some(value) = overrides.start_x {
            start.start_x = value;
        }
        if let Some(value) = overrides.start_y {
            start.start_y = value;
        }
        if let Some(value) = overrides.start_vx {
            start.start_vx = value;
        }
        if let Some(value) = overrides.start_vy {
            start.start_vy = value;
        }
        if let Some(value) = overrides.start_on_ground {
            start.start_on_ground = value;
        }
    }
    Ok(start)
}

fn modules_for_prefix(label: &str) -> Vec<ModuleInfo> {
    if label == "none" {
        return Vec::new();
    }
    let metadata = module_metadata();
    let known_names = metadata.keys().map(String::as_str).collect::<Vec<_>>();
    let Ok(tokens) = crate::parse_prefix_label_tokens(label, &known_names) else {
        return Vec::new();
    };
    tokens
        .into_iter()
        .filter_map(|name| metadata.get(&name).cloned())
        .collect()
}

fn physical_prefix_label(label: &str) -> String {
    match crate::parse_prefix_atoms_from_label(label) {
        Ok(atoms) => atoms
            .iter()
            .map(|atom| atom.cells.clone())
            .fold(Vec::new(), |mut acc, mut cells| {
                acc.append(&mut cells);
                acc
            })
            .iter()
            .enumerate()
            .fold(String::new(), |mut acc, (index, cell)| {
                if index > 0 {
                    acc.push('-');
                }
                acc.push_str(&cell.code());
                acc
            }),
        Err(_) => label.to_string(),
    }
}

fn module_metadata() -> HashMap<String, ModuleInfo> {
    let mut map = HashMap::new();
    for entry in [
        ("DN", "brake", 1, 1.8, 1),
        ("DI", "phaseAdjust", 1, 1.0, 2),
        ("DB", "phaseAdjust", 1, 1.1, 2),
        ("D2B", "phaseAdjust", 2, 2.2, 2),
        ("DS", "phaseAdjust", 1, 1.4, 2),
        ("SN", "brake", 1, 2.5, 1),
        ("SI", "phaseAdjust", 1, 2.0, 2),
        ("SB", "phaseAdjust", 1, 2.2, 2),
        ("R2N", "brake", 2, 5.0, 1),
        ("R2I", "brake", 2, 4.8, 1),
        ("R2B", "brake", 2, 4.8, 1),
        ("R3N", "brake", 3, 6.4, 1),
        ("R3I", "brake", 3, 6.0, 1),
        ("R3B", "brake", 3, 6.2, 1),
        ("F2N", "accelerator", 2, 3.2, 0),
        ("F2I", "accelerator", 2, 3.0, 0),
        ("F2B", "accelerator", 2, 3.4, 0),
        ("F3I", "accelerator", 3, 4.2, 0),
        ("F3B", "accelerator", 3, 4.6, 0),
        ("F4I", "accelerator", 4, 5.5, 0),
        ("F4B", "accelerator", 4, 6.0, 0),
        ("F5I", "accelerator", 5, 6.8, 0),
        ("F5B", "accelerator", 5, 7.4, 0),
        ("FS4I", "accelerator", 4, 5.1, 0),
        ("F2I-stab", "stabilizer", 2, 3.2, 3),
        ("F3I-stab", "stabilizer", 3, 4.4, 3),
        ("DB-stab", "stabilizer", 1, 1.4, 3),
    ] {
        map.insert(
            entry.0.to_string(),
            ModuleInfo {
                name: entry.0.to_string(),
                role: entry.1.to_string(),
                length: entry.2,
                cost: entry.3,
                stage: entry.4,
            },
        );
    }
    map
}

fn full_cadence_compat(full: crate::FullCadence) -> FullCadenceCompat {
    FullCadenceCompat {
        full_cadence_start_tick: full.full_cadence_start_tick,
        full_cadence_pairs: full.full_cadence_pairs,
        full_cadence_mean_abs_distance_error: full.full_cadence_mean_abs_distance_error,
        full_cadence_mean_signed_distance_error: full.full_cadence_mean_signed_distance_error,
        full_cadence_max_abs_distance_error: full.full_cadence_max_abs_distance_error,
        full_cadence_block_hit_rate: full.full_cadence_block_hit_rate,
        full_cadence_within_tolerance_rate: full.full_cadence_within_tolerance_rate,
        full_cadence_longest_hit_run: full.full_cadence_longest_hit_run,
        full_cadence_first_miss: full.full_cadence_first_miss,
        full_cadence_min_hit_margin: full.full_cadence_min_hit_margin,
        full_cadence_mean_hit_margin: full.full_cadence_mean_hit_margin,
        full_cadence_min_endpoint_boundary_margin: full.full_cadence_min_endpoint_boundary_margin,
        full_cadence_mean_endpoint_boundary_margin: full.full_cadence_mean_endpoint_boundary_margin,
        full_cadence_samples: full.full_cadence_samples,
        full_cadence_distance: full.full_cadence_distance,
        full_cadence_average_speed: full.full_cadence_average_speed,
    }
}

fn cell_descriptions(cells: &[crate::Cell]) -> Vec<crate::CellDescription> {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| crate::CellDescription {
            index,
            surface: cell.surface,
            flow: cell.flow,
            derived_flow_hint: (cell.amount == 0).then_some(0),
            amount: cell.amount,
            floor: cell.floor.as_str().to_string(),
            code: cell.code(),
        })
        .collect()
}

fn first_ticks(sim: &crate::Simulation) -> Vec<Value> {
    (0..sim.xs.len().min(12))
        .map(|tick| {
            json!({
                "tick": tick,
                "x": sim.xs[tick],
                "y": sim.ys[tick],
                "vx": sim.vxs[tick],
                "vy": sim.vys[tick],
                "onGround": sim.on_grounds[tick] != 0,
            })
        })
        .collect()
}

fn candidate_state_to_value(state: &CandidateState) -> Value {
    json!({
        "id": state.candidate_id,
        "mode": state.mode,
        "cycle": state.cycle_name,
        "cycleProven": state.cycle_proven,
        "prefixLabel": state.prefix_label,
        "prefixLength": state.prefix_cells.len(),
        "complexity": state.complexity,
        "score": state.selection_score,
        "earlyScore": state.early_score,
        "entryReached": state.entry_reached,
        "entryTick": state.entry_tick,
        "x": state.x,
        "xPhase": state.x_phase,
        "vx": state.vx,
        "y": state.y,
        "vy": state.vy,
        "onGround": state.on_ground,
        "tickMod4": state.tick_mod4,
        "entityIdMod4": state.entity_id_mod4,
        "initialTickMod4": state.initial_tick_mod4,
        "transientShapeMae16": state.transient_shape_mae16,
        "transientShapeMae24": state.transient_shape_mae24,
        "target": state.target,
        "trend": state.trend,
        "modules": modules_for_prefix(&state.prefix_label),
    })
}

fn shortlist_scan_limit(limit: usize, available: usize, args: &CliArgs) -> usize {
    let base = limit.max(1);
    let prefix_factor = if args.max_prefix_cells >= 12 {
        20
    } else if args.max_prefix_cells >= 9 {
        12
    } else {
        8
    };
    let cycle_factor = (args.cycles.len().saturating_add(7) / 8).clamp(1, 3);
    base.saturating_mul(prefix_factor)
        .saturating_mul(cycle_factor)
        .max(base.saturating_add(160))
        .min(available)
        .clamp(base, 12_000)
}

fn select_shortlist_states(
    mut states: Vec<CandidateState>,
    limit: usize,
    args: &CliArgs,
) -> Vec<CandidateState> {
    if states.len() <= limit {
        states.sort_by(compare_candidate_states_for_selection);
        return states;
    }

    let reserve_budget = bridge_reserve_budget(limit, args);
    let bridge_reserve = select_bridge_reserve_states(&states, reserve_budget, args);
    if bridge_reserve.is_empty() {
        return select_diverse_states(states, limit);
    }

    states.sort_by(compare_candidate_states_for_selection);
    let selected_keys = bridge_reserve
        .iter()
        .map(candidate_family_key)
        .collect::<BTreeSet<_>>();
    let remaining = states
        .into_iter()
        .filter(|state| !selected_keys.contains(&candidate_family_key(state)))
        .collect::<Vec<_>>();

    let mut selected = bridge_reserve;
    selected.extend(select_cycle_balanced_states(
        remaining,
        limit.saturating_sub(selected.len()),
        args,
    ));
    selected.sort_by(compare_candidate_states_for_selection);
    selected.truncate(limit);
    selected
}

fn select_cycle_balanced_states(
    states: Vec<CandidateState>,
    limit: usize,
    args: &CliArgs,
) -> Vec<CandidateState> {
    if states.len() <= limit {
        let mut ordered = states;
        ordered.sort_by(compare_candidate_states_for_selection);
        return ordered;
    }

    let mut by_cycle = BTreeMap::<String, Vec<CandidateState>>::new();
    for state in states {
        by_cycle.entry(state.cycle_name.clone()).or_default().push(state);
    }
    if by_cycle.len() <= 1 {
        return select_diverse_states(
            by_cycle.into_values().flatten().collect(),
            limit,
        );
    }

    let cycle_names = by_cycle.keys().cloned().collect::<Vec<_>>();
    let weights = cycle_names
        .iter()
        .map(|cycle_name| {
            let sample = by_cycle
                .get(cycle_name)
                .and_then(|states| states.first())
                .expect("cycle group should have entries");
            if sample.cycle_proven {
                args.proven_cycle_beam_weight.max(1.0)
            } else {
                1.0
            }
        })
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f64>().max(1.0);
    let mut quotas = weights
        .iter()
        .map(|weight| ((limit as f64) * (*weight / total_weight)).round().max(1.0) as usize)
        .collect::<Vec<_>>();
    let mut quota_total = quotas.iter().sum::<usize>();
    while quota_total > limit {
        if let Some((index, _)) = quotas
            .iter()
            .enumerate()
            .filter(|(_, quota)| **quota > 1)
            .max_by_key(|(_, quota)| **quota)
        {
            quotas[index] -= 1;
            quota_total -= 1;
        } else {
            break;
        }
    }

    let mut selected = Vec::with_capacity(limit);
    let mut selected_keys = BTreeSet::new();
    for (index, cycle_name) in cycle_names.iter().enumerate() {
        let Some(group) = by_cycle.get(cycle_name).cloned() else {
            continue;
        };
        for state in select_diverse_states(group, quotas[index]) {
            let family = candidate_family_key(&state);
            if selected_keys.insert(family) {
                selected.push(state);
            }
        }
    }

    let leftovers = by_cycle
        .into_values()
        .flatten()
        .filter(|state| !selected_keys.contains(&candidate_family_key(state)))
        .collect::<Vec<_>>();
    selected.extend(select_diverse_states(
        leftovers,
        limit.saturating_sub(selected.len()),
    ));
    selected.sort_by(compare_candidate_states_for_selection);
    selected.truncate(limit);
    selected
}

fn select_diverse_states(
    mut states: Vec<CandidateState>,
    limit: usize,
) -> Vec<CandidateState> {
    if states.len() <= limit {
        states.sort_by(compare_candidate_states_for_selection);
        return states;
    }

    states.sort_by(compare_candidate_states_for_selection);
    let mut groups = BTreeMap::<String, Vec<CandidateState>>::new();
    for state in states {
        groups
            .entry(candidate_family_key(&state))
            .or_default()
            .push(state);
    }

    let mut keys = groups.keys().cloned().collect::<Vec<_>>();
    keys.sort_by(|left, right| {
        compare_candidate_states_for_selection(&groups[left][0], &groups[right][0])
            .then_with(|| left.cmp(right))
    });

    let mut selected = Vec::with_capacity(limit);
    let mut depth = 0_usize;
    while selected.len() < limit {
        let mut progressed = false;
        for key in &keys {
            let group = &groups[key];
            if depth < group.len() {
                selected.push(group[depth].clone());
                progressed = true;
                if selected.len() >= limit {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        depth += 1;
    }
    selected
}

fn candidate_family_key(state: &CandidateState) -> String {
    format!(
        "{}|{}",
        state.cycle_name,
        physical_prefix_label(&state.prefix_label)
    )
}

fn bridge_reserve_budget(limit: usize, args: &CliArgs) -> usize {
    if limit <= 8 || args.bridge_beam_share <= 0.0 {
        return 0;
    }
    ((limit as f64 * args.bridge_beam_share).round() as usize).clamp(4, (limit / 2).max(4))
}

fn select_bridge_reserve_states(
    states: &[CandidateState],
    limit: usize,
    args: &CliArgs,
) -> Vec<CandidateState> {
    if limit == 0 {
        return Vec::new();
    }

    let mut by_cycle = BTreeMap::<String, Vec<CandidateState>>::new();
    for state in states.iter().filter(|state| bridge_reserve_eligible(state, args)) {
        by_cycle
            .entry(state.cycle_name.clone())
            .or_default()
            .push(state.clone());
    }
    if by_cycle.is_empty() {
        return Vec::new();
    }

    for list in by_cycle.values_mut() {
        list.sort_by(|left, right| {
            bridge_reserve_score(left, args)
                .total_cmp(&bridge_reserve_score(right, args))
                .then_with(|| compare_candidate_states_for_selection(left, right))
        });
    }

    let mut cycle_order = by_cycle.keys().cloned().collect::<Vec<_>>();
    cycle_order.sort_by(|left, right| {
        let left_best = by_cycle
            .get(left)
            .and_then(|list| list.first())
            .expect("bridge reserve cycle should have entries");
        let right_best = by_cycle
            .get(right)
            .and_then(|list| list.first())
            .expect("bridge reserve cycle should have entries");
        bridge_reserve_score(left_best, args)
            .total_cmp(&bridge_reserve_score(right_best, args))
            .then_with(|| left.cmp(right))
    });

    let mut selected = Vec::with_capacity(limit.min(states.len()));
    let mut selected_keys = BTreeSet::new();
    let mut depth = 0_usize;
    while selected.len() < limit {
        let mut progressed = false;
        for cycle_name in &cycle_order {
            let Some(group) = by_cycle.get(cycle_name) else {
                continue;
            };
            if depth >= group.len() {
                continue;
            }
            let state = &group[depth];
            let family = candidate_family_key(state);
            if selected_keys.insert(family) {
                selected.push(state.clone());
                progressed = true;
                if selected.len() >= limit {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        depth += 1;
    }
    selected
}

fn bridge_reserve_eligible(state: &CandidateState, args: &CliArgs) -> bool {
    let long_enough = state.prefix_cells.len() >= args.max_prefix_cells.saturating_sub(3).max(5);
    let late_two_gt_error = state
        .trend
        .last_two_gt_error
        .unwrap_or(state.trend.two_gt_error);
    state.cycle_proven
        && state.entry_reached
        && long_enough
        && state.trend.speed_error_improvement >= 0.035
        && state.trend.speed_error_recent <= 0.28
        && late_two_gt_error <= 0.55
        && state.target.distance <= 3.5
}

fn bridge_reserve_score(state: &CandidateState, args: &CliArgs) -> f64 {
    let late_two_gt_error = state
        .trend
        .last_two_gt_error
        .unwrap_or(state.trend.two_gt_error);
    let first_two_gt_error = state
        .trend
        .first_two_gt_error
        .unwrap_or(state.trend.two_gt_error);
    let slope_penalty = if state.trend.avg_recent_vx > args.target_speed {
        state.trend.recent_velocity_slope.max(0.0)
    } else {
        (-state.trend.recent_velocity_slope).max(0.0)
    };
    state.target.distance * 20.0
        + state.trend.speed_error_recent * 150.0
        + late_two_gt_error * 90.0
        + slope_penalty * 180.0
        + state.early_score * 0.04
        + state.complexity * 0.25
        - state.trend.speed_error_improvement * 180.0
        - (first_two_gt_error - late_two_gt_error).max(0.0) * 160.0
        - state.prefix_cells.len() as f64 * 1.1
}

fn compare_candidate_states_for_selection(
    left: &CandidateState,
    right: &CandidateState,
) -> std::cmp::Ordering {
    left.selection_score
        .total_cmp(&right.selection_score)
        .then_with(|| {
            left.transient_shape_mae16
                .unwrap_or(f64::INFINITY)
                .total_cmp(&right.transient_shape_mae16.unwrap_or(f64::INFINITY))
        })
        .then_with(|| {
            left.transient_shape_mae24
                .unwrap_or(f64::INFINITY)
                .total_cmp(&right.transient_shape_mae24.unwrap_or(f64::INFINITY))
        })
        .then_with(|| left.target.distance.total_cmp(&right.target.distance))
        .then_with(|| left.trend.two_gt_error.total_cmp(&right.trend.two_gt_error))
        .then_with(|| left.early_score.total_cmp(&right.early_score))
        .then_with(|| left.prefix_label.cmp(&right.prefix_label))
        .then_with(|| left.cycle_name.cmp(&right.cycle_name))
        .then_with(|| left.candidate_id.cmp(&right.candidate_id))
}

fn candidate_selection_score(state: &CandidateState, args: &CliArgs) -> f64 {
    let target_speed_error = (state.trend.avg_recent_vx - args.target_speed).abs();
    let macro_speed_error = (state.trend.macro_average_speed - args.target_speed).abs();
    let late_two_gt_error = state
        .trend
        .last_two_gt_error
        .unwrap_or(state.trend.two_gt_error);
    let first_two_gt_error = state
        .trend
        .first_two_gt_error
        .unwrap_or(state.trend.two_gt_error);
    let convergence_bonus = state.trend.speed_error_improvement.max(0.0);
    let late_improvement_bonus = (first_two_gt_error - late_two_gt_error).max(0.0);
    let transient_penalty = state.transient_shape_mae16.unwrap_or(0.0) * crate::TRANSIENT_SHAPE_SCORE_WEIGHT
        + state.transient_shape_mae24.unwrap_or(0.0) * (crate::TRANSIENT_SHAPE_SCORE_WEIGHT * 0.3);
    let stable_window_bonus = match state.target.nearest_category.as_str() {
        "stableWindow" => -40.0,
        "nearStableWindow" => -15.0,
        "acceleratingTrendWindow" => -8.0,
        _ => 0.0,
    };
    state.target.distance * 45.0
        + target_speed_error * 220.0
        + macro_speed_error * 120.0
        + late_two_gt_error * 180.0
        + state.trend.speed_error_recent * 140.0
        + state.trend.two_gt_error * 40.0
        + transient_penalty
        + state.early_score * 0.15
        + state.complexity * 0.5
        - convergence_bonus * 160.0
        - late_improvement_bonus * 120.0
        + stable_window_bonus
}

fn unique_prefix_count(
    candidates: &[SelectedCandidate],
    catalog: &CandidateCatalog,
) -> usize {
    candidates
        .iter()
        .map(|candidate| catalog.prefix(candidate).signature.clone())
        .collect::<BTreeSet<_>>()
        .len()
}

fn render_summary(
    args: &CliArgs,
    total_expanded: usize,
    target_windows: &BTreeMap<String, TargetWindowSummary>,
    frontier_stats: &[Value],
    prune_stats: &BTreeMap<String, Value>,
    short_verified: &[VerificationRow],
    long_verified: &[VerificationRow],
) -> String {
    let target_lines = target_windows
        .values()
        .map(|window| {
            format!(
                "- `{}`: stable={}, near={}, accelerating={}",
                window.cycle,
                window.stable_window.count,
                window.near_stable_window.count,
                window.accelerating_trend_window.count
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prune_lines = prune_stats
        .iter()
        .map(|(reason, value)| format!("- `{reason}`: {}", value.get("count").and_then(Value::as_u64).unwrap_or(0)))
        .collect::<Vec<_>>()
        .join("\n");
    let frontier_table = frontier_stats
        .iter()
        .map(|row| {
            format!(
                "| {} | {} | {} | {} | {} |",
                row.get("layer").and_then(Value::as_u64).unwrap_or(0),
                row.get("attempted").and_then(Value::as_u64).unwrap_or(0),
                row.get("generated").and_then(Value::as_u64).unwrap_or(0),
                row.get("bucketCount").and_then(Value::as_u64).unwrap_or(0),
                row.get("kept").and_then(Value::as_u64).unwrap_or(0),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let best_long = long_verified
        .iter()
        .filter(|row| row.dwell.strict_hit_rate.unwrap_or(0.0) >= 0.9998)
        .collect::<Vec<_>>();
    [
        "# Reachability Candidate Generator 1.17.1".to_string(),
        String::new(),
        "This run was produced by the Rust compatibility generator path. Beam/frontier terms are retained only as artifact names; exact short/long verification still comes from the Rust model simulation.".to_string(),
        String::new(),
        "## Search Scale".to_string(),
        String::new(),
        format!("- maxPrefixCells: {}", args.max_prefix_cells),
        format!("- beamWidth: {}", args.beam_width),
        format!("- bucketKeep: {}", args.bucket_keep),
        format!("- actual expanded states: {total_expanded}"),
        format!("- candidate prefixes selected for short verification: {}", short_verified.len()),
        format!(
            "- long verification short-hit threshold: {:.4}%",
            args.min_short_hit_rate_for_long * 100.0
        ),
        format!("- mode bias among selected candidates: {}", mode_bias(short_verified)),
        String::new(),
        "## Target Windows".to_string(),
        String::new(),
        if target_lines.is_empty() {
            "No target windows found.".to_string()
        } else {
            target_lines
        },
        String::new(),
        "## Frontier".to_string(),
        String::new(),
        "| Layer | Attempted | Generated | Buckets | Kept |".to_string(),
        "|---:|---:|---:|---:|---:|".to_string(),
        frontier_table,
        String::new(),
        "## Pruning".to_string(),
        String::new(),
        if prune_lines.is_empty() {
            "- none".to_string()
        } else {
            prune_lines
        },
        String::new(),
        "## Short Verification".to_string(),
        String::new(),
        markdown_candidate_table(short_verified),
        String::new(),
        "## Long Verification".to_string(),
        String::new(),
        if long_verified.is_empty() {
            "No candidates met the short-pass threshold for long verification.".to_string()
        } else {
            markdown_candidate_table(long_verified)
        },
        String::new(),
        "## 99.98% Long Result".to_string(),
        String::new(),
        if let Some(best) = best_long.first() {
            format!(
                "Found {} long result(s) at or above 99.98%. Best: `{}` / `{}` with {:.4}%.",
                best_long.len(),
                best.prefix_label,
                best.cycle,
                best.dwell.strict_hit_rate.unwrap_or(0.0) * 100.0
            )
        } else {
            "No long-verified candidate reached 99.98% in this run.".to_string()
        },
        String::new(),
    ]
    .join("\n")
}

fn mode_bias(rows: &[VerificationRow]) -> String {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.mode.clone()).or_insert(0_usize) += 1;
    }
    counts
        .into_iter()
        .map(|(mode, count)| format!("{mode}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn markdown_candidate_table(rows: &[VerificationRow]) -> String {
    if rows.is_empty() {
        return "No candidates.".to_string();
    }
    let mut lines = vec![
        "| Rank | Mode | Cycle | Prefix | Ent/Tick | Len | Hit | Avg speed | Stable tick | Score |".to_string(),
        "|---:|---|---|---|---:|---:|---:|---:|---:|---:|".to_string(),
    ];
    for (index, row) in rows.iter().take(20).enumerate() {
        lines.push(format!(
            "| {} | {} | `{}` | `{}` | {}/{} | {} | {} | {} | {} | {} |",
            index + 1,
            row.mode,
            row.cycle,
            row.prefix_label,
            row.entity_id_mod4,
            row.initial_tick_mod4,
            row.prefix_length,
            format_percent(row.dwell.strict_hit_rate),
            format_float(Some(row.average_speed), 6),
            row.acceleration_ticks
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            format_float(Some(row.score), 2),
        ));
    }
    lines.join("\n")
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|number| format!("{:.4}%", number * 100.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_float(value: Option<f64>, digits: usize) -> String {
    value
        .map(|number| format!("{number:.digits$}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn per_mode_limits(total: usize, modes: usize) -> Vec<usize> {
    if modes == 0 {
        return Vec::new();
    }
    let base = total / modes;
    let remainder = total % modes;
    (0..modes)
        .map(|index| base + usize::from(index < remainder))
        .map(|value| value.max(1))
        .collect()
}

fn phase_distance(a: f64, b: f64) -> f64 {
    let diff = (fraction(a) - fraction(b)).abs();
    diff.min(1.0 - diff)
}

fn fraction(value: f64) -> f64 {
    let raw = value - value.floor();
    if raw < 0.0 { raw + 1.0 } else { raw }
}

fn timestamp_now_string() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("Failed to format timestamp: {error}"))
}

fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("unix epoch")
        .as_millis()
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create output dir {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to encode JSON {}: {error}", path.display()))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create output dir {}: {error}", parent.display()))?;
    }
    fs::write(path, text)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join("item-waterway-solver-tests")
            .join(format!("{}-{label}", unix_millis()))
    }

    #[test]
    fn parse_args_accepts_basic_compatible_shape() {
        let args = parse_args(&[
            "--out".to_string(),
            "artifacts/out".to_string(),
            "--run-name".to_string(),
            "smoke".to_string(),
            "--modes".to_string(),
            "launch-fast".to_string(),
            "--cycles".to_string(),
            "W3-I_D3-B".to_string(),
            "--entity-mods".to_string(),
            "0,1".to_string(),
            "--initial-tick-mods".to_string(),
            "0,1".to_string(),
        ])
        .expect("args should parse");
        assert_eq!(args.run_name, "smoke");
        assert_eq!(args.modes, vec!["launch-fast"]);
        assert_eq!(args.cycles, vec!["W3-I_D3-B"]);
        assert_eq!(args.entity_mods, vec![0, 1]);
        assert_eq!(args.initial_tick_mods, vec![0, 1]);
    }

    #[test]
    fn main_cli_writes_expected_artifacts() {
        let out = unique_temp_path("reachable-candidates");
        let argv = vec![
            "--out".to_string(),
            out.display().to_string(),
            "--run-name".to_string(),
            "smoke".to_string(),
            "--modes".to_string(),
            "launch-fast".to_string(),
            "--cycles".to_string(),
            "W3-I_D3-B".to_string(),
            "--max-prefix-cells".to_string(),
            "2".to_string(),
            "--short-ticks".to_string(),
            "60".to_string(),
            "--long-ticks".to_string(),
            "120".to_string(),
            "--short-pairs".to_string(),
            "10".to_string(),
            "--long-pairs".to_string(),
            "20".to_string(),
            "--target-phase-samples".to_string(),
            "3".to_string(),
            "--target-vxs".to_string(),
            "0.25,0.5".to_string(),
            "--top-candidates".to_string(),
            "4".to_string(),
            "--long-limit".to_string(),
            "1".to_string(),
            "--entity-mods".to_string(),
            "0,1".to_string(),
            "--initial-tick-mods".to_string(),
            "0,1".to_string(),
        ];
        main_cli(&argv).expect("smoke run should succeed");
        let run_dir = out.join("smoke");
        for name in [
            "generator-engine.json",
            "generator-config.json",
            "target-windows.json",
            "frontier-stats.json",
            "prune-stats.json",
            "candidate-prefixes.json",
            "short-verified.json",
            "long-verified.json",
            "summary.md",
        ] {
            assert!(run_dir.join(name).exists(), "{name} should exist");
        }
        let short_verified = fs::read_to_string(run_dir.join("short-verified.json"))
            .expect("read short-verified");
        assert!(short_verified.contains("\"mode\": \"launch-fast\""));
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn service_short_pairs_scales_with_requested_ticks() {
        assert_eq!(service_short_pairs(60), 25);
        assert_eq!(service_short_pairs(400), 195);
        assert_eq!(service_short_pairs(800), 395);
    }

    #[test]
    fn modules_for_prefix_parses_legacy_stab_names_as_single_modules() {
        let modules = modules_for_prefix("F5I-DI-F2I-stab-DB-stab-F3I-stab-DB-stab");
        let names = modules
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["F5I", "DI", "F2I-stab", "DB-stab", "F3I-stab", "DB-stab"]
        );
        let total_cost = modules.iter().map(|module| module.cost).sum::<f64>();
        assert!((total_cost - 18.2).abs() < 1.0e-9);
    }

    #[test]
    fn physical_prefix_label_collapses_stab_aliases() {
        assert_eq!(
            physical_prefix_label("R2N-DB-DB-F2I-stab-DB-stab"),
            physical_prefix_label("R2N-D2B-F2I-DB")
        );
        assert_eq!(
            physical_prefix_label("F5I-DI-F2I-stab-DB-stab-F3I-stab-DB-stab"),
            "F8-I-F7-I-F6-I-F5-I-F4-I-D-I-F8-I-F7-I-D-B-F8-I-F7-I-F6-I-D-B"
        );
    }

    #[test]
    fn live_prefix_atoms_include_minimum_historical_bridge_atoms() {
        let live_atoms = crate::prefix_atoms()
            .into_iter()
            .map(|atom| atom.name)
            .collect::<Vec<_>>();
        assert!(live_atoms.contains(&"F2I"));
        assert!(live_atoms.contains(&"D2B"));
        assert!(live_atoms.contains(&"F3I"));
        assert!(live_atoms.contains(&"F5I"));
    }
}
