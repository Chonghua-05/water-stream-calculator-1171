use crate::schema::{self, RunsIndex, RunsIndexEntry, ViewerRun, ViewerRunsPayload};
use crate::service::preprocess_structure_for_launch;
use crate::viewer_runs::{self, ViewerRunOptions};
use serde_json::{json, Value};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const MIN_RUN_ID_BASE: u64 = 920000;
const GAME2_FASTSTART_STRUCTURE: &str =
    "user-z29-faststart / F2-I_D1-B bridge / D1-I_F2-I_D1-B_S1-B_F2-I_D1-B_S1-B";
const GAME2_FASTSTART_ORIGIN_X: f64 = 1000.0;
const GAME2_FASTSTART_RAW_START_X: f64 = -0.875;
const GAME2_FASTSTART_EFFECTIVE_START_X: f64 = -0.3650000000000091;
const REFRESHED_STEADY_SUMMARY_KEYS: &[&str] = &[
    "steady_detection_min_block",
    "steady_start_tick",
    "steady_start_block",
    "steady_start_raw_block",
    "steady_end_block",
    "steady_end_raw_block",
    "steady_source",
    "steady_detect_block_hit_rate",
    "steady_detect_average_speed",
    "steady_detect_mean_abs_distance_error",
    "steady_sample_count",
    "steady_avg_speed",
    "steady_per_block_target_dwell_hit_rate",
    "steady_per_block_two_gt_dwell_hit_rate",
    "steady_dwell_blocks",
    "steady_dwell_failures",
    "steady_tail_inferred_hits",
    "avg_derived_speed",
    "avg_speed_x_gt_3",
    "speed_error",
];

#[derive(Debug)]
pub enum RunStoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NotFound(u64),
}

impl Display for RunStoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::NotFound(run_id) => write!(f, "run {run_id} not found"),
        }
    }
}

impl std::error::Error for RunStoreError {}

impl From<std::io::Error> for RunStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for RunStoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Debug)]
pub struct RunStore {
    legacy_runs_path: PathBuf,
    runs_dir: PathBuf,
    runs_index_path: PathBuf,
}

impl RunStore {
    pub fn new(viewer_data_dir: impl Into<PathBuf>) -> Self {
        let viewer_data_dir = viewer_data_dir.into();
        let runs_dir = viewer_data_dir.join("runs");
        Self {
            legacy_runs_path: viewer_data_dir.join("runs.json"),
            runs_index_path: runs_dir.join("index.json"),
            runs_dir,
        }
    }

    pub fn from_paths(
        legacy_runs_path: impl Into<PathBuf>,
        runs_dir: impl Into<PathBuf>,
        runs_index_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            legacy_runs_path: legacy_runs_path.into(),
            runs_dir: runs_dir.into(),
            runs_index_path: runs_index_path.into(),
        }
    }

    pub fn empty_payload(&self) -> ViewerRunsPayload {
        ViewerRunsPayload {
            updated_at: Some(now_iso()),
            latest_run_id: None,
            run_count: 0,
            runs: Vec::new(),
        }
    }

    pub fn run_file_name(run_id: u64) -> String {
        format!("run-{run_id:06}.json")
    }

    pub fn run_file_path(&self, run_id: u64) -> PathBuf {
        self.runs_dir.join(Self::run_file_name(run_id))
    }

    pub fn viewer_data_dir(&self) -> PathBuf {
        self.runs_dir
            .parent()
            .unwrap_or(&self.runs_dir)
            .to_path_buf()
    }

    pub fn load_runs(&self) -> Result<ViewerRunsPayload, RunStoreError> {
        if self.runs_index_path.exists() {
            self.load_split_runs()
        } else {
            self.load_legacy_runs()
        }
    }

    pub fn load_legacy_runs(&self) -> Result<ViewerRunsPayload, RunStoreError> {
        if !self.legacy_runs_path.exists() {
            return Ok(self.empty_payload());
        }
        let mut payload: ViewerRunsPayload = read_json_file(&self.legacy_runs_path)?;
        payload.run_count = payload.runs.len();
        if payload.updated_at.is_none() {
            payload.updated_at = Some(now_iso());
        }
        if payload.latest_run_id.is_none() {
            payload.latest_run_id = payload
                .runs
                .iter()
                .filter(|run| !run.summary.deleted)
                .filter_map(|run| run.run_id)
                .next_back();
        }
        hydrate_payload(&mut payload);
        Ok(payload)
    }

    pub fn load_split_runs(&self) -> Result<ViewerRunsPayload, RunStoreError> {
        if !self.runs_index_path.exists() {
            return Ok(self.empty_payload());
        }
        let index: RunsIndex = read_json_file(&self.runs_index_path)?;
        let runs_root = self
            .runs_dir
            .canonicalize()
            .unwrap_or_else(|_| self.runs_dir.clone());
        let mut runs = Vec::new();
        for entry in index.runs {
            let file_name = if entry.file.is_empty() {
                Self::run_file_name(entry.run_id)
            } else {
                entry.file
            };
            let candidate = self.runs_dir.join(&file_name);
            let resolved = candidate.canonicalize().unwrap_or(candidate.clone());
            let path = if resolved.starts_with(&runs_root) {
                resolved
            } else {
                self.run_file_path(entry.run_id)
            };
            if !path.exists() {
                continue;
            }
            let mut run: ViewerRun = read_json_file(&path)?;
            if run.run_id.is_none() {
                run.run_id = Some(entry.run_id);
            }
            hydrate_run(&mut run);
            runs.push(run);
        }
        let latest_run_id = runs
            .iter()
            .filter(|run| !run.summary.deleted)
            .filter_map(|run| run.run_id)
            .next_back();
        Ok(ViewerRunsPayload {
            updated_at: index.updated_at.or_else(|| Some(now_iso())),
            latest_run_id,
            run_count: runs.len(),
            runs,
        })
    }

    pub fn save_runs(&self, payload: &ViewerRunsPayload) -> Result<ViewerRunsPayload, RunStoreError> {
        fs::create_dir_all(&self.runs_dir)?;
        let mut normalized = payload.clone();
        normalized.updated_at = Some(now_iso());
        normalized.run_count = normalized.runs.len();
        normalized.latest_run_id = normalized
            .runs
            .iter()
            .filter(|run| !run.summary.deleted)
            .filter_map(|run| run.run_id)
            .next_back();

        let mut index_runs = Vec::with_capacity(normalized.runs.len());
        for run in &mut normalized.runs {
            let run_id = run.run_id.unwrap_or_default();
            run.run_id = Some(run_id);
            let file_name = Self::run_file_name(run_id);
            atomic_write_json(&self.runs_dir.join(&file_name), run)?;
            index_runs.push(RunsIndexEntry {
                run_id,
                file: file_name,
                label: run.label.clone(),
                display_label: run.display_label.clone(),
                source: run.summary.source.clone(),
                deleted: run.summary.deleted,
            });
        }

        let index = RunsIndex {
            updated_at: normalized.updated_at.clone(),
            latest_run_id: normalized.latest_run_id,
            run_count: normalized.runs.len(),
            runs: index_runs,
        };
        atomic_write_json(&self.runs_index_path, &index)?;
        Ok(normalized)
    }

    pub fn append_run(&self, run: ViewerRun) -> Result<ViewerRun, RunStoreError> {
        let mut payload = self.load_runs()?;
        let max_id = payload
            .runs
            .iter()
            .filter_map(|item| item.run_id)
            .max()
            .unwrap_or(MIN_RUN_ID_BASE);
        let mut run = run;
        run.run_id = Some(max_id + 1);
        payload.runs.push(run);
        let saved = self.save_runs(&payload)?;
        saved
            .runs
            .last()
            .cloned()
            .ok_or(RunStoreError::NotFound(max_id + 1))
    }

    pub fn soft_delete_run(&self, run_id: u64) -> Result<DeletedRunResult, RunStoreError> {
        let mut payload = self.load_runs()?;
        let Some(run) = payload.runs.iter_mut().find(|run| run.run_id == Some(run_id)) else {
            return Err(RunStoreError::NotFound(run_id));
        };
        run.summary.deleted = true;
        self.save_runs(&payload)?;
        Ok(DeletedRunResult { deleted_run_id: run_id })
    }

    pub fn restore_run(&self, run_id: u64) -> Result<RestoredRunResult, RunStoreError> {
        let mut payload = self.load_runs()?;
        let Some(run) = payload.runs.iter_mut().find(|run| run.run_id == Some(run_id)) else {
            return Err(RunStoreError::NotFound(run_id));
        };
        run.summary.deleted = false;
        self.save_runs(&payload)?;
        Ok(RestoredRunResult {
            restored_run_id: run_id,
        })
    }

    pub fn purge_run(&self, run_id: u64) -> Result<PurgedRunResult, RunStoreError> {
        let mut payload = self.load_runs()?;
        let before = payload.runs.len();
        payload.runs.retain(|run| run.run_id != Some(run_id));
        if payload.runs.len() == before {
            return Err(RunStoreError::NotFound(run_id));
        }
        self.save_runs(&payload)?;
        match fs::remove_file(self.run_file_path(run_id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(PurgedRunResult {
            permanently_deleted_run_id: run_id,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedRunResult {
    pub deleted_run_id: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredRunResult {
    pub restored_run_id: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgedRunResult {
    pub permanently_deleted_run_id: u64,
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, RunStoreError> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(strip_utf8_bom(&text))?)
}

fn atomic_write_json(path: &Path, payload: &impl Serialize) -> Result<(), RunStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().and_then(|value| value.to_str()).unwrap_or("payload")
    ));
    let json = serde_json::to_string_pretty(payload)?;
    fs::write(&tmp, json)?;
    if path.exists() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn hydrate_payload(payload: &mut ViewerRunsPayload) {
    for run in &mut payload.runs {
        hydrate_run(run);
    }
}

fn hydrate_run(run: &mut ViewerRun) {
    refresh_legacy_piston_search_run(run);
    refresh_display_zero_metrics(run);
    refresh_steady_summary_metrics(run);
    if run.structure.is_some() {
        return;
    }
    let Some(structure) = hydrate_known_structure(run) else {
        return;
    };
    run.structure = Some(structure);
    if run.summary.launch_mode.is_none() {
        run.summary.launch_mode = Some("water".to_string());
    }
}

fn refresh_legacy_piston_search_run(run: &mut ViewerRun) {
    if run.summary.source.as_deref() != Some("reachability-search") {
        return;
    }
    let Some(structure) = run.structure.as_ref() else {
        return;
    };
    let Some(launch) = structure.extra.get("launch").and_then(Value::as_object) else {
        return;
    };
    if launch.get("mode").and_then(Value::as_str) != Some("piston") {
        return;
    }
    let has_effective_local_start = launch.get("effectiveLocalStart").is_some();
    let Some(raw_start_x) = launch
        .get("rawStart")
        .and_then(Value::as_object)
        .and_then(|value| value.get("x"))
        .and_then(Value::as_f64)
    else {
        return;
    };
    let Some(effective_start_x) = launch
        .get("effectiveStart")
        .and_then(Value::as_object)
        .and_then(|value| value.get("x"))
        .and_then(Value::as_f64)
    else {
        return;
    };
    let waterway_start_x = launch
        .get("waterwayStartX")
        .and_then(Value::as_f64)
        .unwrap_or(raw_start_x.floor() + 1.0);
    let expected_local_x = effective_start_x - waterway_start_x;
    let start_matches_effective_local = search_start_matches_effective_local_start(structure, launch);
    if has_effective_local_start
        && (structure.start.x - expected_local_x).abs() <= 1.0e-12
        && start_matches_effective_local
    {
        return;
    }

    let rebuild_input = normalize_legacy_piston_search_structure(structure, launch);
    let Ok(updated_structure) = preprocess_structure_for_launch(&rebuild_input, "piston") else {
        return;
    };
    let ticks = run.points.len().saturating_sub(1);
    let Ok(updated_points) =
        viewer_runs::simulate_viewer_points_for_requested_duration(&updated_structure, ticks)
    else {
        return;
    };
    run.structure = Some(updated_structure);
    run.points = updated_points;
    refresh_launch_summary_metrics(run);
}

fn search_start_matches_effective_local_start(
    structure: &schema::Structure,
    launch: &serde_json::Map<String, Value>,
) -> bool {
    let Some(effective) = launch
        .get("effectiveLocalStart")
        .and_then(Value::as_object)
    else {
        return false;
    };
    let Some(start_x) = effective.get("x").and_then(Value::as_f64) else {
        return false;
    };
    let Some(start_y) = effective.get("y").and_then(Value::as_f64) else {
        return false;
    };
    let Some(start_vx) = effective.get("vx").and_then(Value::as_f64) else {
        return false;
    };
    let Some(start_vy) = effective.get("vy").and_then(Value::as_f64) else {
        return false;
    };
    let Some(start_on_ground) = effective.get("startOnGround").and_then(Value::as_bool) else {
        return false;
    };
    let Some(entity_id_mod4) = effective.get("entityIdMod4").and_then(Value::as_u64) else {
        return false;
    };
    let Some(initial_tick_count) = effective.get("initialTickCount").and_then(Value::as_u64) else {
        return false;
    };

    (structure.start.x - start_x).abs() <= 1.0e-12
        && (structure.start.y - start_y).abs() <= 1.0e-12
        && (structure.start.vx - start_vx).abs() <= 1.0e-12
        && (structure.start.vy - start_vy).abs() <= 1.0e-12
        && structure.start.start_on_ground.unwrap_or(false) == start_on_ground
        && structure.start.entity_id_mod4 == entity_id_mod4 as usize
        && structure.start.initial_tick_count == initial_tick_count as usize
}

fn normalize_legacy_piston_search_structure(
    structure: &schema::Structure,
    launch: &serde_json::Map<String, Value>,
) -> schema::Structure {
    let mut normalized = structure.clone();
    let Some(raw_start) = launch.get("rawStart").and_then(Value::as_object) else {
        return normalized;
    };
    let Some(raw_start_x) = raw_start.get("x").and_then(Value::as_f64) else {
        return normalized;
    };
    let Some(raw_start_y) = raw_start.get("y").and_then(Value::as_f64) else {
        return normalized;
    };
    let Some(raw_start_vx) = raw_start.get("vx").and_then(Value::as_f64) else {
        return normalized;
    };
    let Some(raw_start_vy) = raw_start.get("vy").and_then(Value::as_f64) else {
        return normalized;
    };
    let Some(raw_start_on_ground) = raw_start.get("startOnGround").and_then(Value::as_bool) else {
        return normalized;
    };
    let Some(raw_entity_id_mod4) = raw_start.get("entityIdMod4").and_then(Value::as_u64) else {
        return normalized;
    };
    let Some(raw_initial_tick_count) = raw_start.get("initialTickCount").and_then(Value::as_u64)
    else {
        return normalized;
    };

    let mut adjusted_raw_start = schema::StartState {
        x: raw_start_x,
        y: raw_start_y,
        vx: raw_start_vx,
        vy: raw_start_vy,
        start_on_ground: Some(raw_start_on_ground),
        entity_id_mod4: raw_entity_id_mod4 as usize,
        initial_tick_count: raw_initial_tick_count as usize,
        extra: BTreeMap::new(),
    };

    if let Some(effective) = launch
        .get("effectiveLocalStart")
        .and_then(Value::as_object)
    {
        let effective_initial_tick_count = effective
            .get("initialTickCount")
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let timeline_offset = launch
            .get("timelineOffsetGt")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0);
        if let Some(current_effective_initial_tick_count) = effective_initial_tick_count {
            if structure.start.initial_tick_count != current_effective_initial_tick_count {
                let desired_effective_mod4 = structure.start.initial_tick_count % 4;
                let target_raw_mod4 =
                    (desired_effective_mod4 + 4 - (timeline_offset % 4)) % 4;
                let base_raw_mod4 = adjusted_raw_start.initial_tick_count % 4;
                adjusted_raw_start.initial_tick_count +=
                    (target_raw_mod4 + 4 - base_raw_mod4) % 4;
            }
        }
    }

    adjusted_raw_start.entity_id_mod4 = structure.start.entity_id_mod4;
    normalized.start = adjusted_raw_start.clone();
    if let Some(extra_launch) = normalized.extra.get_mut("launch") {
        extra_launch["rawStart"] = json!({
            "x": adjusted_raw_start.x,
            "y": adjusted_raw_start.y,
            "vx": adjusted_raw_start.vx,
            "vy": adjusted_raw_start.vy,
            "startOnGround": adjusted_raw_start.start_on_ground.unwrap_or(true),
            "entityIdMod4": adjusted_raw_start.entity_id_mod4,
            "initialTickCount": adjusted_raw_start.initial_tick_count,
        });
    }
    normalized
}

fn refresh_display_zero_metrics(run: &mut ViewerRun) {
    if run.points.is_empty() {
        return;
    }
    let start_x_raw = run
        .points
        .first()
        .and_then(|point| point.x_raw.or(point.x))
        .or_else(|| run.summary.extra.get("start_x_raw").and_then(Value::as_f64))
        .or_else(|| run.summary.extra.get("start_x").and_then(Value::as_f64));
    let Some(start_x_raw) = start_x_raw else {
        return;
    };

    for point in &mut run.points {
        if let Some(x_raw) = point.x_raw.or(point.x) {
            let display_x = viewer_runs::display_x_from_raw(x_raw, start_x_raw);
            point.x = Some(display_x);
            if point.x_raw.is_none() {
                point.x_raw = Some(x_raw);
            }
        }
    }

    run.summary
        .extra
        .insert("sample_count".to_string(), json!(run.points.len()));
    run.summary.extra.insert(
        "duration_gt".to_string(),
        json!(run.points.len().saturating_sub(1)),
    );
    if let Some(point) = run.points.first() {
        if let Some(value) = point.x {
            run.summary.extra.insert("start_x".to_string(), json!(value));
        }
        if let Some(value) = point.x_raw {
            run.summary
                .extra
                .insert("start_x_raw".to_string(), json!(value));
        }
    }
    if let Some(point) = run.points.last() {
        if let Some(value) = point.x {
            run.summary.extra.insert("end_x".to_string(), json!(value));
        }
        if let Some(value) = point.x_raw {
            run.summary.extra.insert("end_x_raw".to_string(), json!(value));
        }
    }
}

fn refresh_steady_summary_metrics(run: &mut ViewerRun) {
    if run.points.is_empty() {
        return;
    }
    let options = viewer_run_options_from_summary(&run.summary.extra);
    let metrics = viewer_runs::metrics_for_points(&run.points, &options);
    rewrite_steady_summary_metrics(&mut run.summary.extra, &metrics);
}

fn viewer_run_options_from_summary(summary: &BTreeMap<String, Value>) -> ViewerRunOptions {
    let mut options = ViewerRunOptions::new();
    copy_summary_option(summary, "target_speed", "targetSpeed", &mut options);
    copy_summary_option(summary, "target_dwell_ticks", "targetDwellTicks", &mut options);
    copy_summary_option(
        summary,
        "steady_detection_min_block",
        "steadyMinBlock",
        &mut options,
    );
    options
}

fn copy_summary_option(
    summary: &BTreeMap<String, Value>,
    summary_key: &str,
    option_key: &str,
    target: &mut ViewerRunOptions,
) {
    if let Some(value) = summary.get(summary_key) {
        target.insert(option_key.to_string(), value.clone());
    }
}

fn rewrite_steady_summary_metrics(summary: &mut BTreeMap<String, Value>, metrics: &viewer_runs::ViewerRunMetrics) {
    for key in REFRESHED_STEADY_SUMMARY_KEYS {
        summary.remove(*key);
    }
    let Ok(Value::Object(serialized)) = serde_json::to_value(metrics) else {
        return;
    };
    for key in REFRESHED_STEADY_SUMMARY_KEYS {
        if let Some(value) = serialized.get(*key) {
            summary.insert((*key).to_string(), value.clone());
        }
    }
}

fn refresh_launch_summary_metrics(run: &mut ViewerRun) {
    let Some(structure) = run.structure.as_ref() else {
        return;
    };
    remove_launch_summary_metrics(&mut run.summary.extra);
    if let Some(points_len) = (!run.points.is_empty()).then_some(run.points.len()) {
        run.summary
            .extra
            .insert("sample_count".to_string(), json!(points_len));
        run.summary.extra.insert(
            "duration_gt".to_string(),
            json!(points_len.saturating_sub(1)),
        );
    }
    if let Some(point) = run.points.first() {
        if let Some(value) = point.x {
            run.summary.extra.insert("start_x".to_string(), json!(value));
        }
        if let Some(value) = point.x_raw {
            run.summary
                .extra
                .insert("start_x_raw".to_string(), json!(value));
        }
    }
    if let Some(point) = run.points.last() {
        if let Some(value) = point.x {
            run.summary.extra.insert("end_x".to_string(), json!(value));
        }
        if let Some(value) = point.x_raw {
            run.summary.extra.insert("end_x_raw".to_string(), json!(value));
        }
    }
    let Some(launch) = structure.extra.get("launch").and_then(Value::as_object) else {
        return;
    };
    copy_launch_summary_metric(&mut run.summary.extra, launch, "applied", "launch_applied");
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "pistonTicks",
        "launch_piston_ticks",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "collisionGt",
        "launch_collision_gt",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "lastCollisionGt",
        "launch_last_collision_gt",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "collisionCount",
        "launch_collision_count",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "slimeBlockX",
        "launch_slime_block_x",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "pistonMovement",
        "launch_piston_movement",
    );
    copy_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "pistonMovementTotal",
        "launch_piston_movement_total",
    );
    copy_nested_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "rawStart",
        "x",
        "launch_raw_start_x",
    );
    copy_nested_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "rawStart",
        "vx",
        "launch_raw_start_vx",
    );
    copy_nested_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "effectiveStart",
        "x",
        "launch_effective_start_x",
    );
    copy_nested_launch_summary_metric(
        &mut run.summary.extra,
        launch,
        "effectiveStart",
        "vx",
        "launch_effective_start_vx",
    );
}

fn remove_launch_summary_metrics(summary: &mut BTreeMap<String, Value>) {
    const LAUNCH_KEYS: &[&str] = &[
        "sample_count",
        "start_x_raw",
        "end_x_raw",
        "start_x",
        "end_x",
        "duration_gt",
        "launch_applied",
        "launch_piston_ticks",
        "launch_collision_gt",
        "launch_last_collision_gt",
        "launch_collision_count",
        "launch_slime_block_x",
        "launch_piston_movement",
        "launch_piston_movement_total",
        "launch_raw_start_x",
        "launch_raw_start_vx",
        "launch_effective_start_x",
        "launch_effective_start_vx",
    ];
    for key in LAUNCH_KEYS {
        summary.remove(*key);
    }
}

fn copy_launch_summary_metric(
    summary: &mut BTreeMap<String, Value>,
    launch: &serde_json::Map<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = launch.get(source_key) {
        summary.insert(target_key.to_string(), value.clone());
    }
}

fn copy_nested_launch_summary_metric(
    summary: &mut BTreeMap<String, Value>,
    launch: &serde_json::Map<String, Value>,
    object_key: &str,
    nested_key: &str,
    target_key: &str,
) {
    if let Some(value) = launch
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|object| object.get(nested_key))
    {
        summary.insert(target_key.to_string(), value.clone());
    }
}

fn hydrate_known_structure(run: &ViewerRun) -> Option<schema::Structure> {
    if run.summary.source.as_deref() != Some("game-storage") {
        return None;
    }
    let structure_name = run.summary.structure.as_deref()?;
    if structure_name != GAME2_FASTSTART_STRUCTURE {
        return None;
    }
    Some(game2_faststart_structure(structure_name))
}

fn game2_faststart_structure(name: &str) -> schema::Structure {
    let mut structure = schema::Structure {
        name: Some(name.to_string()),
        origin_x: GAME2_FASTSTART_ORIGIN_X,
        start: schema::StartState {
            x: GAME2_FASTSTART_EFFECTIVE_START_X,
            y: 0.0,
            vx: 1.0,
            vy: 0.0,
            start_on_ground: Some(false),
            entity_id_mod4: 0,
            initial_tick_count: 1,
            extra: BTreeMap::new(),
        },
        launch_config: None,
        prefix: vec![
            schema::make_cell(
                None,
                0,
                "glass",
                Some("D-glass".to_string()),
                Some(0),
            ),
            schema::make_cell(Some(7.0 / 9.0), -1, "packed_ice", None, Some(7)),
            schema::make_cell(Some(8.0 / 9.0), 0, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(Some(6.0 / 9.0), 1, "packed_ice", None, Some(6)),
            schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
        ],
        cycle: vec![
            schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 0, "blue_ice", None, Some(8)),
            schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 0, "blue_ice", None, Some(8)),
        ],
        extra: BTreeMap::new(),
    };
    structure.extra.insert(
        "launch".to_string(),
        game2_faststart_launch_entry(structure.start.x, &structure.start),
    );
    structure
}

fn game2_faststart_launch_entry(
    effective_start_x: f64,
    effective_start: &schema::StartState,
) -> Value {
    json!({
        "mode": "water",
        "applied": true,
        "rawStart": {
            "x": GAME2_FASTSTART_RAW_START_X,
            "y": 0.0,
            "vx": 0.0,
            "vy": 0.0,
            "startOnGround": false,
            "entityIdMod4": 0,
            "initialTickCount": 0
        },
        "effectiveStart": {
            "x": effective_start_x,
            "y": effective_start.y,
            "vx": effective_start.vx,
            "vy": effective_start.vy,
            "startOnGround": effective_start.start_on_ground.unwrap_or(false),
            "entityIdMod4": effective_start.entity_id_mod4,
            "initialTickCount": effective_start.initial_tick_count
        },
        "timelineOffsetGt": 1,
        "timelineSamples": [
            {
                "gt": 0,
                "x": GAME2_FASTSTART_RAW_START_X,
                "y": 0.0,
                "vx": 0.0,
                "vy": 0.0,
                "onGround": false
            },
            {
                "gt": 1,
                "x": effective_start_x,
                "y": effective_start.y,
                "vx": effective_start.vx,
                "vy": effective_start.vy,
                "onGround": effective_start.start_on_ground.unwrap_or(false)
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ViewerRunSummary;
    use crate::viewer_runs;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "item-waterway-solver-run-store-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time before unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("create temp test dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn viewer_run(run_id: u64, label: &str) -> ViewerRun {
        ViewerRun {
            run_id: Some(run_id),
            label: Some(label.to_string()),
            display_label: Some(label.to_string()),
            summary: ViewerRunSummary {
                source: Some("test".to_string()),
                deleted: false,
                ..ViewerRunSummary::default()
            },
            points: Vec::new(),
            structure: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn load_runs_falls_back_to_legacy_when_split_index_is_missing() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);
        let legacy = ViewerRunsPayload {
            updated_at: Some("2026-06-05T00:00:00Z".to_string()),
            latest_run_id: Some(920001),
            run_count: 1,
            runs: vec![viewer_run(920001, "legacy")],
        };
        fs::write(
            temp.path.join("runs.json"),
            serde_json::to_string_pretty(&legacy).expect("serialize legacy payload"),
        )
        .expect("write legacy runs.json");

        let loaded = store.load_runs().expect("load legacy runs");
        assert_eq!(loaded.run_count, 1);
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].run_id, Some(920001));
        assert_eq!(loaded.runs[0].label.as_deref(), Some("legacy"));
    }

    #[test]
    fn load_runs_prefers_split_store_when_index_exists() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);

        fs::write(
            temp.path.join("runs.json"),
            serde_json::to_string_pretty(&ViewerRunsPayload {
                updated_at: Some("2026-06-05T00:00:00Z".to_string()),
                latest_run_id: Some(920001),
                run_count: 1,
                runs: vec![viewer_run(920001, "legacy")],
            })
            .expect("serialize legacy payload"),
        )
        .expect("write legacy runs");

        let split_payload = ViewerRunsPayload {
            updated_at: Some("2026-06-05T00:00:01Z".to_string()),
            latest_run_id: Some(920002),
            run_count: 1,
            runs: vec![viewer_run(920002, "split")],
        };
        store.save_runs(&split_payload).expect("save split payload");

        let loaded = store.load_runs().expect("load split runs");
        assert_eq!(loaded.run_count, 1);
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].run_id, Some(920002));
        assert_eq!(loaded.runs[0].label.as_deref(), Some("split"));
    }

    #[test]
    fn load_split_runs_accepts_existing_snake_case_store_files() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);
        let runs_dir = temp.path.join("runs");
        fs::create_dir_all(&runs_dir).expect("create runs dir");

        fs::write(
            runs_dir.join("index.json"),
            serde_json::to_string_pretty(&json!({
                "updated_at": "2026-06-05T00:00:00Z",
                "latest_run_id": 920010,
                "run_count": 1,
                "runs": [{
                    "run_id": 920010,
                    "file": "run-920010.json",
                    "label": "compat-run",
                    "display_label": "Compat Run",
                    "source": "reachability-search",
                    "deleted": false
                }]
            }))
            .expect("serialize index"),
        )
        .expect("write index");

        fs::write(
            runs_dir.join("run-920010.json"),
            serde_json::to_string_pretty(&json!({
                "run_id": 920010,
                "label": "compat-run",
                "display_label": "Compat Run",
                "summary": {
                    "source": "reachability-search",
                    "model_engine": "rust-item-waterway-solver",
                    "structure_count": 2,
                    "deleted": false,
                    "launch_mode": "piston",
                    "equivalent_fingerprint": "fingerprint-1"
                },
                "points": [{
                    "tick_index": 4,
                    "x": 3.0,
                    "x_raw": 3.125,
                    "derived_speed": 0.5,
                    "on_ground": true
                }]
            }))
            .expect("serialize run file"),
        )
        .expect("write run file");

        let loaded = store.load_runs().expect("load existing split runs");
        assert_eq!(loaded.run_count, 1);
        assert_eq!(loaded.latest_run_id, Some(920010));
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].run_id, Some(920010));
        assert_eq!(loaded.runs[0].display_label.as_deref(), Some("Compat Run"));
        assert_eq!(
            loaded.runs[0].summary.model_engine.as_deref(),
            Some("rust-item-waterway-solver")
        );
        assert_eq!(loaded.runs[0].summary.structure_count, Some(2));
        assert_eq!(loaded.runs[0].summary.launch_mode.as_deref(), Some("piston"));
        assert_eq!(
            loaded.runs[0].summary.equivalent_fingerprint.as_deref(),
            Some("fingerprint-1")
        );
        assert_eq!(loaded.runs[0].points[0].tick_index, 4);
        assert_eq!(loaded.runs[0].points[0].x_raw, Some(3.125));
        assert_eq!(loaded.runs[0].points[0].derived_speed, Some(0.5));
        assert_eq!(loaded.runs[0].points[0].on_ground, Some(true));
    }

    #[test]
    fn load_split_runs_hydrates_known_game_storage_structure() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);
        let runs_dir = temp.path.join("runs");
        fs::create_dir_all(&runs_dir).expect("create runs dir");

        fs::write(
            runs_dir.join("index.json"),
            serde_json::to_string_pretty(&json!({
                "updated_at": "2026-06-08T00:00:00Z",
                "latest_run_id": 910002,
                "run_count": 1,
                "runs": [{
                    "run_id": 910002,
                    "file": "run-910002.json",
                    "label": "Game Storage 0002",
                    "display_label": "游戏实测2",
                    "source": "game-storage",
                    "deleted": false
                }]
            }))
            .expect("serialize index"),
        )
        .expect("write index");

        fs::write(
            runs_dir.join("run-910002.json"),
            serde_json::to_string_pretty(&json!({
                "run_id": 910002,
                "label": "Game Storage 0002",
                "display_label": "游戏实测2",
                "summary": {
                    "source": "game-storage",
                    "structure": GAME2_FASTSTART_STRUCTURE,
                    "deleted": false
                },
                "points": [{
                    "tick_index": 0,
                    "x": 0.0,
                    "x_raw": 999.125
                }]
            }))
            .expect("serialize run"),
        )
        .expect("write run");

        let loaded = store.load_runs().expect("load hydrated game run");
        let structure = loaded.runs[0]
            .structure
            .as_ref()
            .expect("known game storage run should hydrate structure");
        assert_eq!(loaded.runs[0].summary.launch_mode.as_deref(), Some("water"));
        assert_eq!(structure.origin_x, 1000.0);
        assert!((structure.start.x - GAME2_FASTSTART_EFFECTIVE_START_X).abs() < 1.0e-12);
        assert_eq!(structure.start.vx, 1.0);
        assert_eq!(structure.start.start_on_ground, Some(false));
        assert_eq!(structure.start.initial_tick_count, 1);
        assert_eq!(structure.prefix.len(), 9);
        assert_eq!(structure.cycle.len(), 9);
        assert_eq!(structure.prefix[0].floor.as_str(), "glass");
        assert_eq!(structure.prefix[0].code.as_deref(), Some("D-glass"));
        let launch = structure
            .extra
            .get("launch")
            .and_then(Value::as_object)
            .expect("hydrated structure should keep launch timeline");
        assert_eq!(launch.get("mode").and_then(Value::as_str), Some("water"));
        assert_eq!(
            launch
                .get("timelineSamples")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );

        let points = viewer_runs::simulate_viewer_points_for_requested_duration(structure, 4)
            .expect("simulate hydrated historical structure");
        assert_eq!(points.len(), 5);
        assert!((points[0].x_raw.expect("tick0 raw x") - 999.125).abs() < 1.0e-12);
        assert!((points[1].x_raw.expect("tick1 raw x") - 999.635).abs() < 1.0e-12);
        assert!((points[2].x_raw.expect("tick2 raw x") - 1000.635).abs() < 1.0e-12);
        assert!((points[1].speed.expect("tick1 speed") - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn load_split_runs_refreshes_legacy_explicit_steady_summary_from_points() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);
        let runs_dir = temp.path.join("runs");
        fs::create_dir_all(&runs_dir).expect("create runs dir");

        fs::write(
            runs_dir.join("index.json"),
            serde_json::to_string_pretty(&json!({
                "updated_at": "2026-06-09T00:00:00Z",
                "latest_run_id": 910003,
                "run_count": 1,
                "runs": [{
                    "run_id": 910003,
                    "file": "run-910003.json",
                    "label": "search result",
                    "display_label": "search result",
                    "source": "reachability-search",
                    "deleted": false
                }]
            }))
            .expect("serialize index"),
        )
        .expect("write index");

        fs::write(
            runs_dir.join("run-910003.json"),
            serde_json::to_string_pretty(&json!({
                "run_id": 910003,
                "label": "search result",
                "display_label": "search result",
                "summary": {
                    "source": "reachability-search",
                    "deleted": false,
                    "target_speed": 0.5,
                    "target_dwell_ticks": 2,
                    "steady_source": "explicit",
                    "steady_start_tick": 15,
                    "steady_start_raw_block": 8,
                    "steady_start_block": 7,
                    "steady_end_raw_block": 12,
                    "search_dwell_window": {
                        "mode": "cycle",
                        "minBlock": 8,
                        "maxBlock": 12,
                        "minStartTick": 0,
                        "includeFinalGroup": false
                    }
                },
                "points": [
                    { "tick_index": 0, "x": 0.0, "x_raw": 0.75, "derived_speed": 0.0, "on_ground": true },
                    { "tick_index": 1, "x": 0.0, "x_raw": 0.75, "derived_speed": 0.0, "on_ground": true },
                    { "tick_index": 2, "x": 0.385, "x_raw": 1.135, "derived_speed": 0.385, "on_ground": false },
                    { "tick_index": 3, "x": 1.3611400094032289, "x_raw": 2.1111400094032287, "derived_speed": 0.9761400094032286, "on_ground": true },
                    { "tick_index": 4, "x": 1.901650694023664, "x_raw": 2.651650694023664, "derived_speed": 0.5405106846204353, "on_ground": false },
                    { "tick_index": 5, "x": 2.3983341752333276, "x_raw": 3.1483341752333276, "derived_speed": 0.49668348120966366, "on_ground": false },
                    { "tick_index": 6, "x": 2.8850839962922836, "x_raw": 3.635083996292283, "derived_speed": 0.486749821058956, "on_ground": true },
                    { "tick_index": 7, "x": 3.3525585463011627, "x_raw": 4.102558546301163, "derived_speed": 0.46747455000887905, "on_ground": true },
                    { "tick_index": 8, "x": 3.824751503888145, "x_raw": 4.5747515038881446, "derived_speed": 0.4721929575869819, "on_ground": false },
                    { "tick_index": 9, "x": 4.310593124935485, "x_raw": 5.060593124935485, "derived_speed": 0.4858416210473404, "on_ground": false },
                    { "tick_index": 10, "x": 4.80967667965735, "x_raw": 5.55967667965735, "derived_speed": 0.4990835547218646, "on_ground": false },
                    { "tick_index": 11, "x": 5.3216075588040415, "x_raw": 6.0716075588040415, "derived_speed": 0.5119308791466919, "on_ground": false },
                    { "tick_index": 12, "x": 5.819910369065409, "x_raw": 6.569910369065409, "derived_speed": 0.49830281026136786, "on_ground": true },
                    { "tick_index": 13, "x": 6.359910369065409, "x_raw": 7.109910369065409, "derived_speed": 0.5400000000000000, "on_ground": true },
                    { "tick_index": 14, "x": 6.84075801172369, "x_raw": 7.59075801172369, "derived_speed": 0.4808476426582808, "on_ground": false },
                    { "tick_index": 15, "x": 7.34075801172369, "x_raw": 8.09075801172369, "derived_speed": 0.5, "on_ground": false },
                    { "tick_index": 16, "x": 7.827422883469568, "x_raw": 8.577422883469568, "derived_speed": 0.4866648717458783, "on_ground": true },
                    { "tick_index": 17, "x": 8.327422883469568, "x_raw": 9.077422883469568, "derived_speed": 0.5, "on_ground": true },
                    { "tick_index": 18, "x": 8.865386671393953, "x_raw": 9.615386671393953, "derived_speed": 0.5379637879243848, "on_ground": false },
                    { "tick_index": 19, "x": 9.365386671393953, "x_raw": 10.115386671393953, "derived_speed": 0.5, "on_ground": false },
                    { "tick_index": 20, "x": 9.92918276655572, "x_raw": 10.67918276655572, "derived_speed": 0.5637960951617672, "on_ground": false },
                    { "tick_index": 21, "x": 10.42918276655572, "x_raw": 11.17918276655572, "derived_speed": 0.5, "on_ground": false },
                    { "tick_index": 22, "x": 10.924558817989368, "x_raw": 11.674558817989368, "derived_speed": 0.4953760514336477, "on_ground": true },
                    { "tick_index": 23, "x": 11.424558817989368, "x_raw": 12.174558817989368, "derived_speed": 0.5, "on_ground": true }
                ]
            }))
            .expect("serialize run"),
        )
        .expect("write run");

        let loaded = store.load_runs().expect("load run with refreshed steady");
        let summary = &loaded.runs[0].summary.extra;
        assert_eq!(summary.get("steady_source").and_then(Value::as_str), Some("detected"));
        assert_eq!(summary.get("steady_start_tick").and_then(Value::as_u64), Some(3));
        assert_eq!(summary.get("steady_start_raw_block").and_then(Value::as_i64), Some(2));
        assert_eq!(summary.get("steady_start_block").and_then(Value::as_i64), Some(2));
        assert_eq!(summary.get("steady_end_raw_block").and_then(Value::as_i64), Some(12));
        assert_eq!(
            summary.get("search_dwell_window")
                .and_then(Value::as_object)
                .and_then(|value| value.get("mode"))
                .and_then(Value::as_str),
            Some("cycle")
        );
    }

    #[test]
    fn load_split_runs_rehydrates_legacy_piston_search_structure_and_points() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);
        let runs_dir = temp.path.join("runs");
        fs::create_dir_all(&runs_dir).expect("create runs dir");

        fs::write(
            runs_dir.join("index.json"),
            serde_json::to_string_pretty(&json!({
                "updated_at": "2026-06-09T00:00:00Z",
                "latest_run_id": 910003,
                "run_count": 1,
                "runs": [{
                    "run_id": 910003,
                    "file": "run-910003.json",
                    "label": "search result",
                    "display_label": "search result",
                    "source": "reachability-search",
                    "deleted": false
                }]
            }))
            .expect("serialize index"),
        )
        .expect("write index");

        fs::write(
            runs_dir.join("run-910003.json"),
            serde_json::to_string_pretty(&json!({
                "run_id": 910003,
                "label": "search result",
                "display_label": "search result",
                "summary": {
                    "source": "reachability-search",
                    "deleted": false,
                    "launch_mode": "piston",
                    "target_speed": 0.5,
                    "target_dwell_ticks": 2
                },
                "points": [
                    { "tick_index": 0, "x": 0.0, "x_raw": 0.75, "derived_speed": 0.0, "on_ground": true },
                    { "tick_index": 1, "x": 0.0, "x_raw": 0.75, "derived_speed": 0.0, "on_ground": true },
                    { "tick_index": 2, "x": 0.385, "x_raw": 1.135, "derived_speed": 0.385, "on_ground": false },
                    { "tick_index": 3, "x": 1.3611400094032289, "x_raw": 2.1111400094032287, "derived_speed": 0.9761400094032286, "on_ground": true }
                ],
                "structure": {
                    "name": "DI-R2N-DI-F2I-D2B / F2-I_D1-B_S1-B_D1-I_F2-I_D1-B_S1-B",
                    "originX": 0.0,
                    "start": {
                        "x": 1.135,
                        "y": 0.0,
                        "vx": 1.0,
                        "vy": -0.08,
                        "startOnGround": false,
                        "entityIdMod4": 0,
                        "initialTickCount": 2
                    },
                    "launchConfig": {
                        "mode": "piston",
                        "slimeBlockX": -1.0
                    },
                    "prefix": [
                        { "flow": 0, "amount": 0, "floor": "packed_ice", "code": "D-I" },
                        { "surface": 0.7777777777777778, "flow": -1, "amount": 7, "floor": "normal", "code": "R7-N" },
                        { "surface": 0.8888888888888888, "flow": -1, "amount": 8, "floor": "normal", "code": "R8-N" },
                        { "flow": 0, "amount": 0, "floor": "packed_ice", "code": "D-I" },
                        { "surface": 0.8888888888888888, "flow": 1, "amount": 8, "floor": "packed_ice", "code": "F8-I" },
                        { "surface": 0.7777777777777778, "flow": 1, "amount": 7, "floor": "packed_ice", "code": "F7-I" },
                        { "flow": 0, "amount": 0, "floor": "blue_ice", "code": "D-B" },
                        { "flow": 0, "amount": 0, "floor": "blue_ice", "code": "D-B" }
                    ],
                    "cycle": [
                        { "surface": 0.8888888888888888, "flow": 1, "amount": 8, "floor": "packed_ice", "code": "F8-I" },
                        { "surface": 0.7777777777777778, "flow": 1, "amount": 7, "floor": "packed_ice", "code": "F7-I" },
                        { "flow": 0, "amount": 0, "floor": "blue_ice", "code": "D-B" },
                        { "surface": 0.8888888888888888, "flow": 0, "amount": 8, "floor": "blue_ice", "code": "S8-B" }
                    ],
                    "launch": {
                        "mode": "piston",
                        "applied": true,
                        "rawStart": {
                            "x": 0.75,
                            "y": 0.0,
                            "vx": 0.0,
                            "vy": 0.0,
                            "startOnGround": true,
                            "entityIdMod4": 0,
                            "initialTickCount": 0
                        },
                        "effectiveStart": {
                            "x": 1.135,
                            "y": 0.0,
                            "vx": 1.0,
                            "vy": -0.08,
                            "startOnGround": false,
                            "entityIdMod4": 0,
                            "initialTickCount": 2
                        },
                        "timelineOffsetGt": 2,
                        "timelineSamples": [
                            { "gt": 0, "x": 0.75, "y": 0.0, "vx": 0.0, "vy": 0.0, "onGround": true, "pistonCollision": false },
                            { "gt": 1, "x": 0.75, "y": 0.0, "vx": 0.0, "vy": -0.04, "onGround": true, "pistonCollision": false },
                            { "gt": 2, "x": 1.135, "y": 0.0, "vx": 1.0, "vy": -0.08, "onGround": false, "pistonCollision": true }
                        ],
                        "waterwayStartX": 1.0,
                        "slimeBlockX": -1.0
                    }
                }
            }))
            .expect("serialize run"),
        )
        .expect("write run");

        let loaded = store.load_runs().expect("load run with legacy piston search structure");
        let run = &loaded.runs[0];
        let structure = run.structure.as_ref().expect("rehydrated structure");
        assert!((structure.start.x - 0.135).abs() < 1.0e-12);
        assert_eq!(structure.start.initial_tick_count, 2);
        let launch = structure
            .extra
            .get("launch")
            .and_then(Value::as_object)
            .expect("rehydrated launch");
        assert_eq!(
            launch
                .get("effectiveLocalStart")
                .and_then(Value::as_object)
                .and_then(|value| value.get("x"))
                .and_then(Value::as_f64),
            Some(0.135)
        );
        assert_eq!(
            launch.get("displayOriginX").and_then(Value::as_f64),
            Some(1.0)
        );
        assert_eq!(
            launch
                .get("timelineSamples")
                .and_then(Value::as_array)
                .and_then(|samples| samples.get(2))
                .and_then(Value::as_object)
                .and_then(|sample| sample.get("x"))
                .and_then(Value::as_f64),
            Some(0.135)
        );
        assert_eq!(run.points.len(), 4);
        assert!((run.points[3].x_raw.expect("tick 3 raw x") - 2.135).abs() < 1.0e-12);
        assert!((run.points[3].x.expect("tick 3 x") - 2.135).abs() < 1.0e-12);
        assert!((run.points[3].speed.expect("tick 3 speed") - 0.5740000591278076).abs() < 1.0e-12);
    }

    #[test]
    fn save_runs_overwrites_existing_files_and_preserves_delete_restore_purge() {
        let temp = TestDir::new();
        let store = RunStore::new(&temp.path);

        let first = ViewerRunsPayload {
            updated_at: None,
            latest_run_id: None,
            run_count: 0,
            runs: vec![viewer_run(920001, "first")],
        };
        store.save_runs(&first).expect("save first payload");

        let mut second_run = viewer_run(920001, "second");
        second_run.summary.deleted = false;
        let second = ViewerRunsPayload {
            updated_at: None,
            latest_run_id: None,
            run_count: 0,
            runs: vec![second_run],
        };
        store.save_runs(&second).expect("overwrite existing run file");

        let loaded = store.load_runs().expect("load overwritten payload");
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].label.as_deref(), Some("second"));

        let deleted = store.soft_delete_run(920001).expect("soft delete run");
        assert_eq!(deleted.deleted_run_id, 920001);
        let deleted_loaded = store.load_runs().expect("load deleted run");
        assert!(deleted_loaded.runs[0].summary.deleted);

        let restored = store.restore_run(920001).expect("restore run");
        assert_eq!(restored.restored_run_id, 920001);
        let restored_loaded = store.load_runs().expect("load restored run");
        assert!(!restored_loaded.runs[0].summary.deleted);

        let purged = store.purge_run(920001).expect("purge run");
        assert_eq!(purged.permanently_deleted_run_id, 920001);
        assert!(!store.run_file_path(920001).exists());
        let purged_loaded = store.load_runs().expect("load after purge");
        assert_eq!(purged_loaded.runs.len(), 0);
        assert_eq!(purged_loaded.run_count, 0);
    }
}
