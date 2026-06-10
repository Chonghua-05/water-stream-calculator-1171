use crate::{
    backbone_cycles, best_early_cadence, full_cadence_metrics, suffix_long_metrics, simulate,
    window_metric_context, window_metrics, Cell, CellDescription, EarlyCadence,
    EarlyCadenceSample, FirstTick, FullCadence, FullCadenceSample, Layout, PrefixAtom, SimConfig,
    Simulation, WindowMetrics,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TICKS: usize = 6004;
const DEFAULT_START_X: f64 = -0.365;
const DEFAULT_START_Y: f64 = 0.0;
const DEFAULT_START_VX: f64 = 1.0;
const DEFAULT_START_VY: f64 = 0.0;
const DEFAULT_START_ON_GROUND: bool = false;
const DEFAULT_CADENCE_PAIRS: usize = 10;
const DEFAULT_FULL_CADENCE_PAIRS: usize = 3000;
const DEFAULT_TOLERANCE: f64 = 0.1;
const DEFAULT_LONG_WINDOW: usize = 200;
const DEFAULT_FIRST_TICKS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize)]
struct CandidateEvalArgs {
    prefix: String,
    backbone: String,
    out: PathBuf,
    ticks: usize,
    #[serde(rename = "startX")]
    start_x: f64,
    #[serde(rename = "startY")]
    start_y: f64,
    #[serde(rename = "startVX")]
    start_vx: f64,
    #[serde(rename = "startVY")]
    start_vy: f64,
    #[serde(rename = "startOnGround")]
    start_on_ground: bool,
    #[serde(rename = "cadencePairs")]
    cadence_pairs: usize,
    #[serde(rename = "fullCadencePairs")]
    full_cadence_pairs: usize,
    tolerance: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateEvalPayload {
    args: CandidateEvalArgs,
    prefix_label: String,
    backbone: String,
    prefix_cells: Vec<CellDescription>,
    cycle_cells: Vec<CellDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    early: Option<EarlyCadenceOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full: Option<FullCadenceOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    long: Option<WindowMetricsOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suffix: Option<WindowMetricsOutput>,
    first_ticks: Vec<FirstTick>,
}

#[derive(Clone, Debug, Serialize)]
struct CandidateEvalStdoutSummary {
    out: String,
    early: Option<EarlyCadenceOutput>,
    full: CandidateEvalStdoutFullSummary,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateEvalStdoutFullSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pairs: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_err: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_miss: Option<EarlyCadenceSample>,
}

#[derive(Clone, Debug, Serialize)]
struct EarlyCadenceOutput {
    #[serde(rename = "cadenceStartTick")]
    cadence_start_tick: usize,
    #[serde(rename = "cadencePairs")]
    cadence_pairs: usize,
    #[serde(rename = "cadenceMeanAbsDistanceError")]
    cadence_mean_abs_distance_error: f64,
    #[serde(rename = "cadenceMeanSignedDistanceError")]
    cadence_mean_signed_distance_error: f64,
    #[serde(rename = "cadenceMaxAbsDistanceError")]
    cadence_max_abs_distance_error: f64,
    #[serde(rename = "cadenceBlockHitRate")]
    cadence_block_hit_rate: f64,
    #[serde(rename = "cadenceWithinToleranceRate")]
    cadence_within_tolerance_rate: f64,
    #[serde(rename = "cadencePass")]
    cadence_pass: bool,
    #[serde(rename = "cadenceSamples")]
    cadence_samples: Vec<EarlyCadenceSample>,
    #[serde(rename = "earlyCadenceScore")]
    early_cadence_score: f64,
}

#[derive(Clone, Debug, Serialize)]
struct FullCadenceOutput {
    #[serde(rename = "fullCadenceStartTick")]
    full_cadence_start_tick: usize,
    #[serde(rename = "fullCadencePairs")]
    full_cadence_pairs: usize,
    #[serde(rename = "fullCadenceMeanAbsDistanceError")]
    full_cadence_mean_abs_distance_error: f64,
    #[serde(rename = "fullCadenceMeanSignedDistanceError")]
    full_cadence_mean_signed_distance_error: f64,
    #[serde(rename = "fullCadenceMaxAbsDistanceError")]
    full_cadence_max_abs_distance_error: f64,
    #[serde(rename = "fullCadenceBlockHitRate")]
    full_cadence_block_hit_rate: f64,
    #[serde(rename = "fullCadenceWithinToleranceRate")]
    full_cadence_within_tolerance_rate: f64,
    #[serde(rename = "fullCadenceLongestHitRun")]
    full_cadence_longest_hit_run: usize,
    #[serde(rename = "fullCadenceFirstMiss", skip_serializing_if = "Option::is_none")]
    full_cadence_first_miss: Option<EarlyCadenceSample>,
    #[serde(rename = "fullCadenceMinHitMargin")]
    full_cadence_min_hit_margin: f64,
    #[serde(rename = "fullCadenceMeanHitMargin")]
    full_cadence_mean_hit_margin: f64,
    #[serde(rename = "fullCadenceMinEndpointBoundaryMargin")]
    full_cadence_min_endpoint_boundary_margin: f64,
    #[serde(rename = "fullCadenceMeanEndpointBoundaryMargin")]
    full_cadence_mean_endpoint_boundary_margin: f64,
    #[serde(rename = "fullCadenceSamples")]
    full_cadence_samples: Vec<FullCadenceSample>,
    #[serde(rename = "fullCadenceDistance")]
    full_cadence_distance: f64,
    #[serde(rename = "fullCadenceAverageSpeed")]
    full_cadence_average_speed: f64,
}

#[derive(Clone, Debug, Serialize)]
struct WindowMetricsOutput {
    #[serde(rename = "averageVX")]
    average_vx: f64,
    #[serde(rename = "meanVXError")]
    mean_vx_error: f64,
    #[serde(rename = "stdVX")]
    std_vx: f64,
    #[serde(rename = "averageDistanceVX")]
    average_distance_vx: f64,
    #[serde(
        rename = "longWindowScore",
        skip_serializing_if = "Option::is_none"
    )]
    long_window_score: Option<f64>,
    #[serde(
        rename = "longWindowStartTick",
        skip_serializing_if = "Option::is_none"
    )]
    long_window_start_tick: Option<usize>,
    #[serde(rename = "suffixStartTick", skip_serializing_if = "Option::is_none")]
    suffix_start_tick: Option<usize>,
}

pub(super) fn usage() -> String {
    "Usage: item-waterway-solver candidate-eval --prefix <label> --backbone <name> [--out <path>] [--ticks <n>] [--start-x <x>] [--start-on-ground <true|false>]".to_string()
}

pub(super) fn main_cli(argv: &[String]) -> Result<(), String> {
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return Ok(());
    }
    let args = parse_args(argv)?;
    let payload = evaluate_candidate(&args)?;
    write_payload(&args.out, &payload)?;
    let stdout_summary = stdout_summary(&payload);
    println!(
        "{}",
        serde_json::to_string_pretty(&stdout_summary)
            .map_err(|error| format!("Failed to encode candidate-eval stdout JSON: {error}"))?
    );
    Ok(())
}

fn parse_args(argv: &[String]) -> Result<CandidateEvalArgs, String> {
    let mut args = CandidateEvalArgs {
        prefix: String::new(),
        backbone: String::new(),
        out: PathBuf::from("artifacts")
            .join("item-waterway-launch-search")
            .join("candidate-eval.json"),
        ticks: DEFAULT_TICKS,
        start_x: DEFAULT_START_X,
        start_y: DEFAULT_START_Y,
        start_vx: DEFAULT_START_VX,
        start_vy: DEFAULT_START_VY,
        start_on_ground: DEFAULT_START_ON_GROUND,
        cadence_pairs: DEFAULT_CADENCE_PAIRS,
        full_cadence_pairs: DEFAULT_FULL_CADENCE_PAIRS,
        tolerance: DEFAULT_TOLERANCE,
    };

    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        match arg.as_str() {
            "--prefix" => {
                index += 1;
                args.prefix = next_value(argv, index, "--prefix")?.to_string();
            }
            "--backbone" => {
                index += 1;
                args.backbone = next_value(argv, index, "--backbone")?.to_string();
            }
            "--out" => {
                index += 1;
                args.out = PathBuf::from(next_value(argv, index, "--out")?);
            }
            "--ticks" => {
                index += 1;
                args.ticks = next_value(argv, index, "--ticks")?
                    .parse::<usize>()
                    .map_err(|error| format!("Invalid --ticks value: {error}"))?;
            }
            "--start-x" => {
                index += 1;
                args.start_x = next_value(argv, index, "--start-x")?
                    .parse::<f64>()
                    .map_err(|error| format!("Invalid --start-x value: {error}"))?;
            }
            "--start-on-ground" => {
                index += 1;
                args.start_on_ground = next_value(argv, index, "--start-on-ground")?
                    .eq_ignore_ascii_case("true");
            }
            "--help" | "-h" => return Err(usage()),
            _ => return Err(format!("Unknown arg {arg}")),
        }
        index += 1;
    }

    if args.prefix.is_empty() || args.backbone.is_empty() {
        return Err("--prefix and --backbone are required.".to_string());
    }
    Ok(args)
}

fn next_value<'a>(argv: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    argv.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value."))
}

fn evaluate_candidate(args: &CandidateEvalArgs) -> Result<CandidateEvalPayload, String> {
    let cycle = backbone_cycles()
        .into_iter()
        .find(|cycle| cycle.name == args.backbone)
        .ok_or_else(|| format!("Unknown backbone {}", args.backbone))?;
    let atoms = atoms_from_label(&args.prefix)?;
    let prefix_label = prefix_label(&atoms);
    let prefix_cells = prefix_cells(&atoms);
    let layout = Layout::new(&prefix_cells, &cycle.cells);
    let sim = simulate(
        &layout,
        &SimConfig {
            ticks: args.ticks,
            start_x: args.start_x,
            start_y: args.start_y,
            start_vx: args.start_vx,
            start_vy: args.start_vy,
            entity_id_mod4: 0,
            initial_tick_count: 0,
            start_on_ground: Some(args.start_on_ground),
        },
    );

    let early = best_early_cadence(&sim, 5, args.cadence_pairs, args.tolerance);
    let full_start = early
        .as_ref()
        .map(|metrics| metrics.cadence_start_tick)
        .unwrap_or(1);
    let full = full_cadence_metrics(&sim, full_start, args.full_cadence_pairs, args.tolerance);
    let context = window_metric_context(&sim);
    let long = window_metrics(&sim, full_start, DEFAULT_LONG_WINDOW, &context);
    let suffix = suffix_long_metrics(&sim, full_start, DEFAULT_LONG_WINDOW);

    Ok(CandidateEvalPayload {
        args: args.clone(),
        prefix_label,
        backbone: cycle.name.clone(),
        prefix_cells: describe_cells(&prefix_cells),
        cycle_cells: describe_cells(&cycle.cells),
        early: early.as_ref().map(early_output),
        full: full.as_ref().map(full_output),
        long: long.as_ref().map(window_output),
        suffix: suffix.as_ref().map(window_output),
        first_ticks: first_ticks(&sim),
    })
}

fn atoms_from_label(label: &str) -> Result<Vec<PrefixAtom>, String> {
    crate::parse_prefix_atoms_from_label(label)
}

fn prefix_label(atoms: &[PrefixAtom]) -> String {
    crate::format_prefix_label(atoms)
}

fn prefix_cells(atoms: &[PrefixAtom]) -> Vec<Cell> {
    let total = atoms.iter().map(|atom| atom.cells.len()).sum();
    let mut cells = Vec::with_capacity(total);
    for atom in atoms {
        cells.extend_from_slice(&atom.cells);
    }
    cells
}

fn describe_cells(cells: &[Cell]) -> Vec<CellDescription> {
    cells.iter()
        .enumerate()
        .map(|(index, cell)| CellDescription {
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

fn first_ticks(sim: &Simulation) -> Vec<FirstTick> {
    (0..sim.xs.len().min(DEFAULT_FIRST_TICKS))
        .map(|tick| FirstTick {
            tick,
            x: sim.xs[tick],
            y: sim.ys[tick],
            vx: sim.vxs[tick],
            vy: sim.vys[tick],
            floor: sim.floors[tick].as_str().to_string(),
            on_ground: sim.on_grounds[tick] != 0,
        })
        .collect()
}

fn early_output(metrics: &EarlyCadence) -> EarlyCadenceOutput {
    EarlyCadenceOutput {
        cadence_start_tick: metrics.cadence_start_tick,
        cadence_pairs: metrics.cadence_pairs,
        cadence_mean_abs_distance_error: metrics.cadence_mean_abs_distance_error,
        cadence_mean_signed_distance_error: metrics.cadence_mean_signed_distance_error,
        cadence_max_abs_distance_error: metrics.cadence_max_abs_distance_error,
        cadence_block_hit_rate: metrics.cadence_block_hit_rate,
        cadence_within_tolerance_rate: metrics.cadence_within_tolerance_rate,
        cadence_pass: metrics.cadence_pass,
        cadence_samples: metrics.cadence_samples.clone(),
        early_cadence_score: metrics.early_cadence_score,
    }
}

fn full_output(metrics: &FullCadence) -> FullCadenceOutput {
    FullCadenceOutput {
        full_cadence_start_tick: metrics.full_cadence_start_tick,
        full_cadence_pairs: metrics.full_cadence_pairs,
        full_cadence_mean_abs_distance_error: metrics.full_cadence_mean_abs_distance_error,
        full_cadence_mean_signed_distance_error: metrics.full_cadence_mean_signed_distance_error,
        full_cadence_max_abs_distance_error: metrics.full_cadence_max_abs_distance_error,
        full_cadence_block_hit_rate: metrics.full_cadence_block_hit_rate,
        full_cadence_within_tolerance_rate: metrics.full_cadence_within_tolerance_rate,
        full_cadence_longest_hit_run: metrics.full_cadence_longest_hit_run,
        full_cadence_first_miss: metrics.full_cadence_first_miss.clone(),
        full_cadence_min_hit_margin: metrics.full_cadence_min_hit_margin,
        full_cadence_mean_hit_margin: metrics.full_cadence_mean_hit_margin,
        full_cadence_min_endpoint_boundary_margin: metrics.full_cadence_min_endpoint_boundary_margin,
        full_cadence_mean_endpoint_boundary_margin: metrics
            .full_cadence_mean_endpoint_boundary_margin,
        full_cadence_samples: metrics.full_cadence_samples.clone(),
        full_cadence_distance: metrics.full_cadence_distance,
        full_cadence_average_speed: metrics.full_cadence_average_speed,
    }
}

fn window_output(metrics: &WindowMetrics) -> WindowMetricsOutput {
    WindowMetricsOutput {
        average_vx: metrics.average_vx,
        mean_vx_error: metrics.mean_vx_error,
        std_vx: metrics.std_vx,
        average_distance_vx: metrics.average_distance_vx,
        long_window_score: metrics.long_window_score,
        long_window_start_tick: metrics.long_window_start_tick,
        suffix_start_tick: metrics.suffix_start_tick,
    }
}

fn stdout_summary(payload: &CandidateEvalPayload) -> CandidateEvalStdoutSummary {
    let full = payload
        .full
        .as_ref()
        .map(|metrics| CandidateEvalStdoutFullSummary {
            start: Some(metrics.full_cadence_start_tick),
            pairs: Some(metrics.full_cadence_pairs),
            hit_rate: Some(metrics.full_cadence_block_hit_rate),
            avg: Some(metrics.full_cadence_average_speed),
            min_margin: Some(metrics.full_cadence_min_hit_margin),
            max_err: Some(metrics.full_cadence_max_abs_distance_error),
            first_miss: metrics.full_cadence_first_miss.clone(),
        })
        .unwrap_or_default();
    CandidateEvalStdoutSummary {
        out: payload.args.out.display().to_string(),
        early: payload.early.clone(),
        full,
    }
}

fn write_payload(path: &Path, payload: &CandidateEvalPayload) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create output dir {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("Failed to encode candidate-eval JSON: {error}"))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("Failed to write candidate-eval JSON {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(file_name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join("item-waterway-solver-tests")
            .join(format!("{stamp}-{file_name}"))
    }

    #[test]
    fn parse_args_requires_prefix_and_backbone() {
        let error = parse_args(&[]).expect_err("missing args should fail");
        assert!(error.contains("--prefix and --backbone are required"));
    }

    #[test]
    fn parse_args_accepts_expected_cli_shape() {
        let argv = vec![
            "--prefix".to_string(),
            "F2I".to_string(),
            "--backbone".to_string(),
            "W3-I_D3-B".to_string(),
            "--out".to_string(),
            "out.json".to_string(),
            "--ticks".to_string(),
            "123".to_string(),
            "--start-x".to_string(),
            "-0.25".to_string(),
            "--start-on-ground".to_string(),
            "true".to_string(),
        ];
        let parsed = parse_args(&argv).expect("args should parse");
        assert_eq!(parsed.prefix, "F2I");
        assert_eq!(parsed.backbone, "W3-I_D3-B");
        assert_eq!(parsed.out, PathBuf::from("out.json"));
        assert_eq!(parsed.ticks, 123);
        assert_eq!(parsed.start_x, -0.25);
        assert!(parsed.start_on_ground);
        assert_eq!(parsed.start_vx, DEFAULT_START_VX);
    }

    #[test]
    fn evaluate_candidate_smoke_produces_json_payload() {
        let path = unique_temp_path("candidate-eval.json");
        let args = CandidateEvalArgs {
            prefix: "F2I".to_string(),
            backbone: "W3-I_D3-B".to_string(),
            out: path.clone(),
            ticks: 24,
            start_x: DEFAULT_START_X,
            start_y: DEFAULT_START_Y,
            start_vx: DEFAULT_START_VX,
            start_vy: DEFAULT_START_VY,
            start_on_ground: DEFAULT_START_ON_GROUND,
            cadence_pairs: DEFAULT_CADENCE_PAIRS,
            full_cadence_pairs: DEFAULT_FULL_CADENCE_PAIRS,
            tolerance: DEFAULT_TOLERANCE,
        };
        let payload = evaluate_candidate(&args).expect("candidate eval should succeed");
        assert_eq!(payload.prefix_label, "F2I");
        assert_eq!(payload.backbone, "W3-I_D3-B");
        assert!(!payload.prefix_cells.is_empty());
        assert!(!payload.cycle_cells.is_empty());
        assert!(!payload.first_ticks.is_empty());
        write_payload(&path, &payload).expect("write payload");
        let written = fs::read_to_string(&path).expect("read payload");
        assert!(written.contains("\"prefixLabel\": \"F2I\""));
        assert!(written.contains("\"backbone\": \"W3-I_D3-B\""));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn evaluate_candidate_accepts_legacy_stab_prefix_and_proven_backbone() {
        let args = CandidateEvalArgs {
            prefix: "F5I-DI-F2I-stab-DB-stab-F3I-stab-DB-stab".to_string(),
            backbone: "D1-I_F2-I_D1-B_S1-B_F2-I_D1-B_S1-B".to_string(),
            out: unique_temp_path("legacy-candidate-eval.json"),
            ticks: 48,
            start_x: 0.635,
            start_y: 0.0005000000237487257,
            start_vx: 1.0,
            start_vy: 0.0004900000232737512,
            start_on_ground: false,
            cadence_pairs: 8,
            full_cadence_pairs: 12,
            tolerance: DEFAULT_TOLERANCE,
        };
        let payload = evaluate_candidate(&args).expect("legacy candidate eval should succeed");
        assert_eq!(
            payload.prefix_label,
            "F5I-DI-F2I-stab-DB-stab-F3I-stab-DB-stab"
        );
        assert_eq!(
            payload.backbone,
            "D1-I_F2-I_D1-B_S1-B_F2-I_D1-B_S1-B"
        );
        assert_eq!(payload.prefix_cells.len(), 13);
        assert_eq!(payload.cycle_cells.len(), 9);
    }
}
