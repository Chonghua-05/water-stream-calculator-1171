use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const DEFAULT_MIN_BLOCK: i64 = 1005;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliArgs {
    pub source: PathBuf,
    pub out_json: PathBuf,
    pub out_csv: PathBuf,
    pub min_block: i64,
    pub max_block: Option<i64>,
    pub drop_final_group: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeInfo {
    pub ok: Option<bool>,
    pub source: Option<String>,
    pub in_world: Option<bool>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSample {
    pub tick: i64,
    pub x: f64,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub vx: Option<f64>,
    pub vy: Option<f64>,
    pub vz: Option<f64>,
    pub on_ground: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailurePoint {
    pub tick: i64,
    pub x: f64,
    pub block: i64,
    pub vx: Option<f64>,
    pub y: Option<f64>,
    pub vy: Option<f64>,
    pub on_ground: u8,
    pub boundary_margin: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailureSummary {
    pub length: usize,
    pub block: i64,
    pub t0: i64,
    pub t1: i64,
    pub x0: f64,
    pub x1: f64,
    pub vx0: Option<f64>,
    pub vx1: Option<f64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DwellMetrics {
    pub min_block: i64,
    pub max_block: Option<i64>,
    pub drop_final_group: bool,
    pub blocks: usize,
    pub exact2: usize,
    pub failures: usize,
    pub strict_hit_rate: Option<f64>,
    pub count_dist: BTreeMap<usize, usize>,
    pub min_boundary_margin: Option<f64>,
    pub mean_boundary_margin: Option<f64>,
    pub first_failure: Option<Vec<FailurePoint>>,
    pub failure_lead_in: Option<Vec<FailurePoint>>,
    pub failure_summary: Vec<FailureSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageAnalysis {
    pub label: Option<String>,
    pub source: String,
    pub csv: String,
    pub generated_at: String,
    pub bridge: BridgeInfo,
    pub meta: Option<Value>,
    pub current: Option<Value>,
    pub sample_count: usize,
    pub tick_start: i64,
    pub tick_end: i64,
    pub x_start: f64,
    pub x_end: f64,
    pub dx: f64,
    pub average_speed: Option<f64>,
    pub dwell: DwellMetrics,
    pub first30: Vec<NormalizedSample>,
    pub last30: Vec<NormalizedSample>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryOutput {
    pub analysis: String,
    pub csv: String,
    pub strict_hit_rate: Option<f64>,
    pub failures: usize,
    pub blocks: usize,
    pub count_dist: BTreeMap<usize, usize>,
    pub first_failure: Option<Vec<FailurePoint>>,
}

#[derive(Clone, Debug)]
pub struct AnalysisOutput {
    pub analysis: StorageAnalysis,
    pub summary: SummaryOutput,
}

#[derive(Clone, Debug)]
struct LoadedStoragePayload {
    payload: Value,
    data: Value,
    samples: Vec<NormalizedSample>,
}

#[derive(Clone, Debug)]
struct DwellState {
    sample: NormalizedSample,
    block: i64,
    boundary_margin: f64,
}

pub fn usage() -> String {
    "Usage: item-waterway-solver analyze-game-storage --source <storage.json> [--out-json <file>] [--out-csv <file>] [--min-block <n>] [--max-block <n>] [--drop-final-group|--include-final-group] [--label <text>]".to_string()
}

pub fn main_cli(argv: &[String]) -> Result<(), String> {
    if argv.first().is_some_and(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return Ok(());
    }

    let args = parse_args(argv)?;
    let output = analyze_source_to_outputs(&args)?;
    write_csv(&args.out_csv, &load_storage_payload(&args.source)?.samples)?;
    write_analysis_json(&args.out_json, &output.analysis)?;
    let summary = serde_json::to_string_pretty(&output.summary)
        .map_err(|error| format!("Failed to serialize summary JSON: {error}"))?;
    println!("{summary}");
    Ok(())
}

pub fn analyze_source_to_outputs(args: &CliArgs) -> Result<AnalysisOutput, String> {
    let loaded = load_storage_payload(&args.source)?;
    let analysis = analyze_loaded_payload(&loaded, args)?;
    let summary = SummaryOutput {
        analysis: args.out_json.display().to_string(),
        csv: args.out_csv.display().to_string(),
        strict_hit_rate: analysis.dwell.strict_hit_rate,
        failures: analysis.dwell.failures,
        blocks: analysis.dwell.blocks,
        count_dist: analysis.dwell.count_dist.clone(),
        first_failure: analysis.dwell.first_failure.clone(),
    };
    Ok(AnalysisOutput { analysis, summary })
}

fn parse_args(argv: &[String]) -> Result<CliArgs, String> {
    let mut source: Option<PathBuf> = None;
    let mut out_json: Option<PathBuf> = None;
    let mut out_csv: Option<PathBuf> = None;
    let mut min_block = DEFAULT_MIN_BLOCK;
    let mut max_block = None;
    let mut drop_final_group = false;
    let mut label = None;

    let mut index = 0;
    while index < argv.len() {
        let arg = argv[index].as_str();
        let next = |cursor: &mut usize, name: &str| -> Result<&str, String> {
            *cursor += 1;
            argv.get(*cursor)
                .map(|value| value.as_str())
                .ok_or_else(|| format!("Missing value for {name}"))
        };
        match arg {
            "--source" => source = Some(resolve_cli_path(next(&mut index, "--source")?)?),
            "--out-json" => out_json = Some(resolve_cli_path(next(&mut index, "--out-json")?)?),
            "--out-csv" => out_csv = Some(resolve_cli_path(next(&mut index, "--out-csv")?)?),
            "--min-block" => min_block = parse_i64_arg(next(&mut index, "--min-block")?, "--min-block")?,
            "--max-block" => {
                max_block = Some(parse_i64_arg(
                    next(&mut index, "--max-block")?,
                    "--max-block",
                )?)
            }
            "--include-final-group" => drop_final_group = false,
            "--drop-final-group" => drop_final_group = true,
            "--label" => label = Some(next(&mut index, "--label")?.to_string()),
            other => return Err(format!("Unknown argument: {other}")),
        }
        index += 1;
    }

    let source = source.ok_or_else(|| "--source is required".to_string())?;
    let out_json = out_json.unwrap_or_else(|| default_output_path(&source, "-analysis.json"));
    let out_csv = out_csv.unwrap_or_else(|| default_output_path(&source, "-samples.csv"));
    Ok(CliArgs {
        source,
        out_json,
        out_csv,
        min_block,
        max_block,
        drop_final_group,
        label,
    })
}

fn resolve_cli_path(text: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(text);
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| format!("Failed to read current directory: {error}"))
    }
}

fn parse_i64_arg(text: &str, name: &str) -> Result<i64, String> {
    text.parse::<i64>()
        .map_err(|error| format!("Invalid {name} value '{text}': {error}"))
}

fn default_output_path(source: &Path, suffix: &str) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("storage");
    parent.join(format!("{stem}{suffix}"))
}

fn load_storage_payload(source: &Path) -> Result<LoadedStoragePayload, String> {
    let text =
        fs::read_to_string(source).map_err(|error| format!("Failed to read {}: {error}", source.display()))?;
    let payload: Value =
        serde_json::from_str(strip_utf8_bom(&text))
            .map_err(|error| format!("Invalid JSON in {}: {error}", source.display()))?;
    let data = payload.get("data").cloned().unwrap_or_else(|| payload.clone());
    let raw_samples = data
        .get("samples")
        .or_else(|| payload.get("samples"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut samples = raw_samples
        .iter()
        .filter_map(normalize_storage_sample)
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.tick);

    Ok(LoadedStoragePayload {
        payload,
        data,
        samples,
    })
}

fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn normalize_storage_sample(sample: &Value) -> Option<NormalizedSample> {
    let pos = sample.get("pos").or_else(|| sample.get("Pos"))?;
    let motion = sample.get("motion").or_else(|| sample.get("Motion"));
    let tick = numeric_i64(sample.get("tick").or_else(|| sample.get("Tick"))?)?;
    let x = array_numeric(pos, 0)?;

    Some(NormalizedSample {
        tick,
        x,
        y: array_numeric(pos, 1),
        z: array_numeric(pos, 2),
        vx: motion.and_then(|value| array_numeric(value, 0)),
        vy: motion.and_then(|value| array_numeric(value, 1)),
        vz: motion.and_then(|value| array_numeric(value, 2)),
        on_ground: sample
            .get("on_ground")
            .or_else(|| sample.get("OnGround"))
            .and_then(numeric_i64)
            .unwrap_or(0)
            .clamp(0, 1) as u8,
    })
}

fn numeric_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Some(value)
            } else {
                number.as_f64().map(|value| value as i64)
            }
        }
        Value::String(text) => parse_numeric_text(text).map(|value| value as i64),
        _ => None,
    }
}

fn array_numeric(value: &Value, index: usize) -> Option<f64> {
    value.as_array()
        .and_then(|items| items.get(index))
        .and_then(numeric_f64)
        .filter(|number| number.is_finite())
}

fn numeric_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_numeric_text(text),
        _ => None,
    }
}

fn parse_numeric_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .parse::<f64>()
        .ok()
        .or_else(|| trimmed.trim_end_matches(['b', 'B', 'd', 'D', 'f', 'F', 'l', 'L', 's', 'S']).parse().ok())
}

fn analyze_loaded_payload(
    loaded: &LoadedStoragePayload,
    args: &CliArgs,
) -> Result<StorageAnalysis, String> {
    if loaded.samples.is_empty() {
        return Err(format!("No samples found in {}", args.source.display()));
    }

    let dwell = dwell_metrics(&loaded.samples, args);
    let start = &loaded.samples[0];
    let end = &loaded.samples[loaded.samples.len() - 1];
    let tick_delta = end.tick - start.tick;
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    Ok(StorageAnalysis {
        label: args.label.clone(),
        source: args.source.display().to_string(),
        csv: args.out_csv.display().to_string(),
        generated_at,
        bridge: BridgeInfo {
            ok: loaded.payload.get("ok").and_then(Value::as_bool),
            source: loaded
                .payload
                .get("source")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            in_world: loaded.payload.get("inWorld").and_then(Value::as_bool),
            id: loaded
                .payload
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        },
        meta: loaded.data.get("meta").cloned(),
        current: loaded.data.get("current").cloned(),
        sample_count: loaded.samples.len(),
        tick_start: start.tick,
        tick_end: end.tick,
        x_start: start.x,
        x_end: end.x,
        dx: end.x - start.x,
        average_speed: (tick_delta > 0).then_some((end.x - start.x) / tick_delta as f64),
        dwell,
        first30: loaded.samples.iter().take(30).cloned().collect(),
        last30: loaded
            .samples
            .iter()
            .skip(loaded.samples.len().saturating_sub(30))
            .cloned()
            .collect(),
    })
}

fn dwell_metrics(samples: &[NormalizedSample], args: &CliArgs) -> DwellMetrics {
    let groups = build_dwell_groups(samples, args.drop_final_group);
    let eligible = groups
        .into_iter()
        .filter(|group| {
            let block = group[0].block;
            block >= args.min_block && args.max_block.is_none_or(|max_block| block <= max_block)
        })
        .collect::<Vec<_>>();

    let mut count_dist = BTreeMap::new();
    let mut failure_groups = Vec::new();
    let mut min_boundary_margin = f64::INFINITY;
    let mut mean_boundary_margin = 0.0;
    let mut state_count = 0usize;

    for (index, group) in eligible.iter().enumerate() {
        *count_dist.entry(group.len()).or_insert(0) += 1;
        for state in group {
            min_boundary_margin = min_boundary_margin.min(state.boundary_margin);
            mean_boundary_margin += state.boundary_margin;
            state_count += 1;
        }
        if group.len() != 2 {
            failure_groups.push((index, group.clone()));
        }
    }

    let exact2 = eligible.len().saturating_sub(failure_groups.len());
    let first_failure = failure_groups
        .first()
        .map(|(_, group)| failure_points(group.as_slice()));
    let failure_lead_in = failure_groups.first().and_then(|(index, _)| {
        index
            .checked_sub(1)
            .and_then(|prev| eligible.get(prev))
            .map(|group| failure_points(group.as_slice()))
    });

    DwellMetrics {
        min_block: args.min_block,
        max_block: args.max_block,
        drop_final_group: args.drop_final_group,
        blocks: eligible.len(),
        exact2,
        failures: failure_groups.len(),
        strict_hit_rate: (!eligible.is_empty()).then_some(exact2 as f64 / eligible.len() as f64),
        count_dist,
        min_boundary_margin: state_count
            .gt(&0)
            .then_some(min_boundary_margin)
            .filter(|value| value.is_finite()),
        mean_boundary_margin: (state_count > 0).then_some(mean_boundary_margin / state_count as f64),
        first_failure,
        failure_lead_in,
        failure_summary: failure_groups
            .iter()
            .take(48)
            .map(|(_, group)| FailureSummary {
                length: group.len(),
                block: group[0].block,
                t0: group[0].sample.tick,
                t1: group[group.len() - 1].sample.tick,
                x0: group[0].sample.x,
                x1: group[group.len() - 1].sample.x,
                vx0: group[0].sample.vx,
                vx1: group[group.len() - 1].sample.vx,
            })
            .collect(),
    }
}

fn build_dwell_groups(samples: &[NormalizedSample], drop_final_group: bool) -> Vec<Vec<DwellState>> {
    let mut groups = Vec::new();
    let mut current: Vec<DwellState> = Vec::new();

    for sample in samples {
        let block = sample.x.floor() as i64;
        let state = DwellState {
            sample: sample.clone(),
            block,
            boundary_margin: distance_to_integer_boundary(sample.x),
        };
        if current.is_empty() || block == current[current.len() - 1].block {
            current.push(state);
        } else {
            groups.push(current);
            current = vec![state];
        }
    }
    if !drop_final_group && !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn distance_to_integer_boundary(value: f64) -> f64 {
    let fraction = value - value.floor();
    fraction.min(1.0 - fraction)
}

fn failure_points(group: &[DwellState]) -> Vec<FailurePoint> {
    group
        .iter()
        .map(|state| FailurePoint {
            tick: state.sample.tick,
            x: state.sample.x,
            block: state.block,
            vx: state.sample.vx,
            y: state.sample.y,
            vy: state.sample.vy,
            on_ground: state.sample.on_ground,
            boundary_margin: state.boundary_margin,
        })
        .collect()
}

fn write_analysis_json(path: &Path, analysis: &StorageAnalysis) -> Result<(), String> {
    ensure_parent(path)?;
    let json = serde_json::to_string_pretty(analysis)
        .map_err(|error| format!("Failed to serialize analysis JSON: {error}"))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn write_csv(path: &Path, samples: &[NormalizedSample]) -> Result<(), String> {
    ensure_parent(path)?;
    let mut rows = Vec::with_capacity(samples.len() + 1);
    rows.push("tick,x,y,z,vx,vy,vz,on_ground,block".to_string());
    for sample in samples {
        let row = [
            csv_field(sample.tick),
            csv_field(sample.x),
            csv_field(sample.y),
            csv_field(sample.z),
            csv_field(sample.vx),
            csv_field(sample.vy),
            csv_field(sample.vz),
            csv_field(sample.on_ground),
            csv_field(sample.x.floor() as i64),
        ]
        .join(",");
        rows.push(row);
    }
    fs::write(path, format!("{}\n", rows.join("\n")))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn csv_field(value: impl Into<CsvField>) -> String {
    let text = match value.into() {
        CsvField::Empty => String::new(),
        CsvField::Text(text) => text,
    };
    if text.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

enum CsvField {
    Empty,
    Text(String),
}

impl From<i64> for CsvField {
    fn from(value: i64) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<u8> for CsvField {
    fn from(value: u8) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<f64> for CsvField {
    fn from(value: f64) -> Self {
        if value.is_finite() {
            Self::Text(value.to_string())
        } else {
            Self::Empty
        }
    }
}

impl From<Option<f64>> for CsvField {
    fn from(value: Option<f64>) -> Self {
        match value {
            Some(value) if value.is_finite() => Self::Text(value.to_string()),
            _ => Self::Empty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "item-waterway-storage-analysis-{}-{}",
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

    #[test]
    fn dwell_metrics_match_old_failure_shape() {
        let samples = vec![
            NormalizedSample {
                tick: 1,
                x: 1005.1,
                y: Some(101.0),
                z: Some(0.5),
                vx: Some(0.5),
                vy: Some(0.0),
                vz: Some(0.0),
                on_ground: 1,
            },
            NormalizedSample {
                tick: 2,
                x: 1005.8,
                y: Some(101.0),
                z: Some(0.5),
                vx: Some(0.5),
                vy: Some(0.0),
                vz: Some(0.0),
                on_ground: 1,
            },
            NormalizedSample {
                tick: 3,
                x: 1006.2,
                y: Some(101.0),
                z: Some(0.5),
                vx: Some(0.5),
                vy: Some(0.0),
                vz: Some(0.0),
                on_ground: 1,
            },
        ];
        let args = CliArgs {
            source: PathBuf::from("dummy.json"),
            out_json: PathBuf::from("dummy-analysis.json"),
            out_csv: PathBuf::from("dummy-samples.csv"),
            min_block: 1005,
            max_block: None,
            drop_final_group: false,
            label: None,
        };

        let dwell = dwell_metrics(&samples, &args);
        assert_eq!(dwell.blocks, 2);
        assert_eq!(dwell.exact2, 1);
        assert_eq!(dwell.failures, 1);
        assert_eq!(dwell.strict_hit_rate, Some(0.5));
        assert_eq!(dwell.count_dist.get(&1), Some(&1));
        assert_eq!(dwell.count_dist.get(&2), Some(&1));
        assert_eq!(
            dwell.first_failure.as_ref().and_then(|group| group.first()).map(|point| point.block),
            Some(1006)
        );
    }

    #[test]
    fn analyze_source_writes_json_and_csv() {
        let temp = TestDir::new();
        let source = temp.path.join("storage.json");
        fs::write(
            &source,
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "source": "server",
                "inWorld": true,
                "id": "codex:test",
                "data": {
                    "meta": { "layout": "test" },
                    "current": { "tick": 3 },
                    "samples": [
                        { "tick": 1, "pos": [1005.1, 101.0, 0.5], "motion": [0.5, 0.0, 0.0], "on_ground": 1 },
                        { "tick": 2, "pos": [1005.7, 101.0, 0.5], "motion": [0.5, 0.0, 0.0], "on_ground": 1 },
                        { "tick": 3, "pos": [1006.2, 101.0, 0.5], "motion": [0.5, 0.0, 0.0], "on_ground": 1 }
                    ]
                }
            }))
            .expect("serialize source payload"),
        )
        .expect("write source payload");

        let args = CliArgs {
            source: source.clone(),
            out_json: temp.path.join("out").join("analysis.json"),
            out_csv: temp.path.join("out").join("samples.csv"),
            min_block: 1005,
            max_block: None,
            drop_final_group: false,
            label: Some("fixture".to_string()),
        };

        let output = analyze_source_to_outputs(&args).expect("analyze source");
        write_csv(&args.out_csv, &load_storage_payload(&source).expect("load source").samples)
            .expect("write csv");
        write_analysis_json(&args.out_json, &output.analysis).expect("write analysis json");

        assert_eq!(output.summary.failures, 1);
        assert_eq!(output.summary.blocks, 2);
        assert!(args.out_json.exists());
        assert!(args.out_csv.exists());

        let analysis_text = fs::read_to_string(&args.out_json).expect("read analysis json");
        assert!(analysis_text.contains("\"strictHitRate\": 0.5"));

        let csv_text = fs::read_to_string(&args.out_csv).expect("read samples csv");
        assert!(csv_text.starts_with("tick,x,y,z,vx,vy,vz,on_ground,block\n"));
        assert!(csv_text.contains("1005"));
    }

    #[test]
    fn load_storage_payload_accepts_utf8_bom() {
        let temp = TestDir::new();
        let source = temp.path.join("storage-bom.json");
        fs::write(
            &source,
            "\u{feff}{\"data\":{\"samples\":[{\"tick\":1,\"pos\":[1005.1,101.0,0.5]}]}}",
        )
        .expect("write bom source payload");

        let loaded = load_storage_payload(&source).expect("load payload with bom");
        assert_eq!(loaded.samples.len(), 1);
        assert_eq!(loaded.samples[0].tick, 1);
        assert_eq!(loaded.samples[0].x, 1005.1);
    }
}
