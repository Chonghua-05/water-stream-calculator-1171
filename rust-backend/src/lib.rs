use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub mod run_store;
pub mod search_tasks;
pub mod schema;
pub mod service;
pub mod litematic;
pub mod pipeline;
pub mod storage_analysis;
pub mod viewer_runs;
mod candidate_eval;
mod reachable_candidates;
mod reachability;

const WIDTH: f64 = 0.25;
const HEIGHT: f64 = 0.25;
const FLUID_MOVEMENT_THRESHOLD: f64 = (0.25_f32 * 0.85_f32 - 0.11111111_f32) as f64;
const WATER_PUSH: f64 = 0.014;
const HORIZONTAL_WATER_DAMPING: f64 = 0.99_f32 as f64;
const HORIZONTAL_MOVEMENT_DAMPING: f64 = 0.98_f32 as f64;
const VERTICAL_MOVEMENT_DAMPING: f64 = 0.98;
const GRAVITY: f64 = 0.04;
const BUOYANCY: f64 = 5.0e-4_f32 as f64;
const BUOYANCY_CAP: f64 = 0.06_f32 as f64;
const SLIME_STEP_ON_VY_THRESHOLD: f64 = 0.1;
const SLIME_STEP_ON_BASE: f64 = 0.4;
const SLIME_STEP_ON_VY_SCALE: f64 = 0.2;
const HORIZONTAL_REST_THRESHOLD2: f64 = 1.0e-5_f32 as f64;
const AABB_DEFLATE: f64 = 0.001;
const MOVEMENT_SAMPLE_MODULO: usize = 4;
const FLUID_CURRENT_MIN_OLD_MOVEMENT: f64 = 0.003;
const FLUID_CURRENT_MIN_IMPULSE: f64 = 0.0045;
const FLUID_CURRENT_EPSILON2: f64 = 1.0e-5_f32 as f64;
const POST_PISTON_REFERENCE_START_X: f64 = -0.365;
const POST_PISTON_REFERENCE_START_X_TOLERANCE: f64 = 0.02;
pub(crate) const TRANSIENT_SHAPE_WINDOW_SHORT: usize = 16;
pub(crate) const TRANSIENT_SHAPE_WINDOW_LONG: usize = 24;
pub(crate) const TRANSIENT_SHAPE_SCORE_WEIGHT: f64 = 3200.0;
const GAME2_POST_PISTON_DERIVED_SPEEDS: [f64; 23] = [
    1.0,
    0.5880000591278076,
    0.5313481354846772,
    0.487793975779482,
    0.4732577290710651,
    0.4591546621043108,
    0.47319186640197586,
    0.4868107624077993,
    0.5000238156972046,
    0.5128431203714854,
    0.5025862677457553,
    0.4826838741182655,
    0.4866539229284399,
    0.4998716500299451,
    0.5126954892367621,
    0.5251371783973582,
    0.5146344448455693,
    0.4987940183655155,
    0.4790417976431627,
    0.46007176397415606,
    0.46515441783924416,
    0.47901282958537195,
    0.492458261052775,
];

fn java_f32(value: f64) -> f64 {
    value as f32 as f64
}

pub(crate) fn uses_post_piston_game2_reference(
    start_x: f64,
    start_vx: f64,
    start_on_ground: bool,
) -> bool {
    start_vx >= 0.99
        && !start_on_ground
        && (start_x - POST_PISTON_REFERENCE_START_X).abs() <= POST_PISTON_REFERENCE_START_X_TOLERANCE
}

pub(crate) fn post_piston_game2_transient_mae(xs: &[f64], window: usize) -> Option<f64> {
    if xs.len() < 2 || window == 0 {
        return None;
    }
    let limit = window
        .min(GAME2_POST_PISTON_DERIVED_SPEEDS.len())
        .min(xs.len().saturating_sub(1));
    if limit == 0 {
        return None;
    }
    let total = (0..limit)
        .map(|index| {
            let derived_speed = xs[index + 1] - xs[index];
            (derived_speed - GAME2_POST_PISTON_DERIVED_SPEEDS[index]).abs()
        })
        .sum::<f64>();
    Some(total / limit as f64)
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Early,
    Full,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub out: PathBuf,
    pub mode: Mode,
    pub ticks: usize,
    pub top: usize,
    pub max_prefix: usize,
    pub beam_width: usize,
    pub bucket_keep: usize,
    pub frontier_structure_keep: usize,
    pub workers: usize,
    pub cadence_pairs: usize,
    pub cadence_tolerance: f64,
    pub long_window: usize,
    pub start_samples: usize,
    pub keep_weak: bool,
    pub min_early_block_hit_rate: f64,
    pub early_limit: usize,
    pub long_limit: usize,
    pub dedupe_long: bool,
    pub full_cadence_pairs: usize,
    pub full_cadence_tolerance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_start_offsets: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_names: Option<Vec<String>>,
    pub entity_id_mods: Vec<usize>,
    pub initial_tick_counts: Vec<usize>,
    pub start_y: f64,
    pub start_vx: f64,
    pub start_vy: f64,
    pub start_on_ground: bool,
}

pub enum ParsedArgs {
    Help(String),
    Run(Args),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StepOn {
    None,
    Slime,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Floor {
    Normal,
    PackedIce,
    BlueIce,
    Slime,
}

impl Floor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::PackedIce => "packed_ice",
            Self::BlueIce => "blue_ice",
            Self::Slime => "slime",
        }
    }

    fn code(self) -> char {
        match self {
            Self::Normal => 'N',
            Self::PackedIce => 'I',
            Self::BlueIce => 'B',
            Self::Slime => 'S',
        }
    }

    fn friction(self) -> f64 {
        match self {
            Self::Normal => 0.6_f32 as f64,
            Self::PackedIce => 0.98_f32 as f64,
            Self::BlueIce => 0.989_f32 as f64,
            Self::Slime => 0.8_f32 as f64,
        }
    }

    fn step_on(self) -> StepOn {
        match self {
            Self::Slime => StepOn::Slime,
            _ => StepOn::None,
        }
    }
}

#[derive(Clone, Debug)]
struct Cell {
    surface: Option<f64>,
    flow: i8,
    amount: u8,
    floor: Floor,
}

impl Cell {
    fn friction(&self) -> f64 {
        self.floor.friction()
    }

    fn step_on(&self) -> StepOn {
        self.floor.step_on()
    }

    fn code(&self) -> String {
        let prefix = if self.surface.is_none() {
            'D'
        } else if self.flow < 0 {
            'R'
        } else if self.flow > 0 {
            'F'
        } else {
            'S'
        };
        if self.amount == 0 || self.surface.is_none() {
            format!("{prefix}-{}", self.floor.code())
        } else {
            format!("{prefix}{}-{}", self.amount, self.floor.code())
        }
    }
}

#[derive(Clone, Debug)]
struct PrefixAtom {
    name: &'static str,
    cells: Vec<Cell>,
}

#[derive(Clone, Debug)]
struct PrefixSpec {
    label: String,
    cells: Vec<Cell>,
    signature: String,
}

#[derive(Clone)]
pub struct CycleSpec {
    pub name: String,
    cells: Vec<Cell>,
    note: String,
    proven: bool,
    signature: String,
}

impl CycleSpec {
    pub fn period(&self) -> usize {
        self.cells.len()
    }
}

#[derive(Clone)]
struct Layout {
    prefix_length: usize,
    period: usize,
    total_length: usize,
    cells: Vec<Cell>,
}

#[derive(Clone)]
struct SimConfig {
    ticks: usize,
    start_x: f64,
    start_y: f64,
    start_vx: f64,
    start_vy: f64,
    entity_id_mod4: usize,
    initial_tick_count: usize,
    start_on_ground: Option<bool>,
}

#[derive(Clone)]
struct Simulation {
    xs: Vec<f64>,
    ys: Vec<f64>,
    vxs: Vec<f64>,
    vys: Vec<f64>,
    on_grounds: Vec<u8>,
    floors: Vec<Floor>,
}

#[derive(Clone)]
struct WindowMetricContext {
    vx_sum: Vec<f64>,
    vx_sq_sum: Vec<f64>,
}

#[derive(Clone, Debug)]
struct WindowMetrics {
    average_vx: f64,
    mean_vx_error: f64,
    std_vx: f64,
    average_distance_vx: f64,
    long_window_score: Option<f64>,
    long_window_start_tick: Option<usize>,
    suffix_start_tick: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EarlyCadenceSample {
    pub pair: usize,
    pub t0: usize,
    pub t1: usize,
    pub x0: f64,
    pub x1: f64,
    pub distance: f64,
    pub distance_error: f64,
    pub floor_delta: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullCadenceSample {
    pub pair: usize,
    pub t0: usize,
    pub t1: usize,
    pub x0: f64,
    pub x1: f64,
    pub distance: f64,
    pub distance_error: f64,
    pub floor_delta: i32,
    pub hit_margin: f64,
    pub endpoint_boundary_margin: f64,
}

#[derive(Clone, Debug)]
struct EarlyCadence {
    cadence_start_tick: usize,
    cadence_pairs: usize,
    cadence_mean_abs_distance_error: f64,
    cadence_mean_signed_distance_error: f64,
    cadence_max_abs_distance_error: f64,
    cadence_block_hit_rate: f64,
    cadence_within_tolerance_rate: f64,
    cadence_pass: bool,
    cadence_samples: Vec<EarlyCadenceSample>,
    early_cadence_score: f64,
}

#[derive(Clone, Debug)]
struct FullCadence {
    full_cadence_start_tick: usize,
    full_cadence_pairs: usize,
    full_cadence_mean_abs_distance_error: f64,
    full_cadence_mean_signed_distance_error: f64,
    full_cadence_max_abs_distance_error: f64,
    full_cadence_block_hit_rate: f64,
    full_cadence_within_tolerance_rate: f64,
    full_cadence_longest_hit_run: usize,
    full_cadence_first_miss: Option<EarlyCadenceSample>,
    full_cadence_min_hit_margin: f64,
    full_cadence_mean_hit_margin: f64,
    full_cadence_min_endpoint_boundary_margin: f64,
    full_cadence_mean_endpoint_boundary_margin: f64,
    full_cadence_samples: Vec<FullCadenceSample>,
    full_cadence_distance: f64,
    full_cadence_average_speed: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellDescription {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<f64>,
    pub flow: i8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_flow_hint: Option<i8>,
    pub amount: u8,
    pub floor: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstTick {
    pub tick: usize,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub floor: String,
    pub on_ground: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRow {
    pub id: String,
    pub pass: String,
    pub score: f64,
    pub early_score: f64,
    pub prefix_label: String,
    pub prefix_length: usize,
    pub backbone: String,
    pub proven: bool,
    pub start_offset: f64,
    pub entity_id_mod4: usize,
    pub initial_tick_count: usize,
    pub period: usize,
    pub cadence_start_tick: usize,
    pub cadence_pairs: usize,
    pub cadence_mean_abs_distance_error: f64,
    pub cadence_mean_signed_distance_error: f64,
    pub cadence_max_abs_distance_error: f64,
    pub cadence_block_hit_rate: f64,
    pub cadence_within_tolerance_rate: f64,
    pub cadence_pass: bool,
    pub cadence_samples: Vec<EarlyCadenceSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_start_tick: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_pairs: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_mean_abs_distance_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_mean_signed_distance_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_max_abs_distance_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_block_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_within_tolerance_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_longest_hit_run: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_average_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_first_miss: Option<EarlyCadenceSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_min_hit_margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_mean_hit_margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_min_endpoint_boundary_margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_mean_endpoint_boundary_margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_cadence_samples: Option<Vec<FullCadenceSample>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_window_start_tick: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_average_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_mean_vx_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_std_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_average_distance_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_average_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_mean_vx_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_std_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_average_distance_vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ticks: Option<Vec<FirstTick>>,
    pub prefix_cells: Vec<CellDescription>,
    pub cycle_cells: Vec<CellDescription>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPayload {
    pub evaluated: usize,
    pub early_kept: usize,
    pub early_deduped: usize,
    pub long_verified: usize,
    pub results: Vec<ResultRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstantsOutput {
    width: f64,
    height: f64,
    fluid_movement_threshold: f64,
    water_push: f64,
    horizontal_water_damping: f64,
    horizontal_movement_damping: f64,
    vertical_movement_damping: f64,
    gravity: f64,
    buoyancy: f64,
    buoyancy_cap: f64,
    slime_step_on_vy_threshold: f64,
    slime_step_on_base: f64,
    slime_step_on_vy_scale: f64,
    horizontal_rest_threshold2: f64,
    aabb_deflate: f64,
    movement_sample_modulo: usize,
    fluid_current_min_old_movement: f64,
    fluid_current_min_impulse: f64,
    fluid_current_epsilon2: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonOutput {
    generated_at: String,
    args: Args,
    constants: ConstantsOutput,
    evaluated: usize,
    early_kept: usize,
    early_deduped: usize,
    long_verified: usize,
    top: Vec<ResultRow>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSimulationRequest {
    ticks: usize,
    #[serde(default)]
    structure: Option<JsonStructure>,
    #[serde(default)]
    structures: Vec<JsonStructure>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonStructure {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    start: JsonStart,
    #[serde(default)]
    prefix: Vec<JsonCell>,
    #[serde(default)]
    cycle: Vec<JsonCell>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonStart {
    #[serde(default = "default_start_x")]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    vx: f64,
    #[serde(default)]
    vy: f64,
    #[serde(default)]
    start_on_ground: Option<bool>,
    #[serde(default)]
    entity_id_mod4: usize,
    #[serde(default)]
    initial_tick_count: usize,
}

impl Default for JsonStart {
    fn default() -> Self {
        Self {
            x: default_start_x(),
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            start_on_ground: None,
            entity_id_mod4: 0,
            initial_tick_count: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonCell {
    #[serde(default)]
    surface: Option<f64>,
    #[serde(default)]
    flow: i8,
    #[serde(default)]
    amount: Option<u8>,
    #[serde(default)]
    floor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSimulationResponse {
    ok: bool,
    engine: &'static str,
    simulations: Vec<JsonSimulation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSimulation {
    name: String,
    xs: Vec<f64>,
    ys: Vec<f64>,
    vxs: Vec<f64>,
    vys: Vec<f64>,
    on_grounds: Vec<bool>,
    floors: Vec<String>,
}

#[derive(Clone, Debug)]
struct LegalWaterwayArgs {
    out: PathBuf,
    period: usize,
    limit: usize,
    max_sources: usize,
    workers: usize,
    debug: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LegalWaterCell {
    Plate,
    Open,
    Source,
}

#[derive(Clone, Debug)]
struct StableWaterField {
    skeleton: Vec<LegalWaterCell>,
    amounts: Vec<u8>,
    sources: Vec<bool>,
    flows: Vec<i8>,
    iterations: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegalWaterwayOutput {
    ok: bool,
    engine: &'static str,
    generated_at: String,
    args: LegalWaterwayArgsOutput,
    evaluated_skeletons: usize,
    legal_waterfields: usize,
    deduped_rotations: usize,
    waterfields: Vec<LegalWaterFieldOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegalWaterwayArgsOutput {
    period: usize,
    limit: usize,
    max_sources: usize,
    workers: usize,
    debug: bool,
    blocker: &'static str,
    note: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegalWaterFieldOutput {
    id: String,
    period: usize,
    skeleton: String,
    amounts: String,
    flows: String,
    search_score: i64,
    source_count: usize,
    plate_count: usize,
    iterations: usize,
    cells: Vec<LegalWaterCellOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegalWaterCellOutput {
    index: usize,
    upper: &'static str,
    amount: u8,
    surface: Option<f64>,
    flow: i8,
    is_source: bool,
    blocker: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegalWaterwayCliResult {
    ok: bool,
    engine: &'static str,
    out_path: String,
    period: usize,
    workers: usize,
    evaluated_skeletons: usize,
    legal_waterfields: usize,
    deduped_rotations: usize,
}

fn default_start_x() -> f64 {
    0.125
}

#[derive(Clone, Copy, Debug, Default)]
struct FluidTracker {
    height: f64,
    accumulated_current_x: f64,
    current_count: usize,
}

impl FluidTracker {
    fn is_in_fluid(self) -> bool {
        self.height > 0.0
    }

    fn applies_underwater_movement(self) -> bool {
        self.height > FLUID_MOVEMENT_THRESHOLD
    }
}
impl Layout {
    fn new(prefix_cells: &[Cell], cycle_cells: &[Cell]) -> Self {
        let prefix_length = prefix_cells.len();
        let period = cycle_cells.len();
        let total_length = prefix_length + period;
        let mut cells = Vec::with_capacity(total_length);
        cells.extend_from_slice(prefix_cells);
        cells.extend_from_slice(cycle_cells);
        Self {
            prefix_length,
            period,
            total_length,
            cells,
        }
    }

    fn cell_index(&self, index: isize) -> Option<usize> {
        layout_cell_index(self.prefix_length, self.period, self.total_length, index)
    }

    fn cell_at(&self, index: isize) -> Option<&Cell> {
        self.cell_index(index).map(|resolved| &self.cells[resolved])
    }

    fn flow_direction_at(&self, index: isize) -> i8 {
        compute_flow_direction(
            &self.cells,
            self.prefix_length,
            self.period,
            self.total_length,
            index,
        )
    }
}

pub fn usage() -> String {
    "Usage: cargo run --release -- [reachable-candidates|candidate-eval|analyze-game-storage|simulate-json|--out <dir>] [--ticks 500] [--top 80] [--max-prefix 8]\n\nSearches transition prefixes for a slime-piston-launched 1.17.1 item, runs the Rust reachability candidate generator, evaluates a specific prefix/backbone candidate, analyzes captured storage samples, or reads a JSON simulation request from stdin when using simulate-json.".to_string()
}

pub fn parse_args(argv: &[String]) -> Result<ParsedArgs, String> {
    let mut args = Args {
        out: PathBuf::from("artifacts").join("item-waterway-launch-search"),
        mode: Mode::Full,
        ticks: 500,
        top: 80,
        max_prefix: 8,
        beam_width: 512,
        bucket_keep: 12,
        frontier_structure_keep: 24,
        workers: 8,
        cadence_pairs: 20,
        cadence_tolerance: 0.075,
        long_window: 200,
        start_samples: 33,
        keep_weak: false,
        min_early_block_hit_rate: 0.8,
        early_limit: 0,
        long_limit: 0,
        dedupe_long: true,
        full_cadence_pairs: 3000,
        full_cadence_tolerance: 0.05,
        fixed_start_offsets: None,
        cycle_names: None,
        entity_id_mods: vec![0, 1, 2, 3],
        initial_tick_counts: vec![0],
        start_y: 0.0,
        start_vx: 1.0,
        start_vy: 0.0,
        start_on_ground: true,
    };

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if arg == "--help" || arg == "-h" {
            return Ok(ParsedArgs::Help(usage()));
        }
        let next = |index: &mut usize| -> Result<&str, String> {
            *index += 1;
            argv.get(*index)
                .map(|value| value.as_str())
                .ok_or_else(|| format!("Missing value for {}", arg))
        };
        match arg.as_str() {
            "--out" => args.out = PathBuf::from(next(&mut i)?),
            "--mode" => {
                args.mode = match next(&mut i)? {
                    "early" => Mode::Early,
                    "full" => Mode::Full,
                    _ => return Err("--mode must be either 'early' or 'full'.".to_string()),
                }
            }
            "--early-only" => args.mode = Mode::Early,
            "--ticks" => args.ticks = parse_usize(next(&mut i)?, "--ticks")?,
            "--top" => args.top = parse_usize(next(&mut i)?, "--top")?,
            "--max-prefix" => args.max_prefix = parse_usize(next(&mut i)?, "--max-prefix")?,
            "--beam-width" => args.beam_width = parse_usize(next(&mut i)?, "--beam-width")?,
            "--bucket-keep" => args.bucket_keep = parse_usize(next(&mut i)?, "--bucket-keep")?,
            "--frontier-structure-keep" => {
                args.frontier_structure_keep =
                    parse_usize(next(&mut i)?, "--frontier-structure-keep")?
            }
            "--workers" => args.workers = parse_usize(next(&mut i)?, "--workers")?,
            "--cadence-pairs" => {
                args.cadence_pairs = parse_usize(next(&mut i)?, "--cadence-pairs")?
            }
            "--cadence-tolerance" => {
                args.cadence_tolerance = parse_f64(next(&mut i)?, "--cadence-tolerance")?
            }
            "--long-window" => args.long_window = parse_usize(next(&mut i)?, "--long-window")?,
            "--start-samples" => {
                args.start_samples = parse_usize(next(&mut i)?, "--start-samples")?
            }
            "--fixed-start-offsets" => {
                let values = parse_number_list(next(&mut i)?);
                args.fixed_start_offsets = Some(values);
            }
            "--cycles" => {
                let values = parse_string_list(next(&mut i)?);
                args.cycle_names = Some(values);
            }
            "--entity-id-mods" => {
                args.entity_id_mods = parse_usize_list(next(&mut i)?, "--entity-id-mods")?;
            }
            "--initial-tick-counts" => {
                args.initial_tick_counts =
                    parse_usize_list(next(&mut i)?, "--initial-tick-counts")?;
            }
            "--start-y" => args.start_y = parse_f64(next(&mut i)?, "--start-y")?,
            "--start-vx" => args.start_vx = parse_f64(next(&mut i)?, "--start-vx")?,
            "--start-vy" => args.start_vy = parse_f64(next(&mut i)?, "--start-vy")?,
            "--start-on-ground" => {
                args.start_on_ground = match next(&mut i)?.to_ascii_lowercase().as_str() {
                    "true" => true,
                    "false" => false,
                    _ => return Err("--start-on-ground must be true or false.".to_string()),
                }
            }
            "--keep-weak" => args.keep_weak = true,
            "--min-early-block-hit-rate" => {
                args.min_early_block_hit_rate =
                    parse_f64(next(&mut i)?, "--min-early-block-hit-rate")?
            }
            "--early-limit" => args.early_limit = parse_usize(next(&mut i)?, "--early-limit")?,
            "--long-limit" => args.long_limit = parse_usize(next(&mut i)?, "--long-limit")?,
            "--no-dedupe-long" => args.dedupe_long = false,
            "--full-cadence-pairs" => {
                args.full_cadence_pairs = parse_usize(next(&mut i)?, "--full-cadence-pairs")?
            }
            "--full-cadence-tolerance" => {
                args.full_cadence_tolerance = parse_f64(next(&mut i)?, "--full-cadence-tolerance")?
            }
            _ => return Err(format!("Unknown argument: {}\n{}", arg, usage())),
        }
        i += 1;
    }

    let minimum_early_ticks = 5 + args.cadence_pairs * 2 + 4;
    if args.ticks < minimum_early_ticks {
        match args.mode {
            Mode::Early => args.ticks = minimum_early_ticks,
            Mode::Full => {
                return Err("--ticks must be at least 5 + --cadence-pairs * 2 + 4.".to_string());
            }
        }
    }
    if matches!(args.mode, Mode::Full) && args.ticks < args.long_window + 10 {
        return Err("--ticks must be at least --long-window + 10 in full mode.".to_string());
    }
    if args.max_prefix > 16 {
        return Err("--max-prefix must be in [0, 16].".to_string());
    }
    if args.beam_width < 1 {
        return Err("--beam-width must be >= 1.".to_string());
    }
    if args.bucket_keep < 1 {
        return Err("--bucket-keep must be >= 1.".to_string());
    }
    if args.frontier_structure_keep < 1 {
        return Err("--frontier-structure-keep must be >= 1.".to_string());
    }
    if args.workers < 1 || args.workers > 256 {
        return Err("--workers must be in [1, 256].".to_string());
    }
    if !(2..=257).contains(&args.start_samples) {
        return Err("--start-samples must be in [2, 257].".to_string());
    }
    if args.entity_id_mods.is_empty() {
        return Err("--entity-id-mods must contain at least one value.".to_string());
    }
    if args.initial_tick_counts.is_empty() {
        return Err("--initial-tick-counts must contain at least one value.".to_string());
    }
    if args.entity_id_mods.iter().any(|value| *value >= MOVEMENT_SAMPLE_MODULO) {
        return Err("--entity-id-mods values must be in [0, 3].".to_string());
    }
    if let Some(values) = args.fixed_start_offsets.as_ref() {
        if values.is_empty() {
            return Err(
                "--fixed-start-offsets must contain at least one finite number.".to_string(),
            );
        }
    }
    if !args.start_y.is_finite() || !args.start_vx.is_finite() || !args.start_vy.is_finite() {
        return Err("--start-y, --start-vx, and --start-vy must be finite numbers.".to_string());
    }
    if !(0.0..=1.0).contains(&args.min_early_block_hit_rate) {
        return Err("--min-early-block-hit-rate must be in [0, 1].".to_string());
    }
    if args.full_cadence_pairs < 1 {
        return Err("--full-cadence-pairs must be >= 1.".to_string());
    }
    if !(args.full_cadence_tolerance >= 0.0 && args.full_cadence_tolerance.is_finite()) {
        return Err("--full-cadence-tolerance must be >= 0.".to_string());
    }
    if matches!(args.mode, Mode::Full) {
        args.ticks = args.ticks.max(5 + args.full_cadence_pairs * 2 + 4);
    }
    Ok(ParsedArgs::Run(args))
}

pub fn main_cli(argv: Vec<String>) -> Result<(), String> {
    if argv.first().is_some_and(|arg| arg == "serve-web") {
        return service::serve_web();
    }
    if argv
        .first()
        .is_some_and(|arg| arg == "reachable-candidates")
    {
        return reachable_candidates::main_cli(&argv[1..]);
    }
    if argv.first().is_some_and(|arg| arg == "candidate-eval") {
        return candidate_eval::main_cli(&argv[1..]);
    }
    if argv.first().is_some_and(|arg| arg == "pipeline") {
        return pipeline::main_cli(&argv[1..]);
    }
    if argv
        .first()
        .is_some_and(|arg| arg == "analyze-game-storage")
    {
        return storage_analysis::main_cli(&argv[1..]);
    }
    if argv.first().is_some_and(|arg| arg == "simulate-json") {
        return main_simulate_json();
    }
    if argv
        .first()
        .is_some_and(|arg| arg == "generate-legal-waterway")
    {
        return main_generate_legal_waterway(&argv[1..]);
    }

    let parsed = parse_args(&argv)?;
    match parsed {
        ParsedArgs::Help(text) => {
            println!("{}", text);
            Ok(())
        }
        ParsedArgs::Run(args) => {
            fs::create_dir_all(&args.out)
                .map_err(|error| format!("Failed to create output dir: {error}"))?;
            let payload = search(&args);
            let csv_path = args.out.join("launch-search-results.csv");
            let md_path = args.out.join("launch-search-summary.md");
            let json_path = args.out.join("launch-top-candidates.json");
            write_csv(&csv_path, &payload.results)?;
            write_summary(&md_path, &payload, &args)?;
            write_json(&json_path, &payload, &args)?;
            println!("Evaluated {} launch states.", payload.evaluated);
            println!("Kept {} candidates.", payload.results.len());
            println!("CSV: {}", csv_path.display());
            println!("Markdown: {}", md_path.display());
            println!("JSON: {}", json_path.display());
            println!();
            let rows = &payload.results[..payload.results.len().min(args.top.min(20))];
            println!(
                "{}",
                if matches!(args.mode, Mode::Early) {
                    markdown_early_table(rows)
                } else {
                    markdown_table(rows)
                }
            );
            Ok(())
        }
    }
}

fn main_simulate_json() -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("Failed to read stdin: {error}"))?;
    let request: JsonSimulationRequest = serde_json::from_str(&input)
        .map_err(|error| format!("Invalid simulation JSON: {error}"))?;

    let mut structures = Vec::new();
    if let Some(structure) = request.structure {
        structures.push(structure);
    }
    structures.extend(request.structures);
    if structures.is_empty() {
        return Err("Simulation request must include structure or structures.".to_string());
    }

    let mut simulations = Vec::with_capacity(structures.len());
    for (index, structure) in structures.into_iter().enumerate() {
        let prefix = json_cells_to_cells(&structure.prefix)?;
        let cycle = json_cells_to_cells(&structure.cycle)?;
        let layout = Layout::new(&prefix, &cycle);
        let config = SimConfig {
            ticks: request.ticks,
            start_x: structure.start.x,
            start_y: structure.start.y,
            start_vx: structure.start.vx,
            start_vy: structure.start.vy,
            entity_id_mod4: structure.start.entity_id_mod4,
            initial_tick_count: structure.start.initial_tick_count,
            start_on_ground: structure.start.start_on_ground,
        };
        let sim = simulate(&layout, &config);
        simulations.push(json_simulation_from_simulation(
            structure
                .name
                .unwrap_or_else(|| format!("structure-{index}")),
            sim,
        ));
    }

    let response = JsonSimulationResponse {
        ok: true,
        engine: "item-waterway-solver-rust",
        simulations,
    };
    let json = serde_json::to_string(&response)
        .map_err(|error| format!("Failed to encode simulation JSON: {error}"))?;
    println!("{json}");
    Ok(())
}

fn json_cells_to_cells(cells: &[JsonCell]) -> Result<Vec<Cell>, String> {
    cells.iter().map(json_cell_to_cell).collect()
}

fn json_cell_to_cell(cell: &JsonCell) -> Result<Cell, String> {
    let floor = parse_floor(cell.floor.as_deref().unwrap_or("normal"))?;
    let amount = cell
        .amount
        .unwrap_or_else(|| {
            cell.surface
                .map(|surface| (surface * 9.0).round() as u8)
                .unwrap_or(0)
        })
        .min(8);
    let surface = match (cell.surface, amount) {
        (Some(surface), _) => Some(surface),
        (None, 0) => None,
        (None, amount) => Some(amount as f64 / 9.0),
    };
    Ok(Cell {
        surface,
        flow: cell.flow.clamp(-1, 1),
        amount,
        floor,
    })
}

fn parse_floor(value: &str) -> Result<Floor, String> {
    match value {
        "normal" | "stone" | "glass" => Ok(Floor::Normal),
        "packed_ice" | "ice" | "frosted_ice" => Ok(Floor::PackedIce),
        "blue_ice" => Ok(Floor::BlueIce),
        "slime" | "slime_block" => Ok(Floor::Slime),
        other => Err(format!("Unsupported floor: {other}")),
    }
}

fn json_simulation_from_simulation(name: String, sim: Simulation) -> JsonSimulation {
    JsonSimulation {
        name,
        xs: sim.xs,
        ys: sim.ys,
        vxs: sim.vxs,
        vys: sim.vys,
        on_grounds: sim.on_grounds.into_iter().map(|value| value != 0).collect(),
        floors: sim
            .floors
            .into_iter()
            .map(|floor| floor.as_str().to_string())
            .collect(),
    }
}

fn parse_legal_waterway_args(argv: &[String]) -> Result<LegalWaterwayArgs, String> {
    let mut args = LegalWaterwayArgs {
        out: PathBuf::from("artifacts")
            .join("legal-waterway-generator")
            .join("legal-waterfields.json"),
        period: 9,
        limit: 200,
        max_sources: 4,
        workers: 8,
        debug: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--out" => {
                i += 1;
                args.out = PathBuf::from(
                    argv.get(i)
                        .ok_or_else(|| "--out requires a path.".to_string())?,
                );
            }
            "--period" => {
                i += 1;
                args.period = parse_usize(
                    argv.get(i)
                        .ok_or_else(|| "--period requires a value.".to_string())?,
                    "--period",
                )?;
            }
            "--limit" => {
                i += 1;
                args.limit = parse_usize(
                    argv.get(i)
                        .ok_or_else(|| "--limit requires a value.".to_string())?,
                    "--limit",
                )?;
            }
            "--max-sources" => {
                i += 1;
                args.max_sources = parse_usize(
                    argv.get(i)
                        .ok_or_else(|| "--max-sources requires a value.".to_string())?,
                    "--max-sources",
                )?;
            }
            "--workers" => {
                i += 1;
                args.workers = parse_usize(
                    argv.get(i)
                        .ok_or_else(|| "--workers requires a value.".to_string())?,
                    "--workers",
                )?;
            }
            "--debug" => args.debug = true,
            "--help" | "-h" => {
                return Err(
                    "Usage: item-waterway-solver generate-legal-waterway [--out <file>] [--period <n>] [--limit <n>] [--max-sources <n>] [--workers <n>] [--debug]"
                        .to_string(),
                );
            }
            other => return Err(format!("Unknown generate-legal-waterway argument: {other}")),
        }
        i += 1;
    }
    if !(2..=24).contains(&args.period) {
        return Err("--period must be in [2, 24].".to_string());
    }
    if args.limit == 0 {
        return Err("--limit must be at least 1.".to_string());
    }
    if args.max_sources == 0 || args.max_sources > args.period {
        return Err("--max-sources must be in [1, period].".to_string());
    }
    if args.workers == 0 || args.workers > 256 {
        return Err("--workers must be in [1, 256].".to_string());
    }
    Ok(args)
}

fn main_generate_legal_waterway(argv: &[String]) -> Result<(), String> {
    let args = parse_legal_waterway_args(argv)?;
    let payload = generate_legal_waterways(&args)?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("Failed to create output dir {}: {error}", parent.display())
        })?;
    }
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("Failed to encode legal waterway JSON: {error}"))?;
    fs::write(&args.out, format!("{json}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", args.out.display()))?;

    let result = LegalWaterwayCliResult {
        ok: true,
        engine: "rust-legal-waterway-generator",
        out_path: args.out.display().to_string(),
        period: args.period,
        workers: args.workers,
        evaluated_skeletons: payload.evaluated_skeletons,
        legal_waterfields: payload.legal_waterfields,
        deduped_rotations: payload.deduped_rotations,
    };
    let line = serde_json::to_string(&result)
        .map_err(|error| format!("Failed to encode legal waterway result: {error}"))?;
    println!("{line}");
    Ok(())
}

fn generate_legal_waterways(args: &LegalWaterwayArgs) -> Result<LegalWaterwayOutput, String> {
    let total = 3_usize
        .checked_pow(args.period as u32)
        .ok_or_else(|| "Legal waterway skeleton space overflowed usize.".to_string())?;
    let worker_count = args.workers.min(total.max(1));
    let next_ordinal = Arc::new(AtomicUsize::new(0));
    let evaluated_skeletons = Arc::new(AtomicUsize::new(0));
    let deduped_rotations = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(HashSet::<String>::new()));
    let kept = Arc::new(Mutex::new(Vec::<LegalWaterFieldOutput>::new()));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let next_ordinal = Arc::clone(&next_ordinal);
            let evaluated_skeletons = Arc::clone(&evaluated_skeletons);
            let deduped_rotations = Arc::clone(&deduped_rotations);
            let seen = Arc::clone(&seen);
            let kept = Arc::clone(&kept);
            scope.spawn(move || loop {
                let ordinal = next_ordinal.fetch_add(1, Ordering::Relaxed);
                if ordinal >= total {
                    break;
                }
                let skeleton = legal_skeleton_from_ordinal(ordinal, args.period);
                let source_count = skeleton
                    .iter()
                    .filter(|&&cell| cell == LegalWaterCell::Source)
                    .count();
                if source_count == 0 || source_count > args.max_sources {
                    continue;
                }
                evaluated_skeletons.fetch_add(1, Ordering::Relaxed);
                let Some(stable) = stabilize_waterfield(&skeleton) else {
                    continue;
                };
                let key = canonical_waterfield_key(&stable);
                {
                    let mut seen_guard = seen.lock().expect("seen mutex poisoned");
                    if !seen_guard.insert(key) {
                        deduped_rotations.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                }
                let candidate = legal_waterfield_output(&stable);
                let mut kept_guard = kept.lock().expect("kept mutex poisoned");
                kept_guard.push(candidate);
            });
        }
    });

    let mut kept_guard = kept.lock().map_err(|_| "kept mutex poisoned".to_string())?;
    let mut waterfields = std::mem::take(&mut *kept_guard);
    waterfields.sort_by(|left, right| {
        right
            .search_score
            .cmp(&left.search_score)
            .then_with(|| left.amounts.cmp(&right.amounts))
            .then_with(|| left.id.cmp(&right.id))
    });
    waterfields.truncate(args.limit);

    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("Failed to format timestamp: {error}"))?;
    Ok(LegalWaterwayOutput {
        ok: true,
        engine: "rust-legal-waterway-generator",
        generated_at,
        args: LegalWaterwayArgsOutput {
            period: args.period,
            limit: args.limit,
            max_sources: args.max_sources,
            workers: args.workers,
            debug: args.debug,
            blocker: "stone_pressure_plate",
            note: "Experimental generator: enum source/open/stone-pressure-plate skeletons, solve Minecraft-like 1D water fixed point, then derive flow from stabilized water heights.",
        },
        evaluated_skeletons: evaluated_skeletons.load(Ordering::Relaxed),
        legal_waterfields: waterfields.len(),
        deduped_rotations: deduped_rotations.load(Ordering::Relaxed),
        waterfields,
    })
}

fn legal_skeleton_from_ordinal(mut ordinal: usize, period: usize) -> Vec<LegalWaterCell> {
    let mut out = Vec::with_capacity(period);
    for _ in 0..period {
        out.push(match ordinal % 3 {
            0 => LegalWaterCell::Plate,
            1 => LegalWaterCell::Open,
            _ => LegalWaterCell::Source,
        });
        ordinal /= 3;
    }
    out
}

fn stabilize_waterfield(skeleton: &[LegalWaterCell]) -> Option<StableWaterField> {
    let period = skeleton.len();
    if period == 0 || !skeleton.contains(&LegalWaterCell::Source) {
        return None;
    }
    let mut sources = skeleton
        .iter()
        .map(|&cell| cell == LegalWaterCell::Source)
        .collect::<Vec<_>>();
    let mut amounts = sources
        .iter()
        .map(|&source| if source { 8 } else { 0 })
        .collect::<Vec<u8>>();
    let max_iterations = period * 8 + 16;

    for iteration in 0..=max_iterations {
        let mut next_sources = sources.clone();
        let mut next_amounts = amounts.clone();
        for index in 0..period {
            if skeleton[index] == LegalWaterCell::Plate {
                next_sources[index] = false;
                next_amounts[index] = 0;
                continue;
            }
            let left = (index + period - 1) % period;
            let right = (index + 1) % period;
            let source_neighbors = usize::from(sources[left]) + usize::from(sources[right]);
            if sources[index] || source_neighbors >= 2 {
                next_sources[index] = true;
                next_amounts[index] = 8;
            } else {
                let amount = amounts[left].max(amounts[right]).saturating_sub(1);
                next_sources[index] = false;
                next_amounts[index] = amount;
            }
        }
        if next_sources == sources && next_amounts == amounts {
            if skeleton
                .iter()
                .enumerate()
                .any(|(index, &cell)| cell == LegalWaterCell::Open && amounts[index] == 0)
            {
                return None;
            }
            let flows = derive_waterfield_flows(&amounts);
            return Some(StableWaterField {
                skeleton: skeleton.to_vec(),
                amounts,
                sources,
                flows,
                iterations: iteration,
            });
        }
        sources = next_sources;
        amounts = next_amounts;
    }
    None
}

fn derive_waterfield_flows(amounts: &[u8]) -> Vec<i8> {
    let period = amounts.len();
    (0..period)
        .map(|index| {
            if amounts[index] == 0 {
                return 0;
            }
            let own_height = amounts[index] as f64 / 9.0;
            let left = (index + period - 1) % period;
            let right = (index + 1) % period;
            let mut horizontal = 0.0;
            if amounts[left] > 0 {
                horizontal += -1.0 * (own_height - amounts[left] as f64 / 9.0);
            }
            if amounts[right] > 0 {
                horizontal += own_height - amounts[right] as f64 / 9.0;
            }
            if horizontal > 1.0e-12 {
                1
            } else if horizontal < -1.0e-12 {
                -1
            } else {
                0
            }
        })
        .collect()
}

fn canonical_waterfield_key(field: &StableWaterField) -> String {
    let tokens = waterfield_tokens(field);
    (0..tokens.len())
        .map(|offset| {
            (0..tokens.len())
                .map(|index| tokens[(index + offset) % tokens.len()].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .min()
        .unwrap_or_default()
}

fn waterfield_tokens(field: &StableWaterField) -> Vec<String> {
    field
        .amounts
        .iter()
        .enumerate()
        .map(|(index, &amount)| {
            if field.skeleton[index] == LegalWaterCell::Plate {
                "P".to_string()
            } else if field.sources[index] {
                format!("S{}", flow_code(field.flows[index]))
            } else {
                format!("{}{}", amount, flow_code(field.flows[index]))
            }
        })
        .collect()
}

fn flow_code(flow: i8) -> char {
    match flow {
        value if value < 0 => '<',
        value if value > 0 => '>',
        _ => '=',
    }
}

fn circular_longest_run<T>(values: &[T], predicate: impl Fn(&T) -> bool) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut best = 0_usize;
    let mut current = 0_usize;
    for index in 0..values.len() * 2 {
        if predicate(&values[index % values.len()]) {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best.min(values.len())
}

fn legal_waterfield_search_score(field: &StableWaterField) -> i64 {
    let wet_count = field.amounts.iter().filter(|&&amount| amount > 0).count() as i64;
    let source_count = field.sources.iter().filter(|&&value| value).count() as i64;
    let plate_count = field
        .skeleton
        .iter()
        .filter(|&&cell| cell == LegalWaterCell::Plate)
        .count() as i64;
    let forward_count = field
        .amounts
        .iter()
        .zip(field.flows.iter())
        .filter(|(amount, flow)| **amount > 0 && **flow > 0)
        .count() as i64;
    let reverse_count = field
        .amounts
        .iter()
        .zip(field.flows.iter())
        .filter(|(amount, flow)| **amount > 0 && **flow < 0)
        .count() as i64;
    let still_source_count = field
        .sources
        .iter()
        .zip(field.flows.iter())
        .filter(|(is_source, flow)| **is_source && **flow == 0)
        .count() as i64;
    let max_wet_run = circular_longest_run(&field.amounts, |amount| *amount > 0) as i64;
    let max_forward_run = circular_longest_run(&field.flows, |flow| *flow > 0) as i64;
    let directional_segments = (0..field.flows.len())
        .filter(|&index| {
            let current = field.amounts[index] > 0 && field.flows[index] > 0;
            let prev = (index + field.flows.len() - 1) % field.flows.len();
            let previous = field.amounts[prev] > 0 && field.flows[prev] > 0;
            current && !previous
        })
        .count() as i64;
    forward_count * 20 + still_source_count * 14 + directional_segments * 10
        - reverse_count * 25
        - (plate_count - 3).abs() * 8
        - (source_count - 4).abs() * 6
        - (wet_count - 6).abs() * 5
        - (max_wet_run - 5).max(0) * 12
        + max_forward_run.min(3) * 6
}

fn legal_waterfield_output(field: &StableWaterField) -> LegalWaterFieldOutput {
    let skeleton = field
        .skeleton
        .iter()
        .map(|cell| match cell {
            LegalWaterCell::Plate => 'P',
            LegalWaterCell::Open => 'O',
            LegalWaterCell::Source => 'S',
        })
        .collect::<String>();
    let amounts = field
        .amounts
        .iter()
        .map(|amount| {
            if *amount == 0 {
                'P'
            } else {
                char::from_digit(*amount as u32, 10).unwrap_or('?')
            }
        })
        .collect::<String>();
    let flows = field.flows.iter().map(|&flow| flow_code(flow)).collect();
    let source_count = field.sources.iter().filter(|&&value| value).count();
    let plate_count = field
        .skeleton
        .iter()
        .filter(|&&cell| cell == LegalWaterCell::Plate)
        .count();
    let cells = field
        .amounts
        .iter()
        .enumerate()
        .map(|(index, &amount)| {
            let is_plate = field.skeleton[index] == LegalWaterCell::Plate;
            let is_source = field.sources[index];
            LegalWaterCellOutput {
                index,
                upper: if is_plate {
                    "stone_pressure_plate"
                } else if is_source {
                    "water_source"
                } else {
                    "flowing_water"
                },
                amount,
                surface: (amount > 0).then_some(amount as f64 / 9.0),
                flow: field.flows[index],
                is_source,
                blocker: is_plate.then_some("stone_pressure_plate"),
            }
        })
        .collect::<Vec<_>>();
    let id = stable_id(&format!("{skeleton}|{amounts}|{flows}"));
    LegalWaterFieldOutput {
        id,
        period: field.skeleton.len(),
        skeleton,
        amounts,
        flows,
        search_score: legal_waterfield_search_score(field),
        source_count,
        plate_count,
        iterations: field.iterations,
        cells,
    }
}

fn parse_usize(text: &str, flag: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|_| format!("{} must be a non-negative integer.", flag))
}

fn parse_f64(text: &str, flag: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .map_err(|_| format!("{} must be a finite number.", flag))
        .and_then(|value| {
            if value.is_finite() {
                Ok(value)
            } else {
                Err(format!("{} must be a finite number.", flag))
            }
        })
}

fn parse_number_list(text: &str) -> Vec<f64> {
    text.split(',')
        .filter_map(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .collect()
}

fn parse_string_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_usize_list(text: &str, flag: &str) -> Result<Vec<usize>, String> {
    let mut values = Vec::new();
    for value in text.split(',').map(str::trim).filter(|value| !value.is_empty()) {
        values.push(parse_usize(value, flag)?);
    }
    if values.is_empty() {
        return Err(format!("{flag} must contain at least one integer."));
    }
    Ok(values)
}

fn cell(surface: Option<f64>, flow: i8, floor: Floor, amount: Option<u8>) -> Cell {
    let fluid_amount = surface
        .map(|value| amount.unwrap_or_else(|| (value * 9.0).round() as u8))
        .unwrap_or(0);
    Cell {
        surface,
        flow,
        amount: fluid_amount,
        floor,
    }
}

fn dry_gap(length: usize, floor_pattern: &[Floor]) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(length);
    for index in 0..length {
        cells.push(cell(
            None,
            0,
            floor_pattern[index % floor_pattern.len()],
            None,
        ));
    }
    cells
}

fn still_water(length: usize, floor_pattern: &[Floor], surface: f64) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(length);
    for index in 0..length {
        cells.push(cell(
            Some(surface),
            0,
            floor_pattern[index % floor_pattern.len()],
            Some(8),
        ));
    }
    cells
}

fn one_way_water(
    length: usize,
    direction: i8,
    floor_pattern: &[Floor],
    full_height: bool,
) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(length);
    for index in 0..length {
        let distance_from_source = if direction == 1 {
            index
        } else {
            length - 1 - index
        };
        let amount = (8_i32 - distance_from_source as i32).max(1) as u8;
        let surface = if full_height {
            1.0
        } else {
            amount as f64 / 9.0
        };
        cells.push(cell(
            Some(surface),
            direction,
            floor_pattern[index % floor_pattern.len()],
            Some(amount),
        ));
    }
    cells
}

fn centered_source_faststart(floor_pattern: &[Floor]) -> Vec<Cell> {
    [
        cell(Some(7.0 / 9.0), -1, floor_pattern[0 % floor_pattern.len()], Some(7)),
        cell(Some(8.0 / 9.0), 0, floor_pattern[1 % floor_pattern.len()], Some(8)),
        cell(Some(7.0 / 9.0), 1, floor_pattern[2 % floor_pattern.len()], Some(7)),
        cell(Some(6.0 / 9.0), 1, floor_pattern[3 % floor_pattern.len()], Some(6)),
    ]
    .into_iter()
    .collect()
}

fn cycle_definition(
    name: impl Into<String>,
    cells: Vec<Cell>,
    note: impl Into<String>,
    proven: bool,
) -> CycleSpec {
    let cells_signature = cells_signature(&cells);
    CycleSpec {
        name: name.into(),
        cells,
        note: note.into(),
        proven,
        signature: cells_signature,
    }
}

pub fn backbone_cycles() -> Vec<CycleSpec> {
    let mut cycles = vec![
        cycle_definition(
            "W3-I_D3-B",
            [
                one_way_water(3, 1, &[Floor::PackedIce], false),
                dry_gap(3, &[Floor::BlueIce]),
            ]
            .concat(),
            "Proven long-run backbone: 3 forward water cells on packed ice, 3 dry cells on blue ice.",
            true,
        ),
        cycle_definition(
            "W2-I_D2-B",
            [
                one_way_water(2, 1, &[Floor::PackedIce], false),
                dry_gap(2, &[Floor::BlueIce]),
            ]
            .concat(),
            "Dry-gap compact variant from source model search; needs game verification.",
            false,
        ),
        cycle_definition(
            "W2-I_S2-I",
            [
                one_way_water(2, 1, &[Floor::PackedIce], false),
                still_water(2, &[Floor::PackedIce], 8.0 / 9.0),
            ]
            .concat(),
            "Still-source compact variant; build with intentional source-water cells only.",
            false,
        ),
        cycle_definition(
            "W2-B_S2-B",
            [
                one_way_water(2, 1, &[Floor::BlueIce], false),
                still_water(2, &[Floor::BlueIce], 8.0 / 9.0),
            ]
            .concat(),
            "Still-source compact blue-ice variant; build with intentional source-water cells only.",
            false,
        ),
        cycle_definition(
            "W2-I_R2-I_D1-B",
            [
                one_way_water(2, 1, &[Floor::PackedIce], false),
                one_way_water(2, -1, &[Floor::PackedIce], false),
                dry_gap(1, &[Floor::BlueIce]),
            ]
            .concat(),
            "Reverse-water compact variant with a real two-cell water gradient; needs game verification.",
            false,
        ),
        cycle_definition(
            "D1-I_F2-I_D1-B_S1-B_F2-I_D1-B_S1-B",
            [
                dry_gap(1, &[Floor::PackedIce]),
                one_way_water(2, 1, &[Floor::PackedIce], false),
                dry_gap(1, &[Floor::BlueIce]),
                still_water(1, &[Floor::BlueIce], 8.0 / 9.0),
                one_way_water(2, 1, &[Floor::PackedIce], false),
                dry_gap(1, &[Floor::BlueIce]),
                still_water(1, &[Floor::BlueIce], 8.0 / 9.0),
            ]
            .concat(),
            "Game-validated faststart bridge backbone from game storage run 2.",
            true,
        ),
        cycle_definition(
            "F2-I_D1-B_S1-B_D1-I_F2-I_D1-B_S1-B",
            [
                one_way_water(2, 1, &[Floor::PackedIce], false),
                dry_gap(1, &[Floor::BlueIce]),
                still_water(1, &[Floor::BlueIce], 8.0 / 9.0),
                dry_gap(1, &[Floor::PackedIce]),
                one_way_water(2, 1, &[Floor::PackedIce], false),
                dry_gap(1, &[Floor::BlueIce]),
                still_water(1, &[Floor::BlueIce], 8.0 / 9.0),
            ]
            .concat(),
            "Cyclic rotation of the game-validated faststart bridge backbone.",
            true,
        ),
    ];

    let water_floor_sets: [(&str, &[Floor]); 4] = [
        ("I", &[Floor::PackedIce]),
        ("B", &[Floor::BlueIce]),
        ("IB", &[Floor::PackedIce, Floor::BlueIce]),
        ("BI", &[Floor::BlueIce, Floor::PackedIce]),
    ];
    let gap_floor_sets = water_floor_sets;

    for (water_name, water_floors) in water_floor_sets {
        for (gap_name, gap_floors) in gap_floor_sets {
            cycles.push(cycle_definition(
                format!("W2-{}_D2-{}", water_name, gap_name),
                [
                    one_way_water(2, 1, water_floors, false),
                    dry_gap(2, gap_floors),
                ]
                .concat(),
                "Generated compact dry-gap variant; needs game verification.",
                false,
            ));
            cycles.push(cycle_definition(
                format!("W2-{}_S2-{}", water_name, gap_name),
                [one_way_water(2, 1, water_floors, false), still_water(2, gap_floors, 8.0 / 9.0)].concat(),
                "Generated compact source/still-water variant; build with intentional source-water cells only.",
                false,
            ));
        }
    }

    cycles
}

fn prefix_atoms() -> Vec<PrefixAtom> {
    vec![
        PrefixAtom {
            name: "DN",
            cells: dry_gap(1, &[Floor::Normal]),
        },
        PrefixAtom {
            name: "DI",
            cells: dry_gap(1, &[Floor::PackedIce]),
        },
        PrefixAtom {
            name: "DB",
            cells: dry_gap(1, &[Floor::BlueIce]),
        },
        PrefixAtom {
            name: "D2B",
            cells: dry_gap(2, &[Floor::BlueIce]),
        },
        PrefixAtom {
            name: "DS",
            cells: dry_gap(1, &[Floor::Slime]),
        },
        PrefixAtom {
            name: "SN",
            cells: still_water(1, &[Floor::Normal], 8.0 / 9.0),
        },
        PrefixAtom {
            name: "SI",
            cells: still_water(1, &[Floor::PackedIce], 8.0 / 9.0),
        },
        PrefixAtom {
            name: "SB",
            cells: still_water(1, &[Floor::BlueIce], 8.0 / 9.0),
        },
        PrefixAtom {
            name: "R2N",
            cells: one_way_water(2, -1, &[Floor::Normal], false),
        },
        PrefixAtom {
            name: "R2I",
            cells: one_way_water(2, -1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "R2B",
            cells: one_way_water(2, -1, &[Floor::BlueIce], false),
        },
        PrefixAtom {
            name: "R3N",
            cells: one_way_water(3, -1, &[Floor::Normal], false),
        },
        PrefixAtom {
            name: "R3I",
            cells: one_way_water(3, -1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "R3B",
            cells: one_way_water(3, -1, &[Floor::BlueIce], false),
        },
        PrefixAtom {
            name: "F2N",
            cells: one_way_water(2, 1, &[Floor::Normal], false),
        },
        PrefixAtom {
            name: "F2I",
            cells: one_way_water(2, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F3I",
            cells: one_way_water(3, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F5I",
            cells: one_way_water(5, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "FS4I",
            cells: centered_source_faststart(&[Floor::PackedIce]),
        },
        PrefixAtom {
            name: "F2B",
            cells: one_way_water(2, 1, &[Floor::BlueIce], false),
        },
    ]
}

fn prefix_label_atoms() -> Vec<PrefixAtom> {
    let mut atoms = prefix_atoms();
    atoms.extend([
        PrefixAtom {
            name: "F3I",
            cells: one_way_water(3, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F3B",
            cells: one_way_water(3, 1, &[Floor::BlueIce], false),
        },
        PrefixAtom {
            name: "F4I",
            cells: one_way_water(4, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F4B",
            cells: one_way_water(4, 1, &[Floor::BlueIce], false),
        },
        PrefixAtom {
            name: "F5I",
            cells: one_way_water(5, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F5B",
            cells: one_way_water(5, 1, &[Floor::BlueIce], false),
        },
        PrefixAtom {
            name: "D2B",
            cells: dry_gap(2, &[Floor::BlueIce]),
        },
        PrefixAtom {
            name: "F2I-stab",
            cells: one_way_water(2, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "F3I-stab",
            cells: one_way_water(3, 1, &[Floor::PackedIce], false),
        },
        PrefixAtom {
            name: "DB-stab",
            cells: dry_gap(1, &[Floor::BlueIce]),
        },
    ]);
    atoms
}

fn parse_prefix_label_tokens(label: &str, known_names: &[&str]) -> Result<Vec<String>, String> {
    if label == "none" {
        return Ok(Vec::new());
    }
    let mut names = known_names.to_vec();
    names.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));

    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < label.len() {
        let remaining = &label[offset..];
        let Some(name) = names.iter().copied().find(|name| {
            remaining.starts_with(name)
                && (remaining.len() == name.len()
                    || remaining.as_bytes().get(name.len()) == Some(&b'-'))
        }) else {
            return Err(format!("Unknown prefix atom near '{}'.", remaining));
        };
        tokens.push(name.to_string());
        offset += name.len();
        if offset < label.len() {
            if label.as_bytes().get(offset) != Some(&b'-') {
                return Err(format!("Malformed prefix label '{}'.", label));
            }
            offset += 1;
        }
    }
    Ok(tokens)
}

fn parse_prefix_atoms_from_label(label: &str) -> Result<Vec<PrefixAtom>, String> {
    let atoms = prefix_label_atoms();
    let known_names = atoms.iter().map(|atom| atom.name).collect::<Vec<_>>();
    let tokens = parse_prefix_label_tokens(label, &known_names)?;
    tokens
        .into_iter()
        .map(|name| {
            atoms.iter()
                .find(|atom| atom.name == name)
                .cloned()
                .ok_or_else(|| format!("Unknown prefix atom {name}"))
        })
        .collect()
}

fn format_prefix_label(atoms: &[PrefixAtom]) -> String {
    if atoms.is_empty() {
        "none".to_string()
    } else {
        atoms
            .iter()
            .map(|atom| atom.name)
            .collect::<Vec<_>>()
            .join("-")
    }
}

fn prefix_spec_from_indices(indices: &[usize], atoms: &[PrefixAtom]) -> PrefixSpec {
    let label = format_prefix_label(
        &indices
            .iter()
            .map(|&index| atoms[index].clone())
            .collect::<Vec<_>>(),
    );
    let total_len = indices.iter().map(|&index| atoms[index].cells.len()).sum();
    let mut cells = Vec::with_capacity(total_len);
    for &index in indices {
        cells.extend_from_slice(&atoms[index].cells);
    }
    PrefixSpec {
        label,
        signature: cells_signature(&cells),
        cells,
    }
}

fn cells_signature(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(Cell::code)
        .collect::<Vec<_>>()
        .join(",")
}

fn layout_cell_index(
    prefix_length: usize,
    period: usize,
    total_length: usize,
    index: isize,
) -> Option<usize> {
    if total_length == 0 {
        return None;
    }
    if index < prefix_length as isize {
        return (index >= 0).then_some(index as usize);
    }
    if period == 0 {
        return None;
    }
    let cycle_offset = (index - prefix_length as isize).rem_euclid(period as isize) as usize;
    Some(prefix_length + cycle_offset)
}

fn compute_flow_direction(
    cells: &[Cell],
    prefix_length: usize,
    period: usize,
    total_length: usize,
    index: isize,
) -> i8 {
    let Some(current_index) = layout_cell_index(prefix_length, period, total_length, index) else {
        return 0;
    };
    let current = &cells[current_index];
    if current.amount == 0 {
        return 0;
    }

    let own_height = current.amount as f64 / 9.0;
    let mut horizontal = 0.0;
    for (neighbor_index, step_x) in [(index - 1, -1.0), (index + 1, 1.0)] {
        if let Some(resolved) =
            layout_cell_index(prefix_length, period, total_length, neighbor_index)
        {
            let neighbor = &cells[resolved];
            if neighbor.amount > 0 {
                horizontal += step_x * (own_height - neighbor.amount as f64 / 9.0);
            }
        }
    }

    if horizontal > 1.0e-12 {
        1
    } else if horizontal < -1.0e-12 {
        -1
    } else {
        0
    }
}

fn update_fluid_tracker(layout: &Layout, x: f64, y: f64, ignore_current: bool) -> FluidTracker {
    let half_width = WIDTH / 2.0;
    let box_min_x = x - half_width + AABB_DEFLATE;
    let box_max_x = x + half_width - AABB_DEFLATE;
    let box_min_y = y + AABB_DEFLATE;
    let box_max_y = y + HEIGHT - AABB_DEFLATE;
    let x0 = box_min_x.floor() as isize;
    let x1 = box_max_x.ceil() as isize - 1;
    let y0 = box_min_y.floor() as isize;
    let y1 = box_max_y.ceil() as isize - 1;
    let mut tracker = FluidTracker::default();

    for ix in x0..=x1 {
        let Some(current) = layout.cell_at(ix) else {
            continue;
        };
        let Some(surface) = current.surface else {
            continue;
        };
        for iy in y0..=y1 {
            if iy != 0 {
                continue;
            }
            let fluid_bottom = iy as f64;
            let fluid_top = fluid_bottom + surface;
            if fluid_top < box_min_y {
                continue;
            }
            tracker.height = tracker.height.max(fluid_top - y);
            if !ignore_current {
                let mut flow_x = layout.flow_direction_at(ix) as f64;
                if tracker.height < 0.4 {
                    flow_x *= tracker.height;
                }
                tracker.accumulated_current_x += flow_x;
                tracker.current_count += 1;
            }
        }
    }

    tracker
}

fn floor_at(layout: &Layout, x: f64) -> Floor {
    layout
        .cell_at(x.floor() as isize)
        .map(|cell| cell.floor)
        .unwrap_or(Floor::Normal)
}

fn apply_fluid_current(vx: f64, tracker: FluidTracker) -> f64 {
    if tracker.current_count == 0
        || tracker.accumulated_current_x * tracker.accumulated_current_x < FLUID_CURRENT_EPSILON2
    {
        return vx;
    }

    let direction = tracker.accumulated_current_x.signum();
    if direction == 0.0 {
        return vx;
    }

    let mut impulse = direction * WATER_PUSH;
    if vx.abs() < FLUID_CURRENT_MIN_OLD_MOVEMENT && impulse.abs() < FLUID_CURRENT_MIN_IMPULSE {
        impulse = direction * FLUID_CURRENT_MIN_IMPULSE;
    }
    vx + impulse
}
fn horizontal_drag(floor: Floor, on_ground: bool, vy: f64) -> f64 {
    let mut drag = HORIZONTAL_MOVEMENT_DAMPING;
    if on_ground {
        drag = java_f32(floor.friction() * HORIZONTAL_MOVEMENT_DAMPING);
        if matches!(floor.step_on(), StepOn::Slime) && vy.abs() < SLIME_STEP_ON_VY_THRESHOLD {
            drag *= SLIME_STEP_ON_BASE + SLIME_STEP_ON_VY_SCALE * vy.abs();
        }
    }
    drag
}

fn vertical_velocity_after_landing(floor: Floor, vy: f64) -> f64 {
    if matches!(floor.step_on(), StepOn::Slime) {
        if vy < 0.0 {
            -vy * 0.8
        } else {
            vy
        }
    } else {
        0.0
    }
}

fn simulate(layout: &Layout, config: &SimConfig) -> Simulation {
    let n = config.ticks + 1;
    let mut xs = vec![0.0; n];
    let mut ys = vec![0.0; n];
    let mut vxs = vec![0.0; n];
    let mut vys = vec![0.0; n];
    let mut on_grounds = vec![0_u8; n];
    let mut floors = vec![Floor::Normal; n];

    let mut x = config.start_x;
    let mut y = config.start_y;
    let mut vx = config.start_vx;
    let mut vy = config.start_vy;
    let mut was_on_ground = config.start_on_ground.unwrap_or(config.start_y <= 0.0);

    xs[0] = x;
    ys[0] = y;
    vxs[0] = vx;
    vys[0] = vy;
    on_grounds[0] = u8::from(was_on_ground);
    floors[0] = floor_at(layout, x);

    for tick in 1..=config.ticks {
        let tick_count = config.initial_tick_count + tick;
        let fluid_tracker = update_fluid_tracker(layout, x, y, false);
        if fluid_tracker.is_in_fluid() {
            vx = apply_fluid_current(vx, fluid_tracker);
        }
        if fluid_tracker.is_in_fluid() && fluid_tracker.applies_underwater_movement() {
            vx *= HORIZONTAL_WATER_DAMPING;
            if vy < BUOYANCY_CAP {
                vy += BUOYANCY;
            }
        } else {
            vy -= GRAVITY;
        }

        let phase_mod4 = (tick_count + config.entity_id_mod4) % MOVEMENT_SAMPLE_MODULO;
        let should_move = !was_on_ground || vx * vx > HORIZONTAL_REST_THRESHOLD2 || phase_mod4 == 0;
        let mut on_ground = was_on_ground;

        if should_move {
            x += vx;
            y += vy;

            on_ground = false;
            if y < 0.0 {
                on_ground = vy < 0.0;
                y = 0.0;
            }

            let floor = floor_at(layout, x);
            if on_ground {
                vy = vertical_velocity_after_landing(floor, vy);
            }
            vx *= horizontal_drag(floor, on_ground, vy);
            vy *= VERTICAL_MOVEMENT_DAMPING;
            if on_ground && vy < 0.0 {
                vy *= -0.5;
            }
        }

        let post_fluid_tracker = update_fluid_tracker(layout, x, y, false);
        if post_fluid_tracker.is_in_fluid() {
            vx = apply_fluid_current(vx, post_fluid_tracker);
        }

        xs[tick] = x;
        ys[tick] = y;
        vxs[tick] = vx;
        vys[tick] = vy;
        on_grounds[tick] = u8::from(on_ground);
        floors[tick] = floor_at(layout, x);
        was_on_ground = on_ground;
    }

    Simulation {
        xs,
        ys,
        vxs,
        vys,
        on_grounds,
        floors,
    }
}

fn window_metric_context(sim: &Simulation) -> WindowMetricContext {
    let mut vx_sum = vec![0.0; sim.vxs.len() + 1];
    let mut vx_sq_sum = vec![0.0; sim.vxs.len() + 1];

    for index in 0..sim.vxs.len() {
        vx_sum[index + 1] = vx_sum[index] + sim.vxs[index];
        vx_sq_sum[index + 1] = vx_sq_sum[index] + sim.vxs[index] * sim.vxs[index];
    }

    WindowMetricContext { vx_sum, vx_sq_sum }
}

fn range_sum(prefix: &[f64], start_inclusive: usize, end_exclusive: usize) -> f64 {
    prefix[end_exclusive] - prefix[start_inclusive]
}

fn window_metrics(
    sim: &Simulation,
    start_tick: usize,
    window_length: usize,
    context: &WindowMetricContext,
) -> Option<WindowMetrics> {
    let state_start = start_tick + 1;
    let state_end = start_tick + window_length + 1;
    if state_end > sim.xs.len() {
        return None;
    }

    let count = window_length as f64;
    let avg_vx = range_sum(&context.vx_sum, state_start, state_end) / count;
    let vx_var =
        (range_sum(&context.vx_sq_sum, state_start, state_end) / count - avg_vx * avg_vx).max(0.0);
    Some(WindowMetrics {
        average_vx: avg_vx,
        mean_vx_error: (avg_vx - 0.5).abs(),
        std_vx: vx_var.sqrt(),
        average_distance_vx: (sim.xs[start_tick + window_length] - sim.xs[start_tick]) / count,
        long_window_score: None,
        long_window_start_tick: None,
        suffix_start_tick: None,
    })
}

fn cadence_metrics(
    sim: &Simulation,
    start_tick: usize,
    pair_count: usize,
    tolerance: f64,
) -> Option<EarlyCadence> {
    let mut max_abs_distance_error: f64 = 0.0;
    let mut mean_abs_distance_error = 0.0;
    let mut mean_signed_distance_error = 0.0;
    let mut block_hits = 0_usize;
    let mut within_tol = 0_usize;
    let mut samples = Vec::with_capacity(pair_count.min(12));

    for pair in 0..pair_count {
        let t0 = start_tick + pair * 2;
        let t1 = t0 + 2;
        if t1 >= sim.xs.len() {
            return None;
        }
        let distance = sim.xs[t1] - sim.xs[t0];
        let distance_error = distance - 1.0;
        let abs_error = distance_error.abs();
        let floor_delta = sim.xs[t1].floor() as i32 - sim.xs[t0].floor() as i32;
        max_abs_distance_error = max_abs_distance_error.max(abs_error);
        mean_abs_distance_error += abs_error;
        mean_signed_distance_error += distance_error;
        if floor_delta == 1 {
            block_hits += 1;
        }
        if abs_error <= tolerance {
            within_tol += 1;
        }
        if pair < 12 {
            samples.push(EarlyCadenceSample {
                pair,
                t0,
                t1,
                x0: sim.xs[t0],
                x1: sim.xs[t1],
                distance,
                distance_error,
                floor_delta,
            });
        }
    }

    Some(EarlyCadence {
        cadence_start_tick: start_tick,
        cadence_pairs: pair_count,
        cadence_mean_abs_distance_error: mean_abs_distance_error / pair_count as f64,
        cadence_mean_signed_distance_error: mean_signed_distance_error / pair_count as f64,
        cadence_max_abs_distance_error: max_abs_distance_error,
        cadence_block_hit_rate: block_hits as f64 / pair_count as f64,
        cadence_within_tolerance_rate: within_tol as f64 / pair_count as f64,
        cadence_pass: within_tol == pair_count && block_hits == pair_count,
        cadence_samples: samples,
        early_cadence_score: 0.0,
    })
}

fn distance_to_integer_boundary(value: f64) -> f64 {
    let fraction = value - value.floor();
    fraction.min(1.0 - fraction)
}

fn full_cadence_metrics(
    sim: &Simulation,
    start_tick: usize,
    pair_count: usize,
    tolerance: f64,
) -> Option<FullCadence> {
    let mut max_abs_distance_error: f64 = 0.0;
    let mut mean_abs_distance_error = 0.0;
    let mut mean_signed_distance_error = 0.0;
    let mut block_hits = 0_usize;
    let mut within_tol = 0_usize;
    let mut first_miss = None;
    let mut longest_hit_run = 0_usize;
    let mut current_hit_run = 0_usize;
    let mut min_hit_margin = f64::INFINITY;
    let mut mean_hit_margin = 0.0;
    let mut min_endpoint_boundary_margin = f64::INFINITY;
    let mut mean_endpoint_boundary_margin = 0.0;
    let mut samples = Vec::with_capacity(24);

    for pair in 0..pair_count {
        let t0 = start_tick + pair * 2;
        let t1 = t0 + 2;
        if t1 >= sim.xs.len() {
            return None;
        }

        let distance = sim.xs[t1] - sim.xs[t0];
        let distance_error = distance - 1.0;
        let abs_error = distance_error.abs();
        let floor0 = sim.xs[t0].floor() as i32;
        let floor_delta = sim.xs[t1].floor() as i32 - floor0;
        let hit_margin = (sim.xs[t1] - (floor0 + 1) as f64).min((floor0 + 2) as f64 - sim.xs[t1]);
        let endpoint_boundary_margin =
            distance_to_integer_boundary(sim.xs[t0]).min(distance_to_integer_boundary(sim.xs[t1]));
        let hit = floor_delta == 1;

        max_abs_distance_error = max_abs_distance_error.max(abs_error);
        mean_abs_distance_error += abs_error;
        mean_signed_distance_error += distance_error;
        min_hit_margin = min_hit_margin.min(hit_margin);
        mean_hit_margin += hit_margin;
        min_endpoint_boundary_margin = min_endpoint_boundary_margin.min(endpoint_boundary_margin);
        mean_endpoint_boundary_margin += endpoint_boundary_margin;

        if hit {
            block_hits += 1;
            current_hit_run += 1;
            longest_hit_run = longest_hit_run.max(current_hit_run);
        } else {
            current_hit_run = 0;
            if first_miss.is_none() {
                first_miss = Some(EarlyCadenceSample {
                    pair,
                    t0,
                    t1,
                    x0: sim.xs[t0],
                    x1: sim.xs[t1],
                    distance,
                    distance_error,
                    floor_delta,
                });
            }
        }

        if abs_error <= tolerance {
            within_tol += 1;
        }
        if pair < 12 || (!hit && samples.len() < 24) {
            samples.push(FullCadenceSample {
                pair,
                t0,
                t1,
                x0: sim.xs[t0],
                x1: sim.xs[t1],
                distance,
                distance_error,
                floor_delta,
                hit_margin,
                endpoint_boundary_margin,
            });
        }
    }

    Some(FullCadence {
        full_cadence_start_tick: start_tick,
        full_cadence_pairs: pair_count,
        full_cadence_mean_abs_distance_error: mean_abs_distance_error / pair_count as f64,
        full_cadence_mean_signed_distance_error: mean_signed_distance_error / pair_count as f64,
        full_cadence_max_abs_distance_error: max_abs_distance_error,
        full_cadence_block_hit_rate: block_hits as f64 / pair_count as f64,
        full_cadence_within_tolerance_rate: within_tol as f64 / pair_count as f64,
        full_cadence_longest_hit_run: longest_hit_run,
        full_cadence_first_miss: first_miss,
        full_cadence_min_hit_margin: min_hit_margin,
        full_cadence_mean_hit_margin: mean_hit_margin / pair_count as f64,
        full_cadence_min_endpoint_boundary_margin: min_endpoint_boundary_margin,
        full_cadence_mean_endpoint_boundary_margin: mean_endpoint_boundary_margin
            / pair_count as f64,
        full_cadence_samples: samples,
        full_cadence_distance: sim.xs[start_tick + pair_count * 2] - sim.xs[start_tick],
        full_cadence_average_speed: (sim.xs[start_tick + pair_count * 2] - sim.xs[start_tick])
            / (pair_count * 2) as f64,
    })
}

fn best_early_cadence(
    sim: &Simulation,
    max_start_tick: usize,
    pair_count: usize,
    tolerance: f64,
) -> Option<EarlyCadence> {
    let mut best = None;
    let mut best_score = f64::INFINITY;
    for start_tick in 0..=max_start_tick {
        let Some(mut metrics) = cadence_metrics(sim, start_tick, pair_count, tolerance) else {
            continue;
        };
        let score = metrics.cadence_mean_abs_distance_error * 1000.0
            + metrics.cadence_max_abs_distance_error * 100.0
            + (1.0 - metrics.cadence_block_hit_rate) * 500.0
            + start_tick as f64 * 2.0;
        if score < best_score {
            best_score = score;
            metrics.early_cadence_score = score;
            best = Some(metrics);
        }
    }
    best
}

fn best_long_window(sim: &Simulation, window_length: usize) -> Option<WindowMetrics> {
    let mut best = None;
    let mut best_score = f64::INFINITY;
    let context = window_metric_context(sim);
    for start_tick in 0..sim.xs.len() {
        let Some(mut metrics) = window_metrics(sim, start_tick, window_length, &context) else {
            continue;
        };
        let score = metrics.mean_vx_error * 1000.0
            + metrics.std_vx * 20.0
            + (metrics.average_distance_vx - 0.5).abs() * 1000.0;
        if score < best_score {
            best_score = score;
            metrics.long_window_score = Some(score);
            metrics.long_window_start_tick = Some(start_tick);
            best = Some(metrics);
        }
    }
    best
}

fn suffix_long_metrics(
    sim: &Simulation,
    start_tick: usize,
    window_length: usize,
) -> Option<WindowMetrics> {
    let safe_start = start_tick.max(5);
    if safe_start + window_length >= sim.xs.len() {
        return None;
    }
    let context = window_metric_context(sim);
    let mut metrics = window_metrics(sim, safe_start, window_length, &context)?;
    metrics.suffix_start_tick = Some(safe_start);
    Some(metrics)
}

fn stable_id(text: &str) -> String {
    let mut hash = 2_166_136_261_u32;
    for byte in text.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{:08x}", hash)
}

pub fn search(args: &Args) -> SearchPayload {
    reachability::run(args)
}

fn csv_escape(value: &str) -> String {
    if value
        .chars()
        .any(|ch| matches!(ch, '"' | ',' | '\r' | '\n'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn write_csv(path: &Path, rows: &[ResultRow]) -> Result<(), String> {
    let columns = [
        "rank",
        "id",
        "pass",
        "score",
        "backbone",
        "proven",
        "prefixLabel",
        "prefixLength",
        "period",
        "startOffset",
        "entityIdMod4",
        "cadenceStartTick",
        "cadencePairs",
        "cadenceMeanAbsDistanceError",
        "cadenceMaxAbsDistanceError",
        "cadenceBlockHitRate",
        "cadenceWithinToleranceRate",
        "fullCadenceStartTick",
        "fullCadencePairs",
        "fullCadenceMeanAbsDistanceError",
        "fullCadenceMeanSignedDistanceError",
        "fullCadenceMaxAbsDistanceError",
        "fullCadenceBlockHitRate",
        "fullCadenceWithinToleranceRate",
        "fullCadenceLongestHitRun",
        "fullCadenceAverageSpeed",
        "fullCadenceDistance",
        "fullCadenceMinHitMargin",
        "fullCadenceMeanHitMargin",
        "fullCadenceMinEndpointBoundaryMargin",
        "fullCadenceMeanEndpointBoundaryMargin",
        "longWindowStartTick",
        "longAverageVX",
        "longMeanVXError",
        "longStdVX",
        "longAverageDistanceVX",
        "suffixAverageVX",
        "suffixMeanVXError",
        "suffixStdVX",
        "suffixAverageDistanceVX",
    ];
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(columns.join(","));
    for (index, row) in rows.iter().enumerate() {
        let values = columns
            .iter()
            .map(|column| csv_escape(&csv_value(row, column, index + 1)))
            .collect::<Vec<_>>();
        lines.push(values.join(","));
    }
    fs::write(path, format!("{}\n", lines.join("\n")))
        .map_err(|error| format!("Failed to write CSV: {error}"))
}

fn csv_value(row: &ResultRow, column: &str, rank: usize) -> String {
    match column {
        "rank" => rank.to_string(),
        "id" => row.id.clone(),
        "pass" => row.pass.clone(),
        "score" => row.score.to_string(),
        "backbone" => row.backbone.clone(),
        "proven" => row.proven.to_string(),
        "prefixLabel" => row.prefix_label.clone(),
        "prefixLength" => row.prefix_length.to_string(),
        "period" => row.period.to_string(),
        "startOffset" => row.start_offset.to_string(),
        "entityIdMod4" => row.entity_id_mod4.to_string(),
        "cadenceStartTick" => row.cadence_start_tick.to_string(),
        "cadencePairs" => row.cadence_pairs.to_string(),
        "cadenceMeanAbsDistanceError" => row.cadence_mean_abs_distance_error.to_string(),
        "cadenceMaxAbsDistanceError" => row.cadence_max_abs_distance_error.to_string(),
        "cadenceBlockHitRate" => row.cadence_block_hit_rate.to_string(),
        "cadenceWithinToleranceRate" => row.cadence_within_tolerance_rate.to_string(),
        "fullCadenceStartTick" => option_usize(row.full_cadence_start_tick),
        "fullCadencePairs" => option_usize(row.full_cadence_pairs),
        "fullCadenceMeanAbsDistanceError" => option_f64(row.full_cadence_mean_abs_distance_error),
        "fullCadenceMeanSignedDistanceError" => {
            option_f64(row.full_cadence_mean_signed_distance_error)
        }
        "fullCadenceMaxAbsDistanceError" => option_f64(row.full_cadence_max_abs_distance_error),
        "fullCadenceBlockHitRate" => option_f64(row.full_cadence_block_hit_rate),
        "fullCadenceWithinToleranceRate" => option_f64(row.full_cadence_within_tolerance_rate),
        "fullCadenceLongestHitRun" => option_usize(row.full_cadence_longest_hit_run),
        "fullCadenceAverageSpeed" => option_f64(row.full_cadence_average_speed),
        "fullCadenceDistance" => option_f64(row.full_cadence_distance),
        "fullCadenceMinHitMargin" => option_f64(row.full_cadence_min_hit_margin),
        "fullCadenceMeanHitMargin" => option_f64(row.full_cadence_mean_hit_margin),
        "fullCadenceMinEndpointBoundaryMargin" => {
            option_f64(row.full_cadence_min_endpoint_boundary_margin)
        }
        "fullCadenceMeanEndpointBoundaryMargin" => {
            option_f64(row.full_cadence_mean_endpoint_boundary_margin)
        }
        "longWindowStartTick" => option_usize(row.long_window_start_tick),
        "longAverageVX" => option_f64(row.long_average_vx),
        "longMeanVXError" => option_f64(row.long_mean_vx_error),
        "longStdVX" => option_f64(row.long_std_vx),
        "longAverageDistanceVX" => option_f64(row.long_average_distance_vx),
        "suffixAverageVX" => option_f64(row.suffix_average_vx),
        "suffixMeanVXError" => option_f64(row.suffix_mean_vx_error),
        "suffixStdVX" => option_f64(row.suffix_std_vx),
        "suffixAverageDistanceVX" => option_f64(row.suffix_average_distance_vx),
        _ => String::new(),
    }
}

fn option_usize(value: Option<usize>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_f64(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn format_number(value: Option<f64>, digits: usize) -> String {
    value
        .map(|number| format!("{number:.digits$}"))
        .unwrap_or_default()
}

fn markdown_table(rows: &[ResultRow]) -> String {
    let header = "| Rank | Pass | Backbone | Prefix | StartX | CadenceStart | FullHit | FullAvgSpeed | HitMargin | BoundaryMargin | FullMeanErr2gt | FullMaxErr2gt | Score |";
    let sep = "|---:|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|";
    let body = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            format!(
                "| {} | {} | `{}` | `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                index + 1,
                row.pass,
                row.backbone,
                row.prefix_label,
                format_number(Some(row.start_offset), 5),
                row.cadence_start_tick,
                format_number(row.full_cadence_block_hit_rate, 6),
                format_number(row.full_cadence_average_speed, 9),
                format_number(row.full_cadence_min_hit_margin, 6),
                format_number(row.full_cadence_min_endpoint_boundary_margin, 6),
                format_number(row.full_cadence_mean_abs_distance_error, 6),
                format_number(row.full_cadence_max_abs_distance_error, 6),
                format_number(Some(row.score), 3),
            )
        })
        .collect::<Vec<_>>();
    [vec![header.to_string(), sep.to_string()], body]
        .concat()
        .join("\n")
}

fn markdown_early_table(rows: &[ResultRow]) -> String {
    let header = "| Rank | Pass | Backbone | Prefix | StartX | CadenceStart | EarlyHit | EarlyWithin | EarlyMeanErr2gt | EarlyMaxErr2gt | Score |";
    let sep = "|---:|---|---|---|---:|---:|---:|---:|---:|---:|---:|";
    let body = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            format!(
                "| {} | {} | `{}` | `{}` | {} | {} | {} | {} | {} | {} | {} |",
                index + 1,
                row.pass,
                row.backbone,
                row.prefix_label,
                format_number(Some(row.start_offset), 5),
                row.cadence_start_tick,
                format_number(Some(row.cadence_block_hit_rate), 6),
                format_number(Some(row.cadence_within_tolerance_rate), 6),
                format_number(Some(row.cadence_mean_abs_distance_error), 6),
                format_number(Some(row.cadence_max_abs_distance_error), 6),
                format_number(Some(row.score), 3),
            )
        })
        .collect::<Vec<_>>();
    [vec![header.to_string(), sep.to_string()], body]
        .concat()
        .join("\n")
}

fn write_summary(path: &Path, payload: &SearchPayload, args: &Args) -> Result<(), String> {
    let top_rows = payload
        .results
        .iter()
        .take(args.top)
        .cloned()
        .collect::<Vec<_>>();
    let strong_rows = payload
        .results
        .iter()
        .filter(|row| row.pass == "strong")
        .take(args.top)
        .cloned()
        .collect::<Vec<_>>();
    let proven_rows = payload
        .results
        .iter()
        .filter(|row| row.proven)
        .take(args.top)
        .cloned()
        .collect::<Vec<_>>();
    let is_early_mode = matches!(args.mode, Mode::Early);
    let top_table = if is_early_mode {
        markdown_early_table(&top_rows)
    } else {
        markdown_table(&top_rows)
    };
    let strong_table = if is_early_mode {
        "Early-only mode does not classify full-run strong candidates.".to_string()
    } else if strong_rows.is_empty() {
        "No strong candidates found.".to_string()
    } else {
        markdown_table(&strong_rows)
    };
    let proven_table = if is_early_mode {
        markdown_early_table(&proven_rows)
    } else if proven_rows.is_empty() {
        "No candidates on the proven `W3-I_D3-B` backbone passed the early cadence filter."
            .to_string()
    } else {
        markdown_table(&proven_rows)
    };

    let markdown = vec![
        "# Launch-Aware Item Waterway Search 1.17.1".to_string(),
        String::new(),
        format!(
            "Generated by `cargo run --release --` in `{:?}` mode. Evaluated `{}` launch states, kept `{}` early candidates, deduped to `{}`, and long-verified `{}` candidates.",
            args.mode, payload.evaluated, payload.early_kept, payload.early_deduped, payload.long_verified
        ),
        String::new(),
        "Launch assumption: a moving slime block from a piston has already collided with the item, so the modeled initial horizontal velocity is `vx=+1.0`. This matches the modern Minecraft source path where `PistonMovingBlockEntity.moveCollidedEntities()` overwrites the moved-axis velocity for non-player entities on slime collision, and `Entity.updateFluidInteraction()` applies water current with `0.014`.".to_string(),
        String::new(),
        format!(
            "Early hard target: some 2gt cadence phase must start at or before tick 5, with `{}` consecutive two-tick samples, each within `{}` block of 1.0 and each crossing exactly one block.",
            args.cadence_pairs, args.cadence_tolerance
        ),
        String::new(),
        format!(
            "Full model target: `{}` non-overlapping 2gt samples after the chosen cadence start, normally `{}`gt / about `{}` blocks. Primary metric is `floor(x[t+2])-floor(x[t]) == 1` hit rate.",
            args.full_cadence_pairs,
            args.full_cadence_pairs * 2,
            args.full_cadence_pairs
        ),
        String::new(),
        "## Top Overall".to_string(),
        String::new(),
        if top_rows.is_empty() {
            "No candidates passed the early cadence filter.".to_string()
        } else {
            top_table
        },
        String::new(),
        "## Strong".to_string(),
        String::new(),
        strong_table,
        String::new(),
        "## Proven Backbone Only".to_string(),
        String::new(),
        proven_table,
        String::new(),
        "## Build Notes".to_string(),
        String::new(),
        "- `D*` prefix cells are dry cells over the named floor (`N` normal, `I` packed ice, `B` blue ice, `S` slime).".to_string(),
        "- `R2*` / `R3*` prefix cells are real reverse-water gradients with the source at the right end; single-cell reverse water is intentionally not modeled because it has no source-derived flow direction.".to_string(),
        "- Dry glow lichen (`waterlogged=false`) may be used as the lane-internal non-colliding water blocker for dry gap cells. Waterlogged glow lichen should only be used where the model intentionally calls for source/still water.".to_string(),
        String::new(),
    ]
    .join("\n");

    fs::write(path, markdown).map_err(|error| format!("Failed to write markdown summary: {error}"))
}

fn constants_output() -> ConstantsOutput {
    ConstantsOutput {
        width: WIDTH,
        height: HEIGHT,
        fluid_movement_threshold: FLUID_MOVEMENT_THRESHOLD,
        water_push: WATER_PUSH,
        horizontal_water_damping: HORIZONTAL_WATER_DAMPING,
        horizontal_movement_damping: HORIZONTAL_MOVEMENT_DAMPING,
        vertical_movement_damping: VERTICAL_MOVEMENT_DAMPING,
        gravity: GRAVITY,
        buoyancy: BUOYANCY,
        buoyancy_cap: BUOYANCY_CAP,
        slime_step_on_vy_threshold: SLIME_STEP_ON_VY_THRESHOLD,
        slime_step_on_base: SLIME_STEP_ON_BASE,
        slime_step_on_vy_scale: SLIME_STEP_ON_VY_SCALE,
        horizontal_rest_threshold2: HORIZONTAL_REST_THRESHOLD2,
        aabb_deflate: AABB_DEFLATE,
        movement_sample_modulo: MOVEMENT_SAMPLE_MODULO,
        fluid_current_min_old_movement: FLUID_CURRENT_MIN_OLD_MOVEMENT,
        fluid_current_min_impulse: FLUID_CURRENT_MIN_IMPULSE,
        fluid_current_epsilon2: FLUID_CURRENT_EPSILON2,
    }
}

fn write_json(path: &Path, payload: &SearchPayload, args: &Args) -> Result<(), String> {
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("Failed to format timestamp: {error}"))?;
    let data = JsonOutput {
        generated_at,
        args: args.clone(),
        constants: constants_output(),
        evaluated: payload.evaluated,
        early_kept: payload.early_kept,
        early_deduped: payload.early_deduped,
        long_verified: payload.long_verified,
        top: payload.results.iter().take(args.top).cloned().collect(),
    };
    let json = serde_json::to_string_pretty(&data)
        .map_err(|error| format!("Failed to serialize JSON: {error}"))?;
    fs::write(path, format!("{}\n", json)).map_err(|error| format!("Failed to write JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal_cells(text: &str) -> Vec<LegalWaterCell> {
        text.chars()
            .map(|value| match value {
                'P' => LegalWaterCell::Plate,
                'O' => LegalWaterCell::Open,
                'S' => LegalWaterCell::Source,
                other => panic!("unexpected legal water cell {other}"),
            })
            .collect()
    }

    fn stable_amounts(text: &str) -> String {
        let field = stabilize_waterfield(&legal_cells(text)).expect("waterfield should stabilize");
        field
            .amounts
            .iter()
            .map(|amount| {
                if *amount == 0 {
                    'P'
                } else {
                    char::from_digit(*amount as u32, 10).unwrap()
                }
            })
            .collect()
    }

    fn stable_flows(text: &str) -> String {
        let field = stabilize_waterfield(&legal_cells(text)).expect("waterfield should stabilize");
        field
            .flows
            .iter()
            .enumerate()
            .map(|(index, &flow)| {
                if field.skeleton[index] == LegalWaterCell::Plate {
                    'P'
                } else {
                    flow_code(flow)
                }
            })
            .collect()
    }

    #[test]
    fn legal_waterfield_source_spreads_until_plate() {
        assert_eq!(stable_amounts("PSOP"), "P87P");
        assert_eq!(stable_flows("PSOP"), "P>>P");
    }

    #[test]
    fn legal_waterfield_between_two_sources_becomes_source() {
        assert_eq!(stable_amounts("PSOSP"), "P888P");
        assert_eq!(stable_flows("PSOSP"), "P===P");
    }

    #[test]
    fn legal_waterfield_symmetric_source_has_no_source_push() {
        assert_eq!(stable_amounts("POSOP"), "P787P");
        assert_eq!(stable_flows("POSOP"), "P<= >P".replace(' ', ""));
    }

    #[test]
    fn parse_args_bumps_full_ticks_for_full_cadence() {
        let argv = vec![
            "--ticks".to_string(),
            "20".to_string(),
            "--cadence-pairs".to_string(),
            "4".to_string(),
            "--long-window".to_string(),
            "5".to_string(),
            "--full-cadence-pairs".to_string(),
            "12".to_string(),
        ];
        let ParsedArgs::Run(args) = parse_args(&argv).expect("args should parse") else {
            panic!("expected runnable args");
        };
        assert_eq!(args.ticks, 33);
    }

    #[test]
    fn fluid_tracker_uses_item_height_threshold() {
        let layout = Layout::new(&[cell(Some(0.1), 0, Floor::Normal, Some(1))], &[]);
        let tracker = update_fluid_tracker(&layout, 0.0, 0.0, true);
        assert!((tracker.height - 0.1).abs() < 1.0e-12);
        assert!(tracker.is_in_fluid());
        assert!(!tracker.applies_underwater_movement());
    }

    #[test]
    fn simulate_applies_current_in_base_tick_and_after_item_movement() {
        let layout = Layout::new(
            &one_way_water(2, 1, &[Floor::Normal], false),
            &dry_gap(1, &[Floor::Normal]),
        );
        let sim = simulate(
            &layout,
            &SimConfig {
                ticks: 1,
                start_x: 0.1,
                start_y: 0.0,
                start_vx: 0.0,
                start_vy: 0.0,
                entity_id_mod4: 3,
                initial_tick_count: 0,
                start_on_ground: Some(true),
            },
        );
        assert!((sim.xs[1] - 0.11386000013351441).abs() < 1.0e-12);
        assert!((sim.ys[1] - BUOYANCY).abs() < 1.0e-9);
        assert!((sim.vxs[1] - 0.02758280039520264).abs() < 1.0e-12);
        assert!((sim.vys[1] - BUOYANCY * VERTICAL_MOVEMENT_DAMPING).abs() < 1.0e-9);
    }

    #[test]
    fn cycle_flow_direction_uses_world_neighbor_across_period_boundary() {
        let prefix = [
            cell(Some(1.0 / 9.0), 0, Floor::Normal, Some(1)),
            cell(Some(3.0 / 9.0), 1, Floor::Slime, Some(3)),
        ];
        let cycle = [
            cell(Some(3.0 / 9.0), 1, Floor::Slime, Some(3)),
            cell(Some(5.0 / 9.0), 0, Floor::PackedIce, Some(5)),
            cell(Some(3.0 / 9.0), 1, Floor::PackedIce, Some(3)),
            cell(Some(8.0 / 9.0), 1, Floor::Slime, Some(8)),
            cell(None, 0, Floor::BlueIce, Some(0)),
            cell(Some(7.0 / 9.0), -1, Floor::Slime, Some(7)),
        ];
        let layout = Layout::new(&prefix, &cycle);

        assert_eq!(layout.flow_direction_at(2), -1);
        assert_eq!(layout.flow_direction_at(8), 1);
    }

    #[test]
    fn user_z33_fixture_matches_game_validated_early_ticks() {
        let faststart = centered_source_faststart(&[Floor::PackedIce]);
        let prefix = [
            cell(None, 0, Floor::Normal, Some(0)),
            faststart[0].clone(),
            faststart[1].clone(),
            faststart[2].clone(),
            faststart[3].clone(),
            cell(None, 0, Floor::PackedIce, Some(0)),
            cell(Some(8.0 / 9.0), 1, Floor::PackedIce, Some(8)),
            cell(Some(7.0 / 9.0), 1, Floor::PackedIce, Some(7)),
        ];
        let cycle = [
            cell(None, 0, Floor::PackedIce, Some(0)),
            cell(None, 0, Floor::PackedIce, Some(0)),
            cell(Some(8.0 / 9.0), 1, Floor::PackedIce, Some(8)),
            cell(Some(7.0 / 9.0), 1, Floor::PackedIce, Some(7)),
        ];
        let layout = Layout::new(&prefix, &cycle);
        let sim = simulate(
            &layout,
            &SimConfig {
                ticks: 10,
                start_x: -0.3650000000000091,
                start_y: 0.0,
                start_vx: 1.0,
                start_vy: 0.0,
                entity_id_mod4: 0,
                initial_tick_count: 0,
                start_on_ground: Some(false),
            },
        );
        let expected_xs = [
            -0.3650000000000091,
            0.6349999999999909,
            1.2230000591277985,
            1.754348194612501,
            2.2421421703920297,
            2.715399899463147,
            3.174554561567424,
            3.6477464279693894,
            4.134557190377224,
            4.634581006074399,
            5.147424126445879,
        ];
        let expected_vxs = [
            1.0,
            0.5880000591278076,
            0.5507152831981685,
            0.5067211829096699,
            0.47803810556786974,
            0.46379258351637015,
            0.46397157761987184,
            0.4777280380993446,
            0.49107455644485687,
            0.5040233489204285,
            0.5025862677457565,
        ];
        for index in 0..expected_xs.len() {
            assert!(
                (sim.xs[index] - expected_xs[index]).abs() < 1.0e-12,
                "x mismatch at tick {index}: got {}, expected {}",
                sim.xs[index],
                expected_xs[index]
            );
            assert!(
                (sim.vxs[index] - expected_vxs[index]).abs() < 1.0e-12,
                "vx mismatch at tick {index}: got {}, expected {}",
                sim.vxs[index],
                expected_vxs[index]
            );
        }
    }

    #[test]
    fn search_small_case_returns_stable_early_candidates() {
        let args = Args {
            out: PathBuf::from("artifacts/test"),
            mode: Mode::Early,
            ticks: 17,
            top: 5,
            max_prefix: 2,
            beam_width: 128,
            bucket_keep: 32,
            frontier_structure_keep: 32,
            workers: 2,
            cadence_pairs: 4,
            cadence_tolerance: 0.075,
            long_window: 10,
            start_samples: 3,
            keep_weak: false,
            min_early_block_hit_rate: 0.8,
            early_limit: 5,
            long_limit: 0,
            dedupe_long: true,
            full_cadence_pairs: 4,
            full_cadence_tolerance: 0.05,
            fixed_start_offsets: None,
            cycle_names: None,
            entity_id_mods: vec![0, 1, 2, 3],
            initial_tick_counts: vec![0],
            start_y: 0.0,
            start_vx: 1.0,
            start_vy: 0.0,
            start_on_ground: true,
        };
        let payload = search(&args);
        assert!(payload.evaluated >= 27972);
        assert_eq!(payload.results.len(), 5);
        assert_eq!(payload.results[0].backbone, "W2-I_S2-I");
        assert_eq!(payload.results[0].prefix_label, "DS-SN");
        assert!((payload.results[0].start_offset - 0.875).abs() < 1.0e-12);
    }

    #[test]
    fn backbone_cycles_include_game_validated_faststart_bridge_cycle() {
        let names = backbone_cycles()
            .into_iter()
            .map(|cycle| cycle.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"D1-I_F2-I_D1-B_S1-B_F2-I_D1-B_S1-B".to_string()));
        assert!(names.contains(&"F2-I_D1-B_S1-B_D1-I_F2-I_D1-B_S1-B".to_string()));
    }

    #[test]
    fn prefix_atoms_include_game_validated_faststart_source_shape() {
        let atom = prefix_atoms()
            .into_iter()
            .find(|atom| atom.name == "FS4I")
            .expect("FS4I atom should exist");
        let codes = atom.cells.iter().map(Cell::code).collect::<Vec<_>>();
        assert_eq!(codes, vec!["R7-I", "S8-I", "F7-I", "F6-I"]);
    }
}
