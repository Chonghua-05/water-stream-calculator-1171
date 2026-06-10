//! Pure viewer-run business logic for the Rust backend.
//!
//! This module intentionally stops at structure/simulation/metrics/run assembly.
//! It does not own HTTP routing, run persistence, or search-task lifecycle.
//!
//! When piston launch mode is active, callers are expected to pass a structure
//! whose launch metadata has already been preprocessed into the structure's
//! extra `launch` object (`timelineSamples`, `timelineOffsetGt`,
//! `effectiveStart`, and related fields). This module consumes that data but
//! does not synthesize it.

use crate::schema::{FloorName, Structure, ViewerPoint, ViewerRun, ViewerRunSummary};
use crate::{floor_at, parse_floor, simulate, Layout, SimConfig, Simulation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MODEL_ENGINE: &str = "item-waterway-solver-rust";
const COMPARE_EPSILON: f64 = 1.0e-12;

pub type ViewerRunOptions = BTreeMap<String, Value>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationSeries {
    pub engine: String,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub vxs: Vec<f64>,
    pub vys: Vec<f64>,
    pub on_grounds: Vec<bool>,
    pub floors: Vec<FloorName>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewerRunMetrics {
    pub target_speed: f64,
    pub target_dwell_ticks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dwell_min_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dwell_max_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dwell_min_start_tick: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_detection_min_block: Option<i64>,
    pub dwell_include_final_group: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_start_tick: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_start_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_start_raw_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_end_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_end_raw_block: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_detect_block_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_detect_average_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_detect_mean_abs_distance_error: Option<f64>,
    pub steady_sample_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_avg_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_per_block_target_dwell_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_per_block_two_gt_dwell_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_block_target_dwell_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_block_two_gt_dwell_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_per_block_two_gt_dwell_hit_rate: Option<f64>,
    pub dwell_blocks: usize,
    pub dwell_failures: usize,
    pub steady_dwell_blocks: usize,
    pub steady_dwell_failures: usize,
    pub steady_tail_inferred_hits: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_avg_derived_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_derived_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_speed_x_gt_3: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_error: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompareStateSample {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompareFirstDifference {
    pub tick: usize,
    pub dx: f64,
    pub dy: f64,
    pub dvx: f64,
    pub dvy: f64,
    pub reference: CompareStateSample,
    pub candidate: CompareStateSample,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompareFinalState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_vx: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCompareResult {
    pub ok: bool,
    pub ticks: usize,
    pub sample_count: usize,
    pub reference_engine: String,
    pub candidate_engine: String,
    pub max_abs_dx: f64,
    pub max_abs_dy: f64,
    pub max_abs_dvx: f64,
    pub max_abs_dvy: f64,
    pub final_state: CompareFinalState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_difference: Option<CompareFirstDifference>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SteadyWindowDetection {
    pub tick: usize,
    pub raw_block: i64,
    pub display_block: i64,
    pub block_hit_rate: f64,
    pub mean_abs_distance_error: f64,
    pub average_speed: f64,
}

#[derive(Clone, Debug)]
struct DwellGroup {
    block_x: i64,
    points: Vec<ViewerPoint>,
}

#[derive(Clone, Debug)]
struct CadenceMetrics {
    pairs: usize,
    block_hit_rate: f64,
    mean_abs_distance_error: f64,
    average_speed: f64,
}

pub fn simulation_ticks_for_requested_duration(structure: &Structure, ticks: usize) -> usize {
    ticks.saturating_sub(launch_timeline_offset(structure))
}

pub fn simulate_structure(structure: &Structure, ticks: usize) -> Result<SimulationSeries, String> {
    let layout = structure_to_layout(structure)?;
    let config = SimConfig {
        ticks,
        start_x: structure.start.x,
        start_y: structure.start.y,
        start_vx: structure.start.vx,
        start_vy: structure.start.vy,
        entity_id_mod4: structure.start.entity_id_mod4,
        initial_tick_count: structure.start.initial_tick_count,
        start_on_ground: structure.start.start_on_ground,
    };
    Ok(simulation_series_from_simulation(simulate(&layout, &config)))
}

pub fn simulate_structure_for_requested_duration(
    structure: &Structure,
    ticks: usize,
) -> Result<SimulationSeries, String> {
    simulate_structure(structure, simulation_ticks_for_requested_duration(structure, ticks))
}

pub fn simulate_viewer_points_for_requested_duration(
    structure: &Structure,
    ticks: usize,
) -> Result<Vec<ViewerPoint>, String> {
    let sim = simulate_structure_for_requested_duration(structure, ticks)?;
    simulation_to_viewer_points(structure, &sim)
}

pub fn simulation_to_viewer_points(
    structure: &Structure,
    sim: &SimulationSeries,
) -> Result<Vec<ViewerPoint>, String> {
    let stitched = stitch_launch_timeline(structure, sim)?;
    let origin_x = launch_value(structure)
        .and_then(|launch| launch.get("displayOriginX"))
        .and_then(as_f64)
        .unwrap_or(structure.origin_x);
    let start_x_raw = origin_x + stitched.xs.first().copied().unwrap_or(0.0);
    let mut previous_x = None;
    let mut points = Vec::with_capacity(stitched.xs.len());

    for tick in 0..stitched.xs.len() {
        let x_raw = origin_x + stitched.xs[tick];
        let derived = previous_x.map(|value| x_raw - value);
        points.push(ViewerPoint {
            tick_index: tick,
            x: Some(display_x_from_raw(x_raw, start_x_raw)),
            x_raw: Some(x_raw),
            speed: stitched.vxs.get(tick).copied(),
            derived_speed: derived,
            y: stitched.ys.get(tick).copied(),
            vy: stitched.vys.get(tick).copied(),
            floor: stitched.floors.get(tick).cloned(),
            on_ground: stitched.on_grounds.get(tick).copied(),
            log_time: None,
            captured_at: None,
            raw_line: None,
            extra: BTreeMap::new(),
        });
        previous_x = Some(x_raw);
    }

    Ok(points)
}

pub fn metrics_for_points(points: &[ViewerPoint], options: &ViewerRunOptions) -> ViewerRunMetrics {
    let target_speed = option_f64(options, "targetSpeed").unwrap_or(0.5);
    let target_dwell = option_usize(options, "targetDwellTicks")
        .unwrap_or_else(|| default_target_dwell_ticks(target_speed));
    let explicit_min_block = option_i64(options, "minBlock");
    let explicit_max_block = option_i64(options, "maxBlock");
    let explicit_min_start_tick = option_usize(options, "minStartTick");
    let steady_min_block = option_i64(options, "steadyMinBlock");
    let has_explicit_window =
        explicit_min_block.is_some() || explicit_max_block.is_some() || explicit_min_start_tick.is_some();
    let detected_steady = detect_steady_window(points, options);

    let mut min_start_tick = explicit_min_start_tick;
    let mut min_block = explicit_min_block;
    let mut max_block = explicit_max_block;
    if !has_explicit_window {
        if let Some(detected) = &detected_steady {
            min_start_tick = Some(detected.tick);
            min_block = Some(
                detected
                    .raw_block
                    .max(raw_block_at(&points[detected.tick]).unwrap_or(detected.raw_block)),
            );
            max_block = points.last().and_then(raw_block_at);
        }
    }

    let has_dwell_window = has_explicit_window || detected_steady.is_some();
    let include_final_group = option_bool(options, "includeFinalGroup", true);
    let mut groups = block_dwell_groups(points);
    if !include_final_group && groups.last().is_some_and(|group| group.points.len() != target_dwell) {
        groups.pop();
    }

    let eligible = if has_dwell_window {
        filter_dwell_groups(&groups, min_start_tick, min_block, max_block)
    } else {
        Vec::new()
    };

    let steady_start_tick = detected_steady.as_ref().map(|detected| detected.tick);
    let steady_start_raw_block = detected_steady.as_ref().map(|detected| detected.raw_block);
    let steady_start_block = detected_steady
        .as_ref()
        .map(|detected| detected.display_block);
    let steady_end_raw_block = if detected_steady.is_some() {
        points
            .last()
            .and_then(raw_block_at)
            .or_else(|| eligible.last().map(|group| group.block_x))
    } else {
        None
    };
    let steady_end_block = if detected_steady.is_some() {
        points
            .last()
            .and_then(display_block_at)
            .or_else(|| {
                eligible
                    .last()
                    .and_then(|group| group.points.last())
                    .and_then(display_block_at)
            })
    } else {
        None
    };

    let steady_groups = if detected_steady.is_some() {
        filter_dwell_groups(
            &groups,
            steady_start_tick,
            steady_start_raw_block,
            steady_end_raw_block,
        )
    } else {
        Vec::new()
    };

    let (hits, scored_count, _tail_inferred_hits) = score_dwell_groups(&eligible, target_dwell);
    let (steady_hits, steady_scored_count, steady_tail_inferred_hits) =
        score_dwell_groups(&steady_groups, target_dwell);
    let target_hit_rate = ratio(hits, scored_count);
    let steady_target_hit_rate = ratio(steady_hits, steady_scored_count);
    let (two_gt_hits, two_gt_scored_count, _) = score_dwell_groups(&eligible, 2);
    let (steady_two_gt_hits, steady_two_gt_scored_count, _) =
        score_dwell_groups(&steady_groups, 2);
    let two_gt_hit_rate = ratio(two_gt_hits, two_gt_scored_count);
    let steady_two_gt_hit_rate = ratio(steady_two_gt_hits, steady_two_gt_scored_count);

    let point_in_steady_window = |point: &ViewerPoint| {
        if let Some(min_tick) = steady_start_tick {
            if point.tick_index < min_tick {
                return false;
            }
        }
        let block = raw_block_at(point).unwrap_or(0);
        if let Some(min_block) = steady_start_raw_block {
            if block < min_block {
                return false;
            }
        }
        if let Some(max_block) = steady_end_raw_block {
            if block > max_block {
                return false;
            }
        }
        true
    };

    let overall_speeds = points
        .iter()
        .filter_map(|point| point.derived_speed)
        .collect::<Vec<_>>();
    let steady_points = points
        .iter()
        .filter(|point| point.derived_speed.is_some() && point_in_steady_window(point))
        .collect::<Vec<_>>();
    let steady_speeds = steady_points
        .iter()
        .filter_map(|point| point.derived_speed)
        .collect::<Vec<_>>();
    let overall_avg_speed = average(&overall_speeds);
    let steady_avg_speed = average(&steady_speeds);

    ViewerRunMetrics {
        target_speed,
        target_dwell_ticks: target_dwell,
        dwell_min_block: min_block,
        dwell_max_block: max_block,
        dwell_min_start_tick: min_start_tick,
        steady_detection_min_block: steady_min_block,
        dwell_include_final_group: include_final_group,
        steady_start_tick,
        steady_start_block,
        steady_start_raw_block,
        steady_end_block,
        steady_end_raw_block,
        steady_source: detected_steady
            .as_ref()
            .map(|_| "detected".to_string()),
        steady_detect_block_hit_rate: detected_steady.as_ref().map(|window| window.block_hit_rate),
        steady_detect_average_speed: detected_steady.as_ref().map(|window| window.average_speed),
        steady_detect_mean_abs_distance_error: detected_steady
            .as_ref()
            .map(|window| window.mean_abs_distance_error),
        steady_sample_count: steady_points.len(),
        steady_avg_speed,
        steady_per_block_target_dwell_hit_rate: steady_target_hit_rate,
        steady_per_block_two_gt_dwell_hit_rate: steady_two_gt_hit_rate,
        per_block_target_dwell_hit_rate: target_hit_rate,
        per_block_two_gt_dwell_hit_rate: two_gt_hit_rate,
        long_per_block_two_gt_dwell_hit_rate: two_gt_hit_rate,
        dwell_blocks: scored_count,
        dwell_failures: scored_count.saturating_sub(hits),
        steady_dwell_blocks: steady_scored_count,
        steady_dwell_failures: steady_scored_count.saturating_sub(steady_hits),
        steady_tail_inferred_hits: steady_tail_inferred_hits,
        overall_avg_derived_speed: overall_avg_speed,
        avg_derived_speed: steady_avg_speed,
        avg_speed_x_gt_3: steady_avg_speed,
        speed_error: steady_avg_speed.map(|speed| speed - target_speed),
    }
}

pub fn make_run(
    structure: &Structure,
    sim: &SimulationSeries,
    options: &ViewerRunOptions,
    label: &str,
) -> Result<ViewerRun, String> {
    let points = simulation_to_viewer_points(structure, sim)?;
    let metrics = metrics_for_points(&points, options);
    let mut summary = ViewerRunSummary {
        source: Some("simulation".to_string()),
        model_engine: Some(sim.engine.clone()),
        structure: Some(
            structure
                .name
                .clone()
                .unwrap_or_else(|| label.to_string()),
        ),
        structure_count: Some(option_usize(options, "structureCount").unwrap_or(1)),
        deleted: false,
        launch_mode: launch_mode(structure),
        equivalent_fingerprint: None,
        extra: BTreeMap::new(),
    };
    insert_summary_metric(&mut summary.extra, "sample_count", json!(points.len()));
    insert_summary_metric(
        &mut summary.extra,
        "start_x_raw",
        json!(points.first().and_then(|point| point.x_raw)),
    );
    insert_summary_metric(
        &mut summary.extra,
        "end_x_raw",
        json!(points.last().and_then(|point| point.x_raw)),
    );
    insert_summary_metric(
        &mut summary.extra,
        "start_x",
        json!(points.first().and_then(|point| point.x)),
    );
    insert_summary_metric(
        &mut summary.extra,
        "end_x",
        json!(points.last().and_then(|point| point.x)),
    );
    insert_summary_metric(
        &mut summary.extra,
        "duration_gt",
        json!(points.len().saturating_sub(1)),
    );
    extend_summary_metrics(&mut summary.extra, &metrics)?;
    extend_launch_summary(structure, &mut summary.extra);

    Ok(ViewerRun {
        run_id: None,
        label: Some(label.to_string()),
        display_label: Some(label.to_string()),
        summary,
        points,
        structure: Some(structure.clone()),
        extra: BTreeMap::new(),
    })
}

pub fn simulate_run(
    structure: &Structure,
    ticks: usize,
    options: &ViewerRunOptions,
    label: &str,
) -> Result<ViewerRun, String> {
    let sim = simulate_structure_for_requested_duration(structure, ticks)?;
    make_run(structure, &sim, options, label)
}

pub fn compare_simulations(
    reference: &SimulationSeries,
    candidate: &SimulationSeries,
    ticks: usize,
) -> ModelCompareResult {
    let sample_count = reference
        .xs
        .len()
        .min(candidate.xs.len())
        .min(reference.ys.len())
        .min(candidate.ys.len())
        .min(reference.vxs.len())
        .min(candidate.vxs.len())
        .min(reference.vys.len())
        .min(candidate.vys.len());

    let max_abs_dx = max_delta(&reference.xs, &candidate.xs, sample_count);
    let max_abs_dy = max_delta(&reference.ys, &candidate.ys, sample_count);
    let max_abs_dvx = max_delta(&reference.vxs, &candidate.vxs, sample_count);
    let max_abs_dvy = max_delta(&reference.vys, &candidate.vys, sample_count);

    let mut first_difference = None;
    for tick in 0..sample_count {
        let dx = (reference.xs[tick] - candidate.xs[tick]).abs();
        let dy = (reference.ys[tick] - candidate.ys[tick]).abs();
        let dvx = (reference.vxs[tick] - candidate.vxs[tick]).abs();
        let dvy = (reference.vys[tick] - candidate.vys[tick]).abs();
        if dx.max(dy).max(dvx).max(dvy) > COMPARE_EPSILON {
            first_difference = Some(CompareFirstDifference {
                tick,
                dx,
                dy,
                dvx,
                dvy,
                reference: CompareStateSample {
                    x: reference.xs[tick],
                    y: reference.ys[tick],
                    vx: reference.vxs[tick],
                    vy: reference.vys[tick],
                },
                candidate: CompareStateSample {
                    x: candidate.xs[tick],
                    y: candidate.ys[tick],
                    vx: candidate.vxs[tick],
                    vy: candidate.vys[tick],
                },
            });
            break;
        }
    }

    let final_state = if sample_count == 0 {
        CompareFinalState::default()
    } else {
        let index = sample_count - 1;
        CompareFinalState {
            reference_x: Some(reference.xs[index]),
            candidate_x: Some(candidate.xs[index]),
            delta_x: Some(candidate.xs[index] - reference.xs[index]),
            reference_vx: Some(reference.vxs[index]),
            candidate_vx: Some(candidate.vxs[index]),
        }
    };

    ModelCompareResult {
        ok: true,
        ticks,
        sample_count,
        reference_engine: reference.engine.clone(),
        candidate_engine: candidate.engine.clone(),
        max_abs_dx,
        max_abs_dy,
        max_abs_dvx,
        max_abs_dvy,
        final_state,
        first_difference,
    }
}

pub fn compare_structure_models(
    reference: &SimulationSeries,
    candidate: &SimulationSeries,
    ticks: usize,
) -> ModelCompareResult {
    compare_simulations(reference, candidate, ticks)
}

pub fn compare_structure_to_reference(
    structure: &Structure,
    ticks: usize,
    reference: &SimulationSeries,
) -> Result<ModelCompareResult, String> {
    let effective_ticks = simulation_ticks_for_requested_duration(structure, ticks);
    let candidate = simulate_structure(structure, effective_ticks)?;
    Ok(compare_simulations(reference, &candidate, effective_ticks))
}

fn simulation_series_from_simulation(sim: Simulation) -> SimulationSeries {
    SimulationSeries {
        engine: MODEL_ENGINE.to_string(),
        xs: sim.xs,
        ys: sim.ys,
        vxs: sim.vxs,
        vys: sim.vys,
        on_grounds: sim.on_grounds.into_iter().map(|value| value != 0).collect(),
        floors: sim
            .floors
            .into_iter()
            .map(|floor| FloorName::from(floor.as_str().to_string()))
            .collect(),
    }
}

fn structure_to_layout(structure: &Structure) -> Result<Layout, String> {
    let prefix = structure
        .prefix
        .iter()
        .map(schema_cell_to_cell)
        .collect::<Result<Vec<_>, _>>()?;
    let cycle = structure
        .cycle
        .iter()
        .map(schema_cell_to_cell)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Layout::new(&prefix, &cycle))
}

fn schema_cell_to_cell(cell: &crate::schema::Cell) -> Result<crate::Cell, String> {
    let floor = parse_floor(cell.floor.as_str())?;
    let amount = cell.canonical_amount().min(8);
    Ok(crate::Cell {
        surface: cell.canonical_surface(),
        flow: cell.canonical_flow(),
        amount,
        floor,
    })
}

fn launch_value<'a>(structure: &'a Structure) -> Option<&'a serde_json::Map<String, Value>> {
    structure.extra.get("launch")?.as_object()
}

fn launch_mode(structure: &Structure) -> Option<String> {
    launch_value(structure)?
        .get("mode")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn launch_is_active(structure: &Structure) -> bool {
    let Some(launch) = launch_value(structure) else {
        return false;
    };
    if launch.get("applied").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    launch
        .get("timelineSamples")
        .and_then(Value::as_array)
        .is_some_and(|samples| samples.len() >= 2)
}

fn launch_timeline_offset(structure: &Structure) -> usize {
    if !launch_is_active(structure) {
        return 0;
    }
    launch_value(structure)
        .and_then(|launch| launch.get("timelineOffsetGt"))
        .and_then(as_non_negative_usize)
        .unwrap_or(0)
}

pub fn stitch_launch_timeline(
    structure: &Structure,
    sim: &SimulationSeries,
) -> Result<SimulationSeries, String> {
    if !launch_is_active(structure) {
        return Ok(sim.clone());
    }
    let Some(launch) = launch_value(structure) else {
        return Ok(sim.clone());
    };
    let Some(samples) = launch.get("timelineSamples").and_then(Value::as_array) else {
        return Ok(sim.clone());
    };
    if samples.len() < 2 {
        return Ok(sim.clone());
    }
    let prefix = &samples[..samples.len() - 1];
    if prefix.is_empty() {
        return Ok(sim.clone());
    }

    let layout = structure_to_layout(structure)?;
    let mut stitched = SimulationSeries {
        engine: sim.engine.clone(),
        xs: Vec::with_capacity(prefix.len() + sim.xs.len()),
        ys: Vec::with_capacity(prefix.len() + sim.ys.len()),
        vxs: Vec::with_capacity(prefix.len() + sim.vxs.len()),
        vys: Vec::with_capacity(prefix.len() + sim.vys.len()),
        on_grounds: Vec::with_capacity(prefix.len() + sim.on_grounds.len()),
        floors: Vec::with_capacity(prefix.len() + sim.floors.len()),
    };

    for sample in prefix {
        let x = sample.get("x").and_then(as_f64).unwrap_or(0.0);
        stitched.xs.push(x);
        stitched.ys.push(sample.get("y").and_then(as_f64).unwrap_or(0.0));
        stitched.vxs.push(sample.get("vx").and_then(as_f64).unwrap_or(0.0));
        stitched.vys.push(sample.get("vy").and_then(as_f64).unwrap_or(0.0));
        stitched
            .on_grounds
            .push(sample.get("onGround").and_then(Value::as_bool).unwrap_or(false));
        stitched
            .floors
            .push(FloorName::from(floor_at(&layout, x).as_str().to_string()));
    }

    stitched.xs.extend_from_slice(&sim.xs);
    stitched.ys.extend_from_slice(&sim.ys);
    stitched.vxs.extend_from_slice(&sim.vxs);
    stitched.vys.extend_from_slice(&sim.vys);
    stitched.on_grounds.extend_from_slice(&sim.on_grounds);
    stitched.floors.extend(sim.floors.iter().cloned());
    Ok(stitched)
}

fn block_dwell_groups(points: &[ViewerPoint]) -> Vec<DwellGroup> {
    let mut groups: Vec<DwellGroup> = Vec::new();
    for point in points {
        let block_x = raw_block_at(point).unwrap_or(0);
        match groups.last_mut() {
            Some(group) if group.block_x == block_x => group.points.push(point.clone()),
            _ => groups.push(DwellGroup {
                block_x,
                points: vec![point.clone()],
            }),
        }
    }
    groups
}

fn filter_dwell_groups<'a>(
    groups: &'a [DwellGroup],
    min_tick: Option<usize>,
    min_block: Option<i64>,
    max_block: Option<i64>,
) -> Vec<&'a DwellGroup> {
    groups
        .iter()
        .filter(|group| {
            let start_tick = group.points.first().map(|point| point.tick_index).unwrap_or(0);
            (min_tick.is_none() || start_tick >= min_tick.unwrap_or(0))
                && (min_block.is_none() || group.block_x >= min_block.unwrap_or(i64::MIN))
                && (max_block.is_none() || group.block_x <= max_block.unwrap_or(i64::MAX))
        })
        .collect()
}

fn first_stable_dwell_group(
    groups: &[DwellGroup],
    target_dwell: usize,
    min_tick: Option<usize>,
    min_block: Option<i64>,
    min_stable_groups: usize,
) -> Option<&DwellGroup> {
    let needed = 3.max(min_stable_groups.min(groups.len()));
    for index in 0..groups.len() {
        let group = &groups[index];
        let start_tick = group.points.first().map(|point| point.tick_index).unwrap_or(0);
        if min_tick.is_some_and(|value| start_tick < value) {
            continue;
        }
        if min_block.is_some_and(|value| group.block_x < value) {
            continue;
        }

        let mut stable_count = 0;
        let mut previous_block = None;
        for cursor_group in groups.iter().skip(index) {
            if let Some(previous_block) = previous_block {
                if cursor_group.block_x != previous_block + 1 {
                    break;
                }
            }
            if cursor_group.points.len() != target_dwell {
                break;
            }
            stable_count += 1;
            previous_block = Some(cursor_group.block_x);
            if stable_count >= needed {
                return Some(group);
            }
        }
    }
    None
}

fn score_dwell_groups(groups: &[&DwellGroup], target_dwell: usize) -> (usize, usize, usize) {
    let mut hits = 0;
    let mut scored = 0;
    let mut tail_inferred_hits = 0;
    for (index, group) in groups.iter().enumerate() {
        let is_tail = index + 1 == groups.len();
        if group.points.len() == target_dwell {
            hits += 1;
            scored += 1;
            continue;
        }
        if is_tail && !group.points.is_empty() && group.points.len() < target_dwell {
            let previous = &groups[index.saturating_sub(8)..index];
            let previous_blocks = previous.iter().map(|item| item.block_x).collect::<Vec<_>>();
            let has_continuous_previous = previous.len() >= 8
                && previous.iter().all(|item| item.points.len() == target_dwell)
                && previous_blocks
                    .windows(2)
                    .all(|window| window[1] == window[0] + 1)
                && previous_blocks
                    .last()
                    .is_some_and(|block| group.block_x == *block + 1);
            if has_continuous_previous {
                hits += 1;
                scored += 1;
                tail_inferred_hits = 1;
                continue;
            }
        }
        scored += 1;
    }
    (hits, scored, tail_inferred_hits)
}

fn cadence_window_metrics(
    points: &[ViewerPoint],
    start_tick: usize,
    pair_count: usize,
    cadence_ticks: usize,
    target_speed: f64,
) -> Option<CadenceMetrics> {
    let end_tick = start_tick + pair_count * cadence_ticks;
    if end_tick >= points.len() {
        return None;
    }
    let target_distance = target_speed * cadence_ticks as f64;
    let mut block_hits = 0usize;
    let mut mean_abs_distance_error = 0.0;
    for index in 0..pair_count {
        let t0 = start_tick + index * cadence_ticks;
        let t1 = t0 + cadence_ticks;
        let x0 = x_raw_at(&points[t0]).unwrap_or(0.0);
        let x1 = x_raw_at(&points[t1]).unwrap_or(0.0);
        let distance = x1 - x0;
        mean_abs_distance_error += (distance - target_distance).abs();
        if x1.floor() as i64 - x0.floor() as i64 == 1 {
            block_hits += 1;
        }
    }
    let total_distance = x_raw_at(&points[end_tick]).unwrap_or(0.0)
        - x_raw_at(&points[start_tick]).unwrap_or(0.0);
    Some(CadenceMetrics {
        pairs: pair_count,
        block_hit_rate: block_hits as f64 / pair_count as f64,
        mean_abs_distance_error: mean_abs_distance_error / pair_count as f64,
        average_speed: total_distance / (pair_count * cadence_ticks) as f64,
    })
}

pub fn detect_steady_window(
    points: &[ViewerPoint],
    options: &ViewerRunOptions,
) -> Option<SteadyWindowDetection> {
    if points.len() < 6 {
        return None;
    }
    let target_speed = option_f64(options, "targetSpeed").unwrap_or(0.5);
    let target_dwell = option_usize(options, "targetDwellTicks")
        .unwrap_or_else(|| default_target_dwell_ticks(target_speed));
    let cadence_ticks = target_dwell.max(1);
    let max_pairs = ((points.len() - 1) / cadence_ticks).saturating_sub(1);
    let pair_count = option_usize(options, "steadyDetectPairs")
        .unwrap_or(20)
        .clamp(3, max_pairs.max(3));
    if max_pairs == 0 {
        return None;
    }
    let tolerance = option_f64(options, "steadyCadenceTolerance").unwrap_or(0.05);
    let speed_tolerance = option_f64(options, "steadySpeedTolerance").unwrap_or(0.02);
    let block_hit_threshold = option_f64(options, "steadyBlockHitRate").unwrap_or(0.98);
    let steady_min_block = option_i64(options, "steadyMinBlock");
    let groups = block_dwell_groups(points);
    let mut best_score = None::<f64>;

    for start_tick in 0..points.len().saturating_sub(pair_count * cadence_ticks) {
        let Some(metrics) =
            cadence_window_metrics(points, start_tick, pair_count, cadence_ticks, target_speed)
        else {
            continue;
        };
        let score = (1.0 - metrics.block_hit_rate) * 100.0
            + metrics.mean_abs_distance_error * 10.0
            + (metrics.average_speed - target_speed).abs() * 20.0
            + start_tick as f64 * 0.01;
        if best_score.is_none_or(|best| score < best) {
            best_score = Some(score);
        }
        if metrics.block_hit_rate < block_hit_threshold
            || metrics.mean_abs_distance_error > tolerance
            || (metrics.average_speed - target_speed).abs() > speed_tolerance
        {
            continue;
        }
        let raw_block = raw_block_at(&points[start_tick]).unwrap_or(0);
        let min_block = steady_min_block.map_or(raw_block, |value| value.max(raw_block));
        let Some(stable_group) = first_stable_dwell_group(
            &groups,
            target_dwell,
            Some(start_tick),
            Some(min_block),
            metrics.pairs,
        ) else {
            continue;
        };
        let start_point = stable_group.points.first()?;
        return Some(SteadyWindowDetection {
            tick: start_point.tick_index,
            raw_block: stable_group.block_x,
            display_block: display_block_at(start_point).unwrap_or(stable_group.block_x),
            block_hit_rate: metrics.block_hit_rate,
            mean_abs_distance_error: metrics.mean_abs_distance_error,
            average_speed: metrics.average_speed,
        });
    }
    None
}

fn extend_summary_metrics(
    target: &mut BTreeMap<String, Value>,
    metrics: &ViewerRunMetrics,
) -> Result<(), String> {
    let value = serde_json::to_value(metrics)
        .map_err(|error| format!("Failed to serialize viewer run metrics: {error}"))?;
    let Some(object) = value.as_object() else {
        return Err("Viewer run metrics did not serialize to an object.".to_string());
    };
    for (key, value) in object {
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn extend_launch_summary(structure: &Structure, target: &mut BTreeMap<String, Value>) {
    let Some(launch) = launch_value(structure) else {
        return;
    };
    insert_summary_metric(target, "launch_applied", json!(launch.get("applied")));
    insert_summary_metric(target, "launch_piston_ticks", json!(launch.get("pistonTicks")));
    insert_summary_metric(target, "launch_collision_gt", json!(launch.get("collisionGt")));
    insert_summary_metric(
        target,
        "launch_last_collision_gt",
        json!(launch.get("lastCollisionGt")),
    );
    insert_summary_metric(
        target,
        "launch_collision_count",
        json!(launch.get("collisionCount")),
    );
    insert_summary_metric(target, "launch_slime_block_x", json!(launch.get("slimeBlockX")));
    insert_summary_metric(
        target,
        "launch_piston_movement",
        json!(launch.get("pistonMovement")),
    );
    insert_summary_metric(
        target,
        "launch_piston_movement_total",
        json!(launch.get("pistonMovementTotal")),
    );

    let raw_start = launch.get("rawStart").and_then(Value::as_object);
    insert_summary_metric(
        target,
        "launch_raw_start_x",
        json!(raw_start.and_then(|value| value.get("x"))),
    );
    insert_summary_metric(
        target,
        "launch_raw_start_vx",
        json!(raw_start.and_then(|value| value.get("vx"))),
    );

    let effective_start = launch.get("effectiveStart").and_then(Value::as_object);
    insert_summary_metric(
        target,
        "launch_effective_start_x",
        json!(effective_start.and_then(|value| value.get("x"))),
    );
    insert_summary_metric(
        target,
        "launch_effective_start_vx",
        json!(effective_start.and_then(|value| value.get("vx"))),
    );
}

fn insert_summary_metric(target: &mut BTreeMap<String, Value>, key: &str, value: Value) {
    if !value.is_null() {
        target.insert(key.to_string(), value);
    }
}

fn option_bool(options: &ViewerRunOptions, key: &str, fallback: bool) -> bool {
    options.get(key).and_then(Value::as_bool).unwrap_or(fallback)
}

fn option_f64(options: &ViewerRunOptions, key: &str) -> Option<f64> {
    options.get(key).and_then(as_f64)
}

fn option_usize(options: &ViewerRunOptions, key: &str) -> Option<usize> {
    options.get(key).and_then(as_non_negative_usize)
}

fn option_i64(options: &ViewerRunOptions, key: &str) -> Option<i64> {
    options.get(key).and_then(as_i64)
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn as_non_negative_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .or_else(|| {
                number
                    .as_i64()
                    .filter(|value| *value >= 0)
                    .and_then(|value| usize::try_from(value).ok())
            }),
        Value::String(text) => text.parse::<usize>().ok(),
        _ => None,
    }
}

pub fn display_origin_from_start_x_raw(start_x_raw: f64) -> f64 {
    start_x_raw.floor()
}

pub fn display_x_from_raw(x_raw: f64, start_x_raw: f64) -> f64 {
    x_raw - display_origin_from_start_x_raw(start_x_raw)
}

fn x_raw_at(point: &ViewerPoint) -> Option<f64> {
    point.x_raw.or(point.x)
}

fn raw_block_at(point: &ViewerPoint) -> Option<i64> {
    x_raw_at(point).map(|value| value.floor() as i64)
}

fn display_block_at(point: &ViewerPoint) -> Option<i64> {
    point.x.or(point.x_raw).map(|value| value.floor() as i64)
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn default_target_dwell_ticks(target_speed: f64) -> usize {
    if !target_speed.is_finite() || target_speed <= 0.0 {
        1
    } else {
        ((1.0 / target_speed).round() as usize).max(1)
    }
}

fn max_delta(left: &[f64], right: &[f64], sample_count: usize) -> f64 {
    (0..sample_count)
        .map(|index| (left[index] - right[index]).abs())
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::default_structure;
    use serde_json::json;

    fn point(tick_index: usize, x_raw: f64) -> ViewerPoint {
        ViewerPoint {
            tick_index,
            x: Some(x_raw),
            x_raw: Some(x_raw),
            speed: Some(0.5),
            derived_speed: (tick_index > 0).then_some(0.5),
            y: Some(0.0),
            vy: Some(0.0),
            floor: Some(FloorName::from("normal")),
            on_ground: Some(true),
            log_time: None,
            captured_at: None,
            raw_line: None,
            extra: BTreeMap::new(),
        }
    }

    fn points_from_raw_xs(xs: &[f64]) -> Vec<ViewerPoint> {
        let start_x_raw = xs.first().copied().unwrap_or(0.0);
        let mut previous = None;
        xs.iter()
            .enumerate()
            .map(|(tick_index, &x_raw)| {
                let derived_speed = previous.map(|value| x_raw - value);
                previous = Some(x_raw);
                ViewerPoint {
                    tick_index,
                    x: Some(display_x_from_raw(x_raw, start_x_raw)),
                    x_raw: Some(x_raw),
                    speed: derived_speed,
                    derived_speed,
                    y: Some(0.0),
                    vy: Some(0.0),
                    floor: Some(FloorName::from("normal")),
                    on_ground: Some(true),
                    log_time: None,
                    captured_at: None,
                    raw_line: None,
                    extra: BTreeMap::new(),
                }
            })
            .collect()
    }

    #[test]
    fn simulation_ticks_respect_launch_offset() {
        let mut structure = default_structure();
        structure.extra.insert(
            "launch".to_string(),
            json!({
                "mode": "piston",
                "applied": true,
                "timelineOffsetGt": 3,
                "timelineSamples": [
                    {"x": -0.5, "y": 0.0, "vx": 0.0, "vy": 0.0, "onGround": true},
                    {"x": -0.1, "y": 0.0, "vx": 0.2, "vy": 0.0, "onGround": true},
                    {"x": 0.2, "y": 0.0, "vx": 0.3, "vy": 0.0, "onGround": false},
                    {"x": 0.5, "y": 0.0, "vx": 0.4, "vy": 0.0, "onGround": false}
                ]
            }),
        );
        assert_eq!(simulation_ticks_for_requested_duration(&structure, 12), 9);
    }

    #[test]
    fn make_run_populates_summary_metrics() {
        let structure = default_structure();
        let sim = simulate_structure(&structure, 8).expect("simulate structure");
        let run = make_run(&structure, &sim, &ViewerRunOptions::new(), "demo")
            .expect("make viewer run");
        assert_eq!(run.label.as_deref(), Some("demo"));
        assert_eq!(run.summary.source.as_deref(), Some("simulation"));
        assert_eq!(
            run.summary.model_engine.as_deref(),
            Some("item-waterway-solver-rust")
        );
        assert_eq!(
            run.summary
                .extra
                .get("sample_count")
                .and_then(Value::as_u64),
            Some(run.points.len() as u64)
        );
        assert!(run.summary.extra.contains_key("target_speed"));
        assert!(run.summary.extra.contains_key("steady_avg_speed"));
    }

    #[test]
    fn detect_steady_window_finds_two_tick_cadence() {
        let points = vec![
            point(0, 0.0),
            point(1, 0.5),
            point(2, 1.0),
            point(3, 1.5),
            point(4, 2.0),
            point(5, 2.5),
            point(6, 3.0),
            point(7, 3.5),
        ];
        let options = ViewerRunOptions::from([
            ("targetSpeed".to_string(), json!(0.5)),
            ("targetDwellTicks".to_string(), json!(2)),
        ]);
        let detected = detect_steady_window(&points, &options).expect("steady window");
        assert_eq!(detected.tick, 0);
        assert_eq!(detected.raw_block, 0);
        assert_eq!(detected.display_block, 0);
        assert!((detected.block_hit_rate - 1.0).abs() < 1.0e-12);
        assert!((detected.average_speed - 0.5).abs() < 1.0e-12);
    }

    #[test]
    fn explicit_dwell_window_does_not_override_detected_steady_start() {
        let mut xs = vec![0.25, 0.75, 1.25];
        for block in 2..=12 {
            xs.push(block as f64 + 0.125);
            xs.push(block as f64 + 0.625);
        }
        let points = points_from_raw_xs(&xs);
        let options = ViewerRunOptions::from([
            ("targetSpeed".to_string(), json!(0.5)),
            ("targetDwellTicks".to_string(), json!(2)),
            ("steadyDetectPairs".to_string(), json!(3)),
            ("minBlock".to_string(), json!(8)),
            ("maxBlock".to_string(), json!(12)),
            ("minStartTick".to_string(), json!(15)),
        ]);
        let metrics = metrics_for_points(&points, &options);
        assert_eq!(metrics.dwell_min_block, Some(8));
        assert_eq!(metrics.dwell_max_block, Some(12));
        assert_eq!(metrics.dwell_min_start_tick, Some(15));
        assert_eq!(metrics.steady_start_tick, Some(3));
        assert_eq!(metrics.steady_start_raw_block, Some(2));
        assert_eq!(metrics.steady_start_block, Some(2));
        assert_eq!(metrics.steady_source.as_deref(), Some("detected"));
        assert_eq!(metrics.steady_end_raw_block, Some(12));
        assert_eq!(metrics.steady_dwell_failures, 0);
        assert_eq!(metrics.dwell_failures, 0);
    }

    #[test]
    fn explicit_dwell_window_does_not_create_fake_steady_start() {
        let points = points_from_raw_xs(&[0.25, 1.25, 2.25, 3.25, 4.25, 5.25, 6.25, 7.25]);
        let options = ViewerRunOptions::from([
            ("targetSpeed".to_string(), json!(0.5)),
            ("targetDwellTicks".to_string(), json!(2)),
            ("minBlock".to_string(), json!(2)),
            ("maxBlock".to_string(), json!(7)),
            ("minStartTick".to_string(), json!(2)),
        ]);
        let metrics = metrics_for_points(&points, &options);
        assert_eq!(metrics.dwell_min_block, Some(2));
        assert_eq!(metrics.dwell_max_block, Some(7));
        assert_eq!(metrics.dwell_min_start_tick, Some(2));
        assert_eq!(metrics.steady_start_tick, None);
        assert_eq!(metrics.steady_start_raw_block, None);
        assert_eq!(metrics.steady_start_block, None);
        assert_eq!(metrics.steady_end_raw_block, None);
        assert_eq!(metrics.steady_source, None);
    }

    #[test]
    fn stitch_launch_timeline_prepends_prior_launch_samples() {
        let mut structure = default_structure();
        structure.extra.insert(
            "launch".to_string(),
            json!({
                "mode": "piston",
                "applied": true,
                "timelineSamples": [
                    {"x": -0.5, "y": 0.0, "vx": 0.0, "vy": 0.0, "onGround": true},
                    {"x": 0.0, "y": 0.0, "vx": 0.1, "vy": 0.0, "onGround": true},
                    {"x": 0.25, "y": 0.0, "vx": 0.2, "vy": 0.0, "onGround": true}
                ]
            }),
        );
        let sim = SimulationSeries {
            engine: MODEL_ENGINE.to_string(),
            xs: vec![0.25, 0.75],
            ys: vec![0.0, 0.0],
            vxs: vec![0.2, 0.3],
            vys: vec![0.0, 0.0],
            on_grounds: vec![true, true],
            floors: vec![FloorName::from("normal"), FloorName::from("normal")],
        };
        let stitched = stitch_launch_timeline(&structure, &sim).expect("stitched timeline");
        assert_eq!(stitched.xs, vec![-0.5, 0.0, 0.25, 0.75]);
        assert_eq!(stitched.vxs, vec![0.0, 0.1, 0.2, 0.3]);
        assert_eq!(stitched.on_grounds, vec![true, true, true, true]);
        assert_eq!(stitched.floors.len(), 4);
    }

    #[test]
    fn stitch_launch_timeline_supports_water_preface_samples() {
        let mut structure = default_structure();
        structure.extra.insert(
            "launch".to_string(),
            json!({
                "mode": "water",
                "applied": true,
                "timelineSamples": [
                    {"x": -0.875, "y": 0.0, "vx": 0.0, "vy": 0.0, "onGround": false},
                    {"x": -0.3650000000000091, "y": 0.0, "vx": 1.0, "vy": 0.0, "onGround": false}
                ]
            }),
        );
        let sim = SimulationSeries {
            engine: MODEL_ENGINE.to_string(),
            xs: vec![-0.3650000000000091, 0.6349999999999909],
            ys: vec![0.0, 0.0],
            vxs: vec![1.0, 0.5880000591278076],
            vys: vec![0.0, 0.0],
            on_grounds: vec![false, true],
            floors: vec![FloorName::from("normal"), FloorName::from("packed_ice")],
        };
        let stitched = stitch_launch_timeline(&structure, &sim).expect("stitched water preface");
        assert_eq!(stitched.xs, vec![-0.875, -0.3650000000000091, 0.6349999999999909]);
        assert_eq!(stitched.vxs, vec![0.0, 1.0, 0.5880000591278076]);
        assert_eq!(stitched.on_grounds, vec![false, false, true]);
    }

    #[test]
    fn piston_launch_viewer_points_match_user_logged_world_positions() {
        let mut structure = default_structure();
        structure.start.x = 0.135;
        structure.start.y = 0.0;
        structure.start.vx = 1.0;
        structure.start.vy = -0.08;
        structure.start.start_on_ground = Some(false);
        structure.start.initial_tick_count = 2;
        structure.prefix = vec![
            crate::schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            crate::schema::make_cell(Some(7.0 / 9.0), -1, "glass", None, Some(7)),
            crate::schema::make_cell(Some(8.0 / 9.0), -1, "glass", None, Some(8)),
            crate::schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            crate::schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            crate::schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            crate::schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            crate::schema::make_cell(None, 0, "blue_ice", None, Some(0)),
        ];
        structure.cycle = vec![
            crate::schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            crate::schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            crate::schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            crate::schema::make_cell(Some(8.0 / 9.0), 0, "blue_ice", None, Some(8)),
        ];
        structure.extra.insert(
            "launch".to_string(),
            json!({
                "mode": "piston",
                "applied": true,
                "displayOriginX": 1.0,
                "timelineOffsetGt": 2,
                "timelineSamples": [
                    {"gt": 0, "x": -0.25, "y": 0.0, "vx": 0.0, "vy": 0.0, "onGround": true, "pistonCollision": false},
                    {"gt": 1, "x": -0.25, "y": 0.0, "vx": 0.0, "vy": -0.04, "onGround": true, "pistonCollision": false},
                    {"gt": 2, "x": 0.135, "y": 0.0, "vx": 1.0, "vy": -0.08, "onGround": false, "pistonCollision": true}
                ]
            }),
        );

        let sim = simulate_structure_for_requested_duration(&structure, 6).expect("simulate structure");
        let points = simulation_to_viewer_points(&structure, &sim).expect("viewer points");
        let expected_raw_xs = [
            0.75,
            0.75,
            1.135,
            2.135,
            2.6894000638771063,
            3.1995590212336724,
            3.666795255795139,
        ];
        for (index, expected) in expected_raw_xs.iter().enumerate() {
            assert!(
                (points[index].x_raw.unwrap_or_default() - expected).abs() < 1.0e-9,
                "raw x mismatch at tick {index}: got {}, expected {}",
                points[index].x_raw.unwrap_or_default(),
                expected
            );
        }
        let expected_display_xs = [
            0.75,
            0.75,
            1.135,
            2.135,
            2.6894000638771063,
            3.1995590212336724,
            3.666795255795139,
        ];
        for (index, expected) in expected_display_xs.iter().enumerate() {
            assert!(
                (points[index].x.unwrap_or_default() - expected).abs() < 1.0e-9,
                "display x mismatch at tick {index}: got {}, expected {}",
                points[index].x.unwrap_or_default(),
                expected
            );
        }
    }

    #[test]
    fn compare_simulations_reports_first_difference() {
        let mut left = SimulationSeries {
            engine: "left".to_string(),
            xs: vec![0.0, 1.0, 2.0],
            ys: vec![0.0, 0.0, 0.0],
            vxs: vec![0.5, 0.5, 0.5],
            vys: vec![0.0, 0.0, 0.0],
            on_grounds: vec![true, true, true],
            floors: vec![FloorName::from("normal"); 3],
        };
        let mut right = left.clone();
        right.engine = "right".to_string();
        right.xs[2] = 2.1;
        let comparison = compare_simulations(&left, &right, 2);
        assert!(comparison.ok);
        assert_eq!(comparison.sample_count, 3);
        assert_eq!(comparison.reference_engine, "left");
        assert_eq!(comparison.candidate_engine, "right");
        assert_eq!(
            comparison.first_difference.as_ref().map(|difference| difference.tick),
            Some(2)
        );
        left.xs[2] = 2.1;
        let exact = compare_simulations(&left, &right, 2);
        assert!(exact.first_difference.is_none());
    }
}
