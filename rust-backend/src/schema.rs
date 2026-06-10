use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEFAULT_START_X: f64 = 0.125;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FloorName(String);

impl FloorName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn canonical_kind(&self) -> CanonicalFloor {
        match self.0.trim().to_ascii_lowercase().as_str() {
            "packed_ice" | "ice" | "frosted_ice" => CanonicalFloor::PackedIce,
            "blue_ice" => CanonicalFloor::BlueIce,
            "slime" | "slime_block" => CanonicalFloor::Slime,
            _ => CanonicalFloor::Normal,
        }
    }

    pub fn texture_name(&self) -> &'static str {
        match self.0.trim().to_ascii_lowercase().as_str() {
            "glass" => "glass",
            "ice" | "frosted_ice" => "ice",
            "packed_ice" => "packed_ice",
            "blue_ice" => "blue_ice",
            "slime" | "slime_block" => "slime_block",
            _ => "stone",
        }
    }

    pub fn friction(&self) -> f64 {
        match self.canonical_kind() {
            CanonicalFloor::Normal => 0.6_f32 as f64,
            CanonicalFloor::PackedIce => 0.98_f32 as f64,
            CanonicalFloor::BlueIce => 0.989_f32 as f64,
            CanonicalFloor::Slime => 0.8_f32 as f64,
        }
    }

    pub fn step_on(&self) -> &'static str {
        match self.canonical_kind() {
            CanonicalFloor::Slime => "slime",
            _ => "none",
        }
    }

    pub fn canonical_code_suffix(&self) -> char {
        match self.canonical_kind() {
            CanonicalFloor::Normal => 'N',
            CanonicalFloor::PackedIce => 'I',
            CanonicalFloor::BlueIce => 'B',
            CanonicalFloor::Slime => 'S',
        }
    }
}

impl Default for FloorName {
    fn default() -> Self {
        Self::new("normal")
    }
}

impl From<&str> for FloorName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for FloorName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalFloor {
    Normal,
    PackedIce,
    BlueIce,
    Slime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<f64>,
    #[serde(default)]
    pub flow: i8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u8>,
    #[serde(default)]
    pub floor: FloorName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friction: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            surface: None,
            flow: 0,
            amount: Some(0),
            floor: FloorName::default(),
            friction: None,
            step_on: None,
            code: None,
            texture: None,
            extra: BTreeMap::new(),
        }
    }
}

impl Cell {
    pub fn canonical_amount(&self) -> u8 {
        self.amount.unwrap_or_else(|| {
            self.surface
                .map(|surface| (surface * 9.0).round().clamp(0.0, 8.0) as u8)
                .unwrap_or(0)
        })
    }

    pub fn canonical_surface(&self) -> Option<f64> {
        match (self.surface, self.canonical_amount()) {
            (Some(surface), _) => Some(surface),
            (None, 0) => None,
            (None, amount) => Some(amount as f64 / 9.0),
        }
    }

    pub fn canonical_flow(&self) -> i8 {
        if self.flow == 0 {
            self.code
                .as_deref()
                .map(flow_from_code)
                .unwrap_or(0)
                .clamp(-1, 1)
        } else {
            self.flow.clamp(-1, 1)
        }
    }

    pub fn derived_code(&self) -> String {
        let amount = self.canonical_amount();
        let suffix = self.floor.canonical_code_suffix();
        if self.canonical_surface().is_none() {
            format!("D-{suffix}")
        } else if self.canonical_flow() < 0 {
            format!("R{amount}-{suffix}")
        } else if self.canonical_flow() > 0 {
            format!("F{amount}-{suffix}")
        } else {
            format!("S{amount}-{suffix}")
        }
    }

    pub fn normalized(mut self) -> Self {
        self.amount = Some(self.canonical_amount().min(8));
        self.surface = self.canonical_surface();
        self.flow = self.canonical_flow();
        if self.friction.is_none() {
            self.friction = Some(self.floor.friction());
        }
        if self.step_on.is_none() {
            self.step_on = Some(self.floor.step_on().to_string());
        }
        if self.texture.is_none() {
            self.texture = Some(self.floor.texture_name().to_string());
        }
        if self.code.is_none() {
            self.code = Some(self.derived_code());
        }
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartState {
    #[serde(default = "default_start_x")]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub vx: f64,
    #[serde(default)]
    pub vy: f64,
    #[serde(default)]
    pub start_on_ground: Option<bool>,
    #[serde(default)]
    pub entity_id_mod4: usize,
    #[serde(default)]
    pub initial_tick_count: usize,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for StartState {
    fn default() -> Self {
        Self {
            x: default_start_x(),
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            start_on_ground: Some(true),
            entity_id_mod4: 0,
            initial_tick_count: 0,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slime_block_x: Option<f64>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub origin_x: f64,
    #[serde(default)]
    pub start: StartState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_config: Option<LaunchConfig>,
    #[serde(default)]
    pub prefix: Vec<Cell>,
    #[serde(default)]
    pub cycle: Vec<Cell>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Structure {
    fn default() -> Self {
        Self {
            name: Some("default-w3-cycle".to_string()),
            origin_x: 0.0,
            start: StartState::default(),
            launch_config: None,
            prefix: Vec::new(),
            cycle: default_cycle_cells(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    #[serde(default)]
    pub params: SearchParams,
    #[serde(default)]
    pub options: SearchOptions,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    #[serde(flatten, default)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    #[serde(flatten, default)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationRequest {
    pub ticks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(default)]
    pub structures: Vec<Structure>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewerPoint {
    #[serde(default, alias = "tickIndex")]
    pub tick_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, alias = "xRaw", skip_serializing_if = "Option::is_none")]
    pub x_raw: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, alias = "derivedSpeed", skip_serializing_if = "Option::is_none")]
    pub derived_speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<FloorName>,
    #[serde(default, alias = "onGround", skip_serializing_if = "Option::is_none")]
    pub on_ground: Option<bool>,
    #[serde(default, alias = "logTime", skip_serializing_if = "Option::is_none")]
    pub log_time: Option<String>,
    #[serde(default, alias = "capturedAt", skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(default, alias = "rawLine", skip_serializing_if = "Option::is_none")]
    pub raw_line: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewerRunSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, alias = "modelEngine", skip_serializing_if = "Option::is_none")]
    pub model_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<String>,
    #[serde(default, alias = "structureCount", skip_serializing_if = "Option::is_none")]
    pub structure_count: Option<usize>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, alias = "launchMode", skip_serializing_if = "Option::is_none")]
    pub launch_mode: Option<String>,
    #[serde(default, alias = "equivalentFingerprint", skip_serializing_if = "Option::is_none")]
    pub equivalent_fingerprint: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewerRun {
    #[serde(default, alias = "runId", skip_serializing_if = "Option::is_none")]
    pub run_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, alias = "displayLabel", skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    #[serde(default)]
    pub summary: ViewerRunSummary,
    #[serde(default)]
    pub points: Vec<ViewerPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunsIndexEntry {
    #[serde(alias = "runId")]
    pub run_id: u64,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, alias = "displayLabel", skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunsIndex {
    #[serde(default, alias = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, alias = "latestRunId", skip_serializing_if = "Option::is_none")]
    pub latest_run_id: Option<u64>,
    #[serde(default, alias = "runCount")]
    pub run_count: usize,
    #[serde(default)]
    pub runs: Vec<RunsIndexEntry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewerRunsPayload {
    #[serde(default, alias = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, alias = "latestRunId", skip_serializing_if = "Option::is_none")]
    pub latest_run_id: Option<u64>,
    #[serde(default, alias = "runCount")]
    pub run_count: usize,
    #[serde(default)]
    pub runs: Vec<ViewerRun>,
}

pub fn default_start_x() -> f64 {
    DEFAULT_START_X
}

pub fn default_structure() -> Structure {
    Structure::default()
}

pub fn flow_from_code(code: &str) -> i8 {
    match code.trim().to_ascii_uppercase().chars().next() {
        Some('F') => 1,
        Some('R') => -1,
        _ => 0,
    }
}

pub fn make_cell(
    surface: Option<f64>,
    flow: i8,
    floor: impl Into<FloorName>,
    code: Option<String>,
    amount: Option<u8>,
) -> Cell {
    Cell {
        surface,
        flow,
        amount,
        floor: floor.into(),
        friction: None,
        step_on: None,
        code,
        texture: None,
        extra: BTreeMap::new(),
    }
    .normalized()
}

pub fn default_cycle_cells() -> Vec<Cell> {
    vec![
        make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
        make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
        make_cell(Some(6.0 / 9.0), 1, "packed_ice", None, Some(6)),
        make_cell(None, 0, "blue_ice", None, Some(0)),
        make_cell(None, 0, "blue_ice", None, Some(0)),
        make_cell(None, 0, "blue_ice", None, Some(0)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_cycle_matches_python_default_w3_cycle() {
        let cells = default_cycle_cells();
        assert_eq!(cells.len(), 6);

        assert_eq!(cells[0].surface, Some(8.0 / 9.0));
        assert_eq!(cells[0].flow, 1);
        assert_eq!(cells[0].amount, Some(8));
        assert_eq!(cells[0].floor.as_str(), "packed_ice");
        assert_eq!(cells[0].code.as_deref(), Some("F8-I"));

        assert_eq!(cells[1].surface, Some(7.0 / 9.0));
        assert_eq!(cells[1].flow, 1);
        assert_eq!(cells[1].amount, Some(7));
        assert_eq!(cells[1].floor.as_str(), "packed_ice");
        assert_eq!(cells[1].code.as_deref(), Some("F7-I"));

        assert_eq!(cells[2].surface, Some(6.0 / 9.0));
        assert_eq!(cells[2].flow, 1);
        assert_eq!(cells[2].amount, Some(6));
        assert_eq!(cells[2].floor.as_str(), "packed_ice");
        assert_eq!(cells[2].code.as_deref(), Some("F6-I"));

        for cell in &cells[3..] {
            assert_eq!(cell.surface, None);
            assert_eq!(cell.flow, 0);
            assert_eq!(cell.amount, Some(0));
            assert_eq!(cell.floor.as_str(), "blue_ice");
            assert_eq!(cell.code.as_deref(), Some("D-B"));
        }
    }

    #[test]
    fn viewer_runs_payload_keeps_snake_case_and_accepts_camel_case_aliases() {
        let payload = ViewerRunsPayload {
            updated_at: Some("2026-06-05T00:00:00Z".to_string()),
            latest_run_id: Some(920010),
            run_count: 1,
            runs: vec![ViewerRun {
                run_id: Some(920010),
                label: Some("compat".to_string()),
                display_label: Some("Compat".to_string()),
                summary: ViewerRunSummary {
                    source: Some("test".to_string()),
                    model_engine: Some("rust".to_string()),
                    structure: Some("demo".to_string()),
                    structure_count: Some(1),
                    deleted: false,
                    launch_mode: Some("piston".to_string()),
                    equivalent_fingerprint: Some("abc".to_string()),
                    extra: BTreeMap::new(),
                },
                points: vec![ViewerPoint {
                    tick_index: 7,
                    x: Some(1.0),
                    x_raw: Some(1.125),
                    speed: Some(0.5),
                    derived_speed: Some(0.5),
                    y: None,
                    vy: None,
                    floor: None,
                    on_ground: Some(true),
                    log_time: None,
                    captured_at: None,
                    raw_line: None,
                    extra: BTreeMap::new(),
                }],
                structure: None,
                extra: BTreeMap::new(),
            }],
        };

        let value = serde_json::to_value(&payload).expect("serialize viewer runs payload");
        assert_eq!(value.get("run_count").and_then(Value::as_u64), Some(1));
        assert_eq!(value.get("latest_run_id").and_then(Value::as_u64), Some(920010));
        let first_run = value
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|runs| runs.first())
            .expect("first run");
        assert_eq!(first_run.get("run_id").and_then(Value::as_u64), Some(920010));
        assert_eq!(
            first_run.get("display_label").and_then(Value::as_str),
            Some("Compat")
        );
        let summary = first_run.get("summary").expect("summary");
        assert_eq!(
            summary.get("model_engine").and_then(Value::as_str),
            Some("rust")
        );
        assert_eq!(
            summary.get("structure_count").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary.get("launch_mode").and_then(Value::as_str),
            Some("piston")
        );
        assert_eq!(
            summary
                .get("equivalent_fingerprint")
                .and_then(Value::as_str),
            Some("abc")
        );
        let first_point = first_run
            .get("points")
            .and_then(Value::as_array)
            .and_then(|points| points.first())
            .expect("first point");
        assert_eq!(first_point.get("tick_index").and_then(Value::as_u64), Some(7));
        assert_eq!(first_point.get("x_raw").and_then(Value::as_f64), Some(1.125));
        assert_eq!(
            first_point.get("derived_speed").and_then(Value::as_f64),
            Some(0.5)
        );
        assert_eq!(
            first_point.get("on_ground").and_then(Value::as_bool),
            Some(true)
        );

        let alias_json = json!({
            "updatedAt": "2026-06-05T00:00:00Z",
            "latestRunId": 920010,
            "runCount": 1,
            "runs": [{
                "runId": 920010,
                "label": "compat",
                "displayLabel": "Compat",
                "summary": {
                    "source": "test",
                    "modelEngine": "rust",
                    "structure": "demo",
                    "structureCount": 1,
                    "deleted": false,
                    "launchMode": "piston",
                    "equivalentFingerprint": "abc"
                },
                "points": [{
                    "tickIndex": 7,
                    "x": 1.0,
                    "xRaw": 1.125,
                    "speed": 0.5,
                    "derivedSpeed": 0.5,
                    "onGround": true
                }]
            }]
        });

        let parsed: ViewerRunsPayload =
            serde_json::from_value(alias_json).expect("deserialize camelCase aliases");
        assert_eq!(parsed.run_count, 1);
        assert_eq!(parsed.latest_run_id, Some(920010));
        assert_eq!(parsed.runs[0].run_id, Some(920010));
        assert_eq!(parsed.runs[0].display_label.as_deref(), Some("Compat"));
        assert_eq!(parsed.runs[0].summary.model_engine.as_deref(), Some("rust"));
        assert_eq!(parsed.runs[0].summary.structure.as_deref(), Some("demo"));
        assert_eq!(parsed.runs[0].summary.structure_count, Some(1));
        assert_eq!(parsed.runs[0].summary.launch_mode.as_deref(), Some("piston"));
        assert_eq!(
            parsed.runs[0].summary.equivalent_fingerprint.as_deref(),
            Some("abc")
        );
        assert_eq!(parsed.runs[0].points[0].tick_index, 7);
        assert_eq!(parsed.runs[0].points[0].x_raw, Some(1.125));
        assert_eq!(parsed.runs[0].points[0].derived_speed, Some(0.5));
        assert_eq!(parsed.runs[0].points[0].on_ground, Some(true));
    }
}
