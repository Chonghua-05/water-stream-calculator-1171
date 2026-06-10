use crate::litematic::{
    self, LitematicExportOptions, LitematicImportOptions,
};
use crate::run_store::RunStore;
use crate::schema;
use crate::search_tasks::{
    SearchTaskContext, SearchTaskEnvelope, SearchTaskError, SearchTaskManager, SearchTaskProgress,
};
use crate::viewer_runs::{self, ViewerRunOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const BACKEND_NAME: &str = "item-waterway-solver-rust";
const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8766;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusPayload {
    pub ok: bool,
    pub backend: &'static str,
    pub viewer_data: String,
    pub runs_index: String,
    pub legacy_runs_json: String,
    pub game_capture: &'static str,
    pub fabric_bridge: &'static str,
    pub model_engine: &'static str,
}

pub fn status_payload() -> StatusPayload {
    let viewer_data = resolve_viewer_data_dir();
    let runs_index = if let Some(path) = viewer_data.as_ref() {
        path.join("runs").join("index.json").display().to_string()
    } else {
        String::new()
    };
    let legacy_runs_json = if let Some(path) = viewer_data.as_ref() {
        path.join("runs.json").display().to_string()
    } else {
        String::new()
    };
    StatusPayload {
        ok: true,
        backend: "rust",
        viewer_data: viewer_data
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        runs_index,
        legacy_runs_json,
        game_capture: "disabled",
        fabric_bridge: "disabled",
        model_engine: BACKEND_NAME,
    }
}

pub fn status_handler() -> StatusPayload {
    status_payload()
}

pub trait SearchTaskService: Send + Sync {
    fn start_search_task(&self, request: Value) -> Result<SearchTaskEnvelope, String>;
    fn get_search_task(&self, task_id: &str) -> Result<SearchTaskEnvelope, String>;
    fn cancel_search_task(&self, task_id: &str) -> Result<SearchTaskEnvelope, String>;
}

pub trait SimulationAdapter: Send + Sync {
    fn simulate(&self, request: Value) -> Result<Value, String>;
}

pub trait CompareAdapter: Send + Sync {
    fn compare(&self, request: Value) -> Result<Value, String>;
}

pub trait LitematicAdapter: Send + Sync {
    fn import_litematic(
        &self,
        bytes: &[u8],
        options: &BTreeMap<String, Value>,
    ) -> Result<Value, String>;
    fn export_litematic(
        &self,
        request: Value,
    ) -> Result<(Vec<u8>, String, &'static str), String>;
}

pub trait RunsAdapter: Send + Sync {
    fn load_runs(&self) -> Result<Value, String>;
    fn soft_delete_run(&self, run_id: u64) -> Result<Value, String>;
    fn restore_run(&self, run_id: u64) -> Result<Value, String>;
    fn purge_run(&self, run_id: u64) -> Result<Value, String>;
}

#[derive(Clone)]
pub struct ServiceAdapters {
    search: Arc<dyn SearchTaskService>,
    simulation: Arc<dyn SimulationAdapter>,
    compare: Arc<dyn CompareAdapter>,
    litematic: Arc<dyn LitematicAdapter>,
    runs: Arc<dyn RunsAdapter>,
}

impl Default for ServiceAdapters {
    fn default() -> Self {
        Self {
            search: Arc::new(ManagedSearchTaskService::new(
                resolve_viewer_data_dir().map(RunStore::new),
            )),
            simulation: Arc::new(RustSimulationAdapter::new(
                resolve_viewer_data_dir().map(RunStore::new),
            )),
            compare: Arc::new(RustCompareAdapter),
            litematic: Arc::new(RustLitematicAdapter),
            runs: Arc::new(RunStoreAdapter::from_env()),
        }
    }
}

impl ServiceAdapters {
    #[cfg(test)]
    fn with_viewer_data_dir(viewer_data_dir: PathBuf) -> Self {
        let store = Some(RunStore::new(viewer_data_dir));
        Self {
            search: Arc::new(ManagedSearchTaskService::new(store.clone())),
            simulation: Arc::new(RustSimulationAdapter::new(store.clone())),
            compare: Arc::new(RustCompareAdapter),
            litematic: Arc::new(RustLitematicAdapter),
            runs: Arc::new(RunStoreAdapter {
                store: store.clone(),
            }),
        }
    }
}

pub fn serve_web() -> Result<(), String> {
    serve_web_with_adapters(ServiceAdapters::default())
}

pub fn serve_web_with_adapters(adapters: ServiceAdapters) -> Result<(), String> {
    let host = env::var("MC_VIEWER_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let port = env::var("MC_VIEWER_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let bind_addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&bind_addr)
        .map_err(|error| format!("Failed to bind {bind_addr}: {error}"))?;
    println!("Rust web service listening on http://{bind_addr}");
    let adapters = Arc::new(adapters);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let adapters = Arc::clone(&adapters);
                thread::spawn(move || {
                    let _ = handle_connection(stream, adapters);
                });
            }
            Err(error) => eprintln!("Accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, adapters: Arc<ServiceAdapters>) -> Result<(), String> {
    let request = HttpRequest::read_from(&mut stream)?;
    let response = route_request(request, adapters);
    response.write_to(&mut stream)
}

fn route_request(request: HttpRequest, adapters: Arc<ServiceAdapters>) -> HttpResponse {
    let path_only = request
        .path
        .split('?')
        .next()
        .unwrap_or(request.path.as_str());
    let segments = split_path(path_only);
    match (request.method.as_str(), path_only) {
        ("GET", "/api/status") => json_response(200, &status_handler()),
        ("GET", "/api/model/default-structure") => {
            json_response(200, &json!({ "ok": true, "structure": schema::default_structure() }))
        }
        ("GET", "/api/runs") => match adapters.runs.load_runs() {
            Ok(payload) => value_response(200, payload),
            Err(error) => error_response(500, "runs_load_failed", &error),
        },
        ("POST", "/api/model/search") => match read_json_body(&request) {
            Ok(body) => match adapters.search.start_search_task(body) {
                Ok(payload) => json_response(200, &payload),
                Err(error) => error_response(500, "search_failed", &error),
            },
            Err(error) => error_response(400, "invalid_json", &error),
        },
        ("POST", "/api/model/simulate") => match read_json_body(&request) {
            Ok(body) => match adapters.simulation.simulate(body) {
                Ok(payload) => value_response(200, payload),
                Err(error) => error_response(500, "simulate_failed", &error),
            },
            Err(error) => error_response(400, "invalid_json", &error),
        },
        ("POST", "/api/model/compare") => match read_json_body(&request) {
            Ok(body) => match adapters.compare.compare(body) {
                Ok(payload) => value_response(200, payload),
                Err(error) => error_response(500, "compare_failed", &error),
            },
            Err(error) => error_response(400, "invalid_json", &error),
        },
        ("POST", "/api/litematic/import") => {
            let options = parse_query_options(request.path.as_str());
            match adapters.litematic.import_litematic(&request.body, &options) {
                Ok(payload) => value_response(200, payload),
                Err(error) => error_response(400, "litematic_import_failed", &error),
            }
        }
        ("POST", "/api/litematic/export") => match read_json_body(&request) {
            Ok(body) => match adapters.litematic.export_litematic(body) {
                Ok((bytes, filename, content_type)) => {
                    binary_response(200, bytes, &filename, content_type)
                }
                Err(error) => error_response(400, "litematic_export_failed", &error),
            },
            Err(error) => error_response(400, "invalid_json", &error),
        },
        ("DELETE", _) if segments.len() == 3 && segments[0] == "api" && segments[1] == "runs" => {
            match parse_u64_segment(segments[2]) {
                Ok(run_id) => match adapters.runs.soft_delete_run(run_id) {
                    Ok(payload) => value_response(200, payload),
                    Err(error) => error_response(404, "run_not_found", &error),
                },
                Err(error) => error_response(400, "invalid_run_id", &error),
            }
        }
        ("POST", _)
            if segments.len() == 4
                && segments[0] == "api"
                && segments[1] == "runs"
                && segments[3] == "restore" =>
        {
            match parse_u64_segment(segments[2]) {
                Ok(run_id) => match adapters.runs.restore_run(run_id) {
                    Ok(payload) => value_response(200, payload),
                    Err(error) => error_response(404, "run_not_found", &error),
                },
                Err(error) => error_response(400, "invalid_run_id", &error),
            }
        }
        ("POST", _)
            if segments.len() == 4
                && segments[0] == "api"
                && segments[1] == "runs"
                && segments[3] == "purge" =>
        {
            match parse_u64_segment(segments[2]) {
                Ok(run_id) => match adapters.runs.purge_run(run_id) {
                    Ok(payload) => value_response(200, payload),
                    Err(error) => error_response(404, "run_not_found", &error),
                },
                Err(error) => error_response(400, "invalid_run_id", &error),
            }
        }
        ("GET", _)
            if segments.len() == 4
                && segments[0] == "api"
                && segments[1] == "model"
                && segments[2] == "search" =>
        {
            match adapters.search.get_search_task(segments[3]) {
                Ok(payload) => json_response(200, &payload),
                Err(error) => error_response(404, "search_task_not_found", &error),
            }
        }
        ("POST", _)
            if segments.len() == 5
                && segments[0] == "api"
                && segments[1] == "model"
                && segments[2] == "search"
                && segments[4] == "cancel" =>
        {
            match adapters.search.cancel_search_task(segments[3]) {
                Ok(payload) => json_response(200, &payload),
                Err(error) => error_response(404, "search_task_not_found", &error),
            }
        }
        ("GET", _) if path_only.starts_with("/mc-assets/block/") => serve_block_asset(path_only),
        ("GET", _) => serve_static_path(path_only),
        _ => error_response(404, "not_found", "Route is not implemented in the Rust service"),
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

impl HttpRequest {
    fn read_from(stream: &mut TcpStream) -> Result<Self, String> {
        let mut reader = BufReader::new(stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .map_err(|error| format!("Failed to read request line: {error}"))?;
        if first_line.trim().is_empty() {
            return Err("Empty request".to_string());
        }
        let mut parts = first_line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| "Missing request method".to_string())?
            .to_string();
        let path = parts
            .next()
            .ok_or_else(|| "Missing request path".to_string())?
            .to_string();
        let mut content_length = 0usize;
        loop {
            let mut header_line = String::new();
            reader
                .read_line(&mut header_line)
                .map_err(|error| format!("Failed to read headers: {error}"))?;
            let trimmed = header_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':')
                && name.eq_ignore_ascii_case("Content-Length")
            {
                content_length = value.trim().parse::<usize>().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        reader
            .read_exact(&mut body)
            .map_err(|error| format!("Failed to read body: {error}"))?;
        Ok(Self { method, path, body })
    }
}

struct HttpResponse {
    status: u16,
    content_type: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn write_to(&self, stream: &mut TcpStream) -> Result<(), String> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "OK",
        };
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            reason,
            self.content_type,
            self.body.len()
        );
        for (name, value) in &self.headers {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(&self.body))
            .map_err(|error| format!("Failed to write response: {error}"))
    }
}

fn json_response(status: u16, payload: &impl Serialize) -> HttpResponse {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    HttpResponse {
        status,
        content_type: "application/json; charset=utf-8".to_string(),
        headers: vec![
            ("Cache-Control".to_string(), "no-store".to_string()),
            ("Access-Control-Allow-Origin".to_string(), "*".to_string()),
        ],
        body,
    }
}

fn value_response(status: u16, payload: Value) -> HttpResponse {
    let body = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    HttpResponse {
        status,
        content_type: "application/json; charset=utf-8".to_string(),
        headers: vec![
            ("Cache-Control".to_string(), "no-store".to_string()),
            ("Access-Control-Allow-Origin".to_string(), "*".to_string()),
        ],
        body,
    }
}

fn binary_response(
    status: u16,
    body: Vec<u8>,
    filename: &str,
    content_type: &str,
) -> HttpResponse {
    HttpResponse {
        status,
        content_type: content_type.to_string(),
        headers: vec![
            (
                "Content-Disposition".to_string(),
                format!("attachment; filename=\"{}\"", sanitize_filename(filename)),
            ),
            ("Access-Control-Allow-Origin".to_string(), "*".to_string()),
        ],
        body,
    }
}

fn bytes_response(status: u16, body: Vec<u8>, content_type: &str) -> HttpResponse {
    HttpResponse {
        status,
        content_type: content_type.to_string(),
        headers: vec![("Access-Control-Allow-Origin".to_string(), "*".to_string())],
        body,
    }
}

fn text_response(status: u16, body: &str, content_type: &str) -> HttpResponse {
    bytes_response(status, body.as_bytes().to_vec(), content_type)
}

fn error_response(status: u16, error: &str, detail: &str) -> HttpResponse {
    json_response(status, &json!({ "ok": false, "error": error, "detail": detail }))
}

fn read_json_body(request: &HttpRequest) -> Result<Value, String> {
    if request.body.is_empty() {
        Ok(Value::Object(Default::default()))
    } else {
        serde_json::from_slice(&request.body).map_err(|error| format!("Invalid JSON body: {error}"))
    }
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|segment| !segment.is_empty()).collect()
}

fn parse_u64_segment(segment: &str) -> Result<u64, String> {
    segment
        .parse::<u64>()
        .map_err(|error| format!("Invalid integer segment '{segment}': {error}"))
}

fn parse_query_options(path: &str) -> BTreeMap<String, Value> {
    let mut options = BTreeMap::new();
    let Some(query) = path.split_once('?').map(|(_, query)| query) else {
        return options;
    };
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        options.insert(key.to_string(), Value::String(percent_decode(raw_value)));
    }
    options
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &input[index + 1..index + 3];
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    output.push(value);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            value => {
                output.push(value);
                index += 1;
            }
        }
    }
    String::from_utf8(output).unwrap_or_else(|_| input.to_string())
}

fn sanitize_filename(input: &str) -> String {
    let filtered = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if filtered.is_empty() {
        "waterway.litematic".to_string()
    } else {
        filtered
    }
}

fn resolve_viewer_data_dir() -> Option<PathBuf> {
    env_path("MC_VIEWER_DATA_DIR")
        .or_else(|| env_path("WATERWAY_DATA_DIR").map(|path| path.join("viewer_data")))
        .or_else(|| env_path("WATERWAY_HOME").map(|path| path.join("data").join("viewer_data")))
}

fn resolve_static_dir() -> Option<PathBuf> {
    env_path("MC_VIEWER_STATIC_DIR")
        .or_else(|| env_path("WATERWAY_APP_DIR").map(|path| path.join("web-data-analysis").join("viewer")))
        .or_else(|| env_path("WATERWAY_HOME").map(|path| path.join("app").join("web-data-analysis").join("viewer")))
        .or_else(|| env::current_dir().ok().map(|path| path.join("viewer")))
        .filter(|path| path.exists() && path.is_dir())
}

fn resolve_block_asset_dir() -> Option<PathBuf> {
    env_path("WATERWAY_ASSET_DIR")
        .or_else(|| {
            env_path("WATERWAY_HOME")
                .map(|path| path.join("assets").join("minecraft").join("textures").join("block"))
        })
        .or_else(|| {
            env_path("WATERWAY_APP_DIR")
                .map(|path| path.join("assets").join("minecraft").join("textures").join("block"))
        })
        .filter(|path| path.exists() && path.is_dir())
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Clone)]
struct ManagedSearchTaskService {
    manager: SearchTaskManager,
    store: Option<RunStore>,
}

impl ManagedSearchTaskService {
    fn new(store: Option<RunStore>) -> Self {
        Self {
            manager: SearchTaskManager::new(),
            store,
        }
    }
}

impl SearchTaskService for ManagedSearchTaskService {
    fn start_search_task(&self, request: Value) -> Result<SearchTaskEnvelope, String> {
        self.manager
            .cleanup_finished_older_than(Duration::from_secs(60 * 60));
        let store = self.store.clone();
        Ok(self.manager.start(request, move |ctx, request| {
            run_search_request(store.clone(), ctx, request)
        }))
    }

    fn get_search_task(&self, task_id: &str) -> Result<SearchTaskEnvelope, String> {
        self.manager
            .get(task_id)
            .ok_or_else(|| format!("search task '{task_id}' not found"))
    }

    fn cancel_search_task(&self, task_id: &str) -> Result<SearchTaskEnvelope, String> {
        self.manager
            .cancel(task_id)
            .ok_or_else(|| format!("search task '{task_id}' not found"))
    }
}

#[derive(Clone)]
struct RustSimulationAdapter {
    store: Option<RunStore>,
}

impl RustSimulationAdapter {
    fn new(store: Option<RunStore>) -> Self {
        Self { store }
    }

    fn with_store<T>(
        &self,
        op_name: &str,
        f: impl FnOnce(&RunStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self.store.as_ref().ok_or_else(|| {
            format!(
                "{op_name} unavailable: viewer data directory is not configured (MC_VIEWER_DATA_DIR or WATERWAY_DATA_DIR)"
            )
        })?;
        f(store)
    }
}

impl SimulationAdapter for RustSimulationAdapter {
    fn simulate(&self, request: Value) -> Result<Value, String> {
        let (structure, options, ticks, label) = parse_simulation_request(&request)?;
        let launch_mode = options
            .get("launchMode")
            .and_then(Value::as_str)
            .unwrap_or("water")
            .to_string();
        let structure = preprocess_structure_for_launch(
            &structure,
            &launch_mode,
        )?;
        let mut run = viewer_runs::simulate_run(&structure, ticks, &options, &label)?;
        run.summary.launch_mode = Some(launch_mode.clone());
        run.summary
            .extra
            .insert("requested_launch_mode".to_string(), json!(launch_mode));
        rewrite_model_engine(&mut run);
        let saved = self.with_store("simulate", |store| {
            store
                .append_run(run)
                .map_err(|error| format!("Failed to append simulation run: {error}"))
        })?;
        let payload = json!({
            "ok": true,
            "run": saved,
        });
        Ok(payload)
    }
}

struct RustCompareAdapter;

impl CompareAdapter for RustCompareAdapter {
    fn compare(&self, request: Value) -> Result<Value, String> {
        let (_structure, _options, ticks, _label) = parse_simulation_request(&request)?;
        Ok(json!({
            "ok": false,
            "error": "legacy_compare_unavailable",
            "detail": "Rust-only backend no longer bundles the historical Python/Node legacy wrapper. /api/model/compare cannot produce a true legacy reference in this build.",
            "ticks": ticks,
            "legacy_engine": Value::Null,
            "rust_engine": BACKEND_NAME,
        }))
    }
}

struct RustLitematicAdapter;

impl LitematicAdapter for RustLitematicAdapter {
    fn import_litematic(
        &self,
        bytes: &[u8],
        options: &BTreeMap<String, Value>,
    ) -> Result<Value, String> {
        let options: LitematicImportOptions = serde_json::from_value(serde_json::to_value(options).map_err(
            |error| format!("Failed to encode litematic import options: {error}"),
        )?)
        .map_err(|error| format!("Invalid litematic import options: {error}"))?;
        let payload = litematic::import_litematic(bytes, options)?;
        serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize litematic import payload: {error}"))
    }

    fn export_litematic(
        &self,
        request: Value,
    ) -> Result<(Vec<u8>, String, &'static str), String> {
        let request: LitematicExportRequest = serde_json::from_value(request)
            .map_err(|error| format!("Invalid litematic export request: {error}"))?;
        let structure = request
            .structure
            .ok_or_else(|| "Missing structure for litematic export.".to_string())?;
        let filename_stem = request
            .options
            .as_ref()
            .and_then(|options| options.name.clone())
            .or_else(|| structure.name.clone())
            .unwrap_or_else(|| "waterway".to_string());
        let bytes = litematic::export_litematic(&structure, request.options)?;
        let filename = format!("{}.litematic", sanitize_filename(&filename_stem));
        Ok((bytes, filename, "application/octet-stream"))
    }
}

#[derive(Clone)]
struct RunStoreAdapter {
    store: Option<RunStore>,
}

impl RunStoreAdapter {
    fn from_env() -> Self {
        Self {
            store: resolve_viewer_data_dir().map(RunStore::new),
        }
    }

    fn with_store<T>(
        &self,
        op_name: &str,
        f: impl FnOnce(&RunStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self.store.as_ref().ok_or_else(|| {
            format!(
                "{op_name} unavailable: viewer data directory is not configured (MC_VIEWER_DATA_DIR or WATERWAY_DATA_DIR)"
            )
        })?;
        f(store)
    }
}

impl RunsAdapter for RunStoreAdapter {
    fn load_runs(&self) -> Result<Value, String> {
        let payload = self.with_store("load runs", |store| {
            store
                .load_runs()
                .map_err(|error| format!("Failed to load runs from store: {error}"))
        })?;
        serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize runs payload: {error}"))
    }

    fn soft_delete_run(&self, run_id: u64) -> Result<Value, String> {
        let payload = self.with_store("delete run", |store| {
            store
                .soft_delete_run(run_id)
                .map_err(|error| error.to_string())
        })?;
        serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize delete payload: {error}"))
    }

    fn restore_run(&self, run_id: u64) -> Result<Value, String> {
        let payload = self.with_store("restore run", |store| {
            store.restore_run(run_id).map_err(|error| error.to_string())
        })?;
        serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize restore payload: {error}"))
    }

    fn purge_run(&self, run_id: u64) -> Result<Value, String> {
        let payload = self.with_store("purge run", |store| {
            store.purge_run(run_id).map_err(|error| error.to_string())
        })?;
        serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize purge payload: {error}"))
    }
}

fn parse_simulation_request(
    request: &Value,
) -> Result<(schema::Structure, ViewerRunOptions, usize, String), String> {
    let structure = request
        .get("structure")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("Invalid structure payload: {error}"))?
        .unwrap_or_else(schema::default_structure);
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .map(json_object_to_options)
        .unwrap_or_default();
    let ticks = options
        .get("ticks")
        .and_then(value_as_usize)
        .or_else(|| request.get("ticks").and_then(value_as_usize))
        .unwrap_or(400)
        .clamp(1, 200_000);
    let label = options
        .get("label")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("结构模拟")
        .to_string();
    Ok((structure, options, ticks, label))
}

fn run_search_request(
    store: Option<RunStore>,
    ctx: SearchTaskContext,
    request: Value,
) -> Result<Value, SearchTaskError> {
    let params = request
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| SearchTaskError::failed("Missing search params object"))?;
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .map(json_object_to_options)
        .unwrap_or_default();
    let keep = params
        .get("keep")
        .and_then(value_as_usize)
        .unwrap_or(8)
        .clamp(1, 200);
    let target_speed = params
        .get("targetSpeed")
        .or_else(|| options.get("targetSpeed"))
        .and_then(value_as_f64)
        .unwrap_or(0.5);
    let target_dwell_ticks = options
        .get("targetDwellTicks")
        .or_else(|| params.get("targetDwellTicks"))
        .and_then(value_as_usize)
        .unwrap_or_else(|| ((1.0 / target_speed).round() as usize).max(1));
    let min_hit_rate = params
        .get("minHitRate")
        .and_then(value_as_f64)
        .unwrap_or(1.0);
    let max_threads = params
        .get("maxThreads")
        .and_then(value_as_usize)
        .unwrap_or(8)
        .clamp(1, 256);

    let execution = preprocess_search_request(params)?;
    let cycles = search_cycle_names(params)?;
    let modes = search_modes_for_request(params, &execution);
    let keep = keep.clamp(1, 50);
    let top_candidates = search_top_candidates(params, keep, cycles.len(), modes.len());
    let artifact_root = resolve_search_artifact_root(store.as_ref());

    ctx.update_progress(SearchTaskProgress {
        stage: Some("searching".to_string()),
        message: Some("running Rust reachable candidate generator".to_string()),
        checked: Some(0),
        total: Some(top_candidates as u64),
        candidate_count: Some(0),
        unique_count: Some(0),
        written: Some(0),
        parallel_workers: Some(max_threads as u64),
        ..SearchTaskProgress::default()
    });
    ctx.check_cancelled()?;

    let mut generator_progress = |update: crate::reachable_candidates::ServiceProgressUpdate| {
        ctx.update_progress(SearchTaskProgress {
            stage: Some(update.stage),
            message: Some(update.message),
            checked: update.checked,
            total: update.total,
            percent: update.percent,
            candidate_count: update.candidate_count,
            unique_count: update.unique_count,
            expanded_states: update.expanded_states,
            bucket_count: update.bucket_count,
            parallel_workers: Some(max_threads as u64),
            ..SearchTaskProgress::default()
        });
    };

    let payload = crate::reachable_candidates::run_service(
        &crate::reachable_candidates::ServiceSearchRequest {
            out_root: artifact_root,
            run_name: format!("task-{}", ctx.task_id()),
            modes,
            cycles,
            max_prefix_cells: params
                .get("maxPrefixCells")
                .and_then(value_as_usize)
                .unwrap_or(9)
                .clamp(0, 64),
            workers: max_threads,
            short_ticks: execution.ticks,
            target_speed,
            target_dwell_ticks,
            top_candidates,
            long_limit: 0,
            min_short_hit_rate_for_long: min_hit_rate,
            entity_mods: vec![0, 1, 2, 3],
            initial_tick_mods: vec![0, 1, 2, 3],
            start_x: execution.start_x,
            start_y: execution.start_y,
            start_vx: execution.start_vx,
            start_vy: execution.start_vy,
            start_on_ground: execution.start_on_ground,
            debug_generator: false,
        },
        Some(ctx.cancel_flag()),
        Some(&mut generator_progress),
    )
    .map_err(|error| {
        if crate::reachable_candidates::is_cancelled_error(&error) {
            SearchTaskError::cancelled()
        } else {
            SearchTaskError::failed(error)
        }
    })?;
    ctx.check_cancelled()?;

    let passing_count = payload
        .short_verified
        .iter()
        .filter(|row| {
            row.target_hit_rate
                .unwrap_or(row.strict_hit_rate.unwrap_or(0.0))
                >= min_hit_rate
        })
        .count();

    ctx.update_progress(SearchTaskProgress {
        stage: Some("verifying".to_string()),
        candidate_count: Some(payload.short_verified.len() as u64),
        passing_count: Some(passing_count as u64),
        message: Some("candidate verification complete; writing runs".to_string()),
        ..SearchTaskProgress::default()
    });

    let created = write_search_runs(
        store,
        &payload,
        &execution,
        &options,
        keep,
        min_hit_rate,
        target_speed,
        target_dwell_ticks,
        &ctx,
    )?;

    let message = if !created.is_empty() {
        format!("已写入 {} 个搜索结果 run", created.len())
    } else if passing_count == 0 {
        format!("没有候选达到命中率要求 ({min_hit_rate:.4})")
    } else {
        format!("验证后没有候选写入 runs ({min_hit_rate:.4})")
    };

    Ok(json!({
        "ok": true,
        "engine": "rust-reachable-candidates",
        "generator_engine": "rust",
        "verification_status": "short-long-solver-verified",
        "legal_waterfields": Value::Null,
        "artifact_dir": payload.out_dir.display().to_string(),
        "candidate_count": payload.short_verified.len(),
        "top_candidates": top_candidates,
        "passing_count": passing_count,
        "filtered_out_count": payload.short_verified.len().saturating_sub(passing_count),
        "unique_count": created.len(),
        "parallel_workers": max_threads,
        "short_passing_for_long": payload.short_passing_for_long,
        "long_verified": payload.long_verified,
        "generator_summary": payload.generator_payload,
        "message": message,
        "created": created,
    }))
}

fn write_search_runs(
    store: Option<RunStore>,
    payload: &crate::reachable_candidates::ServiceSearchResult,
    execution: &SearchExecution,
    base_options: &ViewerRunOptions,
    keep: usize,
    min_hit_rate: f64,
    target_speed: f64,
    target_dwell_ticks: usize,
    ctx: &SearchTaskContext,
) -> Result<Vec<Value>, SearchTaskError> {
    let store = store.ok_or_else(|| {
        SearchTaskError::failed(
            "search unavailable: viewer data directory is not configured (MC_VIEWER_DATA_DIR or WATERWAY_DATA_DIR)",
        )
    })?;
    let mut created = Vec::new();
    let selected_rows = payload
        .short_verified
        .iter()
        .filter(|row| {
            row.target_hit_rate
                .unwrap_or(row.strict_hit_rate.unwrap_or(0.0))
                >= min_hit_rate
        })
        .take(keep)
        .collect::<Vec<_>>();
    let write_total = selected_rows.len();
    for (index, row) in selected_rows.iter().enumerate() {
        ctx.check_cancelled()?;
        let structure = build_search_structure(row, execution)?;
        let label = format!(
            "状态搜索结果 {}: {} / {}",
            index + 1,
            row.prefix_label,
            row.cycle
        );
        let mut options = base_options.clone();
        options.insert("targetSpeed".to_string(), json!(target_speed));
        options.insert("targetDwellTicks".to_string(), json!(target_dwell_ticks));
        apply_search_dwell_window_options(&mut options, row);
        let mut run = viewer_runs::simulate_run(&structure, execution.ticks, &options, &label)
            .map_err(SearchTaskError::failed)?;
        rewrite_model_engine(&mut run);
        run.summary.source = Some("reachability-search".to_string());
        run.summary.structure = Some(format!("{} / {}", row.prefix_label, row.cycle));
        run.summary.structure_count = Some(1);
        run.summary.equivalent_fingerprint = Some(row.id.clone());
        run.summary.extra.insert(
            "structures".to_string(),
            json!([{
                "structure": format!("{} / {}", row.prefix_label, row.cycle),
                "mode": row.mode,
                "prefixLabel": row.prefix_label,
                "cycle": row.cycle,
                "score": row.score,
                "average_speed": row.average_speed,
                "target_hit_rate": row.target_hit_rate.unwrap_or(row.strict_hit_rate.unwrap_or(0.0)),
                "two_gt_hit_rate": row.strict_hit_rate.unwrap_or(0.0),
                "dwell_window": row.dwell_window,
            }]),
        );
        run.summary
            .extra
            .insert("reachability_score".to_string(), json!(row.score));
        run.summary
            .extra
            .insert("reachability_mode".to_string(), json!(row.mode));
        run.summary.extra.insert(
            "reachability_hit_rate".to_string(),
            json!(row.target_hit_rate.unwrap_or(row.strict_hit_rate.unwrap_or(0.0))),
        );
        run.summary.extra.insert(
            "search_dwell_window".to_string(),
            row.dwell_window.clone(),
        );
        run.summary.extra.insert(
            "search_raw_short_hit_rate".to_string(),
            json!(row.raw_short_hit_rate.unwrap_or(row.strict_hit_rate.unwrap_or(0.0))),
        );
        run.summary.extra.insert(
            "search_short_hit_rate".to_string(),
            json!(row.target_hit_rate.unwrap_or(row.strict_hit_rate.unwrap_or(0.0))),
        );
        run.summary.extra.insert(
            "artifact_dir".to_string(),
            json!(payload.out_dir.display().to_string()),
        );
        let saved = store
            .append_run(run)
            .map_err(|error| SearchTaskError::failed(format!("Failed to append search run: {error}")))?;
        created.push(json!({
            "run_id": saved.run_id,
            "label": saved.display_label,
            "score": row.score,
            "equivalent_count": 1,
        }));
        ctx.update_progress(SearchTaskProgress {
            stage: Some("writing".to_string()),
            written: Some((index + 1) as u64),
            write_total: Some(write_total as u64),
            unique_count: Some(created.len() as u64),
            candidate_count: Some(payload.short_verified.len() as u64),
            passing_count: Some(created.len() as u64),
            message: Some(format!("writing search result {}/{}", index + 1, write_total)),
            ..SearchTaskProgress::default()
        });
    }
    Ok(created)
}

fn apply_search_dwell_window_options(
    options: &mut ViewerRunOptions,
    row: &crate::reachable_candidates::ServiceCandidateRow,
) {
    let Some(window) = row.dwell_window.as_object() else {
        return;
    };
    if let Some(value) = window.get("minBlock").and_then(Value::as_i64) {
        options.insert("minBlock".to_string(), json!(value));
    }
    if let Some(value) = window.get("maxBlock").and_then(Value::as_i64) {
        options.insert("maxBlock".to_string(), json!(value));
    }
    if let Some(value) = window.get("includeFinalGroup").and_then(Value::as_bool) {
        options.insert("includeFinalGroup".to_string(), json!(value));
    }
    let mode = window.get("mode").and_then(Value::as_str).unwrap_or_default();
    if let Some(value) = window.get("minStartTick").and_then(value_as_usize) {
        if should_apply_explicit_min_start_tick(mode, value) {
            options.insert("minStartTick".to_string(), json!(value));
        }
    }
}

fn should_apply_explicit_min_start_tick(mode: &str, min_start_tick: usize) -> bool {
    min_start_tick > 0 || !mode.eq_ignore_ascii_case("cycle")
}

fn build_search_structure(
    row: &crate::reachable_candidates::ServiceCandidateRow,
    execution: &SearchExecution,
) -> Result<schema::Structure, SearchTaskError> {
    let prefix = row
        .prefix_cells
        .iter()
        .map(cell_description_to_schema_cell)
        .collect::<Result<Vec<_>, _>>()
        .map_err(SearchTaskError::failed)?;
    let cycle = row
        .cycle_cells
        .iter()
        .map(cell_description_to_schema_cell)
        .collect::<Result<Vec<_>, _>>()
        .map_err(SearchTaskError::failed)?;
    let mut start = execution.raw_start.clone();
    start.entity_id_mod4 = row.entity_id_mod4;
    start.initial_tick_count = if execution.launch_mode == "piston" {
        let desired_raw_mod4 =
            (row.initial_tick_mod4 + 4 - (execution.launch_timeline_offset % 4)) % 4;
        let base_mod4 = execution.raw_start.initial_tick_count % 4;
        execution.raw_start.initial_tick_count + ((desired_raw_mod4 + 4 - base_mod4) % 4)
    } else {
        row.initial_tick_mod4
    };

    let structure = schema::Structure {
        name: Some(format!("{} / {}", row.prefix_label, row.cycle)),
        origin_x: execution.origin_x,
        start,
        launch_config: execution.launch_config.clone(),
        prefix,
        cycle,
        extra: BTreeMap::new(),
    };
    preprocess_structure_for_launch(&structure, &execution.launch_mode)
        .map_err(SearchTaskError::failed)
}

fn cell_description_to_schema_cell(
    cell: &crate::CellDescription,
) -> Result<schema::Cell, String> {
    Ok(schema::make_cell(
        cell.surface,
        cell.flow,
        cell.floor.clone(),
        Some(cell.code.clone()),
        Some(cell.amount),
    ))
}

fn search_cycle_names(
    params: &serde_json::Map<String, Value>,
) -> Result<Vec<String>, SearchTaskError> {
    let max_cycle_cells = params
        .get("maxCycleCells")
        .and_then(value_as_usize)
        .unwrap_or(0);
    let cycles = crate::backbone_cycles()
        .into_iter()
        .filter(|cycle| max_cycle_cells == 0 || cycle.period() <= max_cycle_cells)
        .map(|cycle| cycle.name)
        .collect::<Vec<_>>();
    if cycles.is_empty() {
        return Err(SearchTaskError::failed(format!(
            "No reachable backbone cycles match maxCycleCells={max_cycle_cells}."
        )));
    }
    Ok(cycles)
}

fn search_top_candidates(
    params: &serde_json::Map<String, Value>,
    keep: usize,
    cycle_count: usize,
    mode_count: usize,
) -> usize {
    if let Some(explicit) = params
        .get("topCandidates")
        .or_else(|| params.get("candidateCount"))
        .and_then(value_as_usize)
    {
        return explicit.clamp(1, 2_000);
    }

    let max_prefix_cells = params
        .get("maxPrefixCells")
        .and_then(value_as_usize)
        .unwrap_or(9);
    let base = keep.saturating_mul(20).max(80);
    let prefix_factor = usize::from(max_prefix_cells >= 12) + 1;
    let cycle_factor: usize = if cycle_count >= 24 {
        2
    } else if cycle_count >= 8 {
        1
    } else {
        0
    };
    base.saturating_mul(prefix_factor)
        .saturating_add(cycle_factor.saturating_mul(40))
        .saturating_mul(mode_count.max(1))
        .clamp(80, 400)
}

fn search_modes_for_request(
    params: &serde_json::Map<String, Value>,
    execution: &SearchExecution,
) -> Vec<String> {
    let launch_mode = params
        .get("launchMode")
        .and_then(Value::as_str)
        .unwrap_or("water")
        .to_ascii_lowercase();
    let legacy_mode = params
        .get("mode")
        .or_else(|| params.get("searchMode"))
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());

    if launch_mode == "piston" {
        return vec!["launch-fast".to_string()];
    }
    match legacy_mode.as_deref() {
        Some("early") => vec!["launch-fast".to_string(), "hybrid".to_string()],
        Some("full") => vec!["water-accelerate".to_string(), "hybrid".to_string()],
        _ if execution.start_vx >= 0.75 => vec!["launch-fast".to_string(), "hybrid".to_string()],
        _ if execution.start_vx >= 0.2 => vec!["hybrid".to_string(), "water-accelerate".to_string()],
        _ => vec!["water-accelerate".to_string(), "hybrid".to_string()],
    }
}

fn resolve_search_artifact_root(store: Option<&RunStore>) -> PathBuf {
    if let Some(store) = store {
        let viewer_data_dir = store.viewer_data_dir();
        return viewer_data_dir
            .parent()
            .unwrap_or(&viewer_data_dir)
            .join("reachability-candidate-generator");
    }
    PathBuf::from("artifacts").join("reachability-candidate-generator")
}

#[derive(Clone)]
struct SearchExecution {
    start_x: f64,
    start_y: f64,
    start_vx: f64,
    start_vy: f64,
    start_on_ground: bool,
    ticks: usize,
    origin_x: f64,
    raw_start: schema::StartState,
    launch_mode: String,
    launch_timeline_offset: usize,
    launch_config: Option<schema::LaunchConfig>,
    launch: Option<Value>,
}

fn search_launch_placeholder_cycle() -> Vec<schema::Cell> {
    vec![schema::make_cell(None, 0, "normal", None, Some(0))]
}

fn preprocess_search_request(
    params: &serde_json::Map<String, Value>,
) -> Result<SearchExecution, SearchTaskError> {
    let ticks = params
        .get("ticks")
        .and_then(value_as_usize)
        .unwrap_or(800)
        .clamp(1, 200_000);
    let start_x = params.get("startX").and_then(value_as_f64).unwrap_or(0.125);
    let start_y = params.get("startY").and_then(value_as_f64).unwrap_or(0.0);
    let start_vx = params.get("startVX").and_then(value_as_f64).unwrap_or(0.0);
    let start_vy = params.get("startVY").and_then(value_as_f64).unwrap_or(0.0);
    let start_on_ground = params
        .get("startOnGround")
        .and_then(value_as_bool)
        .unwrap_or(true);
    let launch_mode = params
        .get("launchMode")
        .and_then(Value::as_str)
        .unwrap_or("water");
    let origin_x = params.get("originX").and_then(value_as_f64).unwrap_or(0.0);
    let raw_slime_block_x = params
        .get("slimeBlockX")
        .and_then(value_as_f64)
        .unwrap_or(-1.0);
    let launch_config = if launch_mode == "piston" {
        Some(schema::LaunchConfig {
            mode: Some("piston".to_string()),
            slime_block_x: Some(raw_slime_block_x),
            extra: BTreeMap::new(),
        })
    } else {
        None
    };

    let raw_start = schema::StartState {
        x: start_x,
        y: start_y,
        vx: start_vx,
        vy: start_vy,
        start_on_ground: Some(start_on_ground),
        entity_id_mod4: params
            .get("entityIdMod4")
            .and_then(value_as_usize)
            .unwrap_or(0),
        initial_tick_count: params
            .get("initialTickCount")
            .and_then(value_as_usize)
            .unwrap_or(0),
        extra: BTreeMap::new(),
    };

    let mut structure = schema::Structure {
        name: Some("search-start".to_string()),
        origin_x,
        start: raw_start.clone(),
        launch_config,
        prefix: Vec::new(),
        cycle: if launch_mode == "piston" {
            search_launch_placeholder_cycle()
        } else {
            schema::default_cycle_cells()
        },
        extra: BTreeMap::new(),
    };
    structure = preprocess_structure_for_launch(&structure, launch_mode)
        .map_err(SearchTaskError::failed)?;
    Ok(SearchExecution {
        start_x: structure.start.x,
        start_y: structure.start.y,
        start_vx: structure.start.vx,
        start_vy: structure.start.vy,
        start_on_ground: structure.start.start_on_ground.unwrap_or(true),
        ticks,
        origin_x: structure.origin_x,
        raw_start,
        launch_mode: launch_mode.to_string(),
        launch_timeline_offset: structure
            .extra
            .get("launch")
            .and_then(Value::as_object)
            .and_then(|launch| launch.get("timelineOffsetGt"))
            .and_then(value_as_usize)
            .unwrap_or(0),
        launch_config: structure.launch_config.clone(),
        launch: structure.extra.get("launch").cloned(),
    })
}

pub(crate) fn preprocess_structure_for_launch(
    structure: &schema::Structure,
    launch_mode: &str,
) -> Result<schema::Structure, String> {
    if launch_mode != "piston" {
        return Ok(structure.clone());
    }

    let raw_start = structure
        .extra
        .get("launch")
        .and_then(Value::as_object)
        .and_then(|launch| launch.get("rawStart"))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("Invalid launch.rawStart in piston payload: {error}"))?
        .unwrap_or_else(|| structure.start.clone());

    let mut raw_structure = structure.clone();
    raw_structure.start = raw_start;

    let layout = structure_to_layout(&raw_structure)?;
    let launch = slime_piston_launch_entry(&raw_structure, &layout)?;
    let effective_start = launch
        .get("effectiveLocalStart")
        .cloned()
        .or_else(|| launch.get("effectiveStart").cloned())
        .ok_or_else(|| "piston launch preprocessing missing effectiveStart".to_string())?;
    let mut updated = raw_structure;
    updated.start = serde_json::from_value(effective_start)
        .map_err(|error| format!("Invalid effectiveStart produced by launch preprocessing: {error}"))?;
    updated.extra.insert("launch".to_string(), launch);
    Ok(updated)
}

fn structure_to_layout(structure: &schema::Structure) -> Result<crate::Layout, String> {
    let prefix = structure
        .prefix
        .iter()
        .map(schema_cell_to_layout_cell)
        .collect::<Result<Vec<_>, _>>()?;
    let cycle = structure
        .cycle
        .iter()
        .map(schema_cell_to_layout_cell)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(crate::Layout::new(&prefix, &cycle))
}

fn schema_cell_to_layout_cell(cell: &schema::Cell) -> Result<crate::Cell, String> {
    Ok(crate::Cell {
        surface: cell.canonical_surface(),
        flow: cell.canonical_flow(),
        amount: cell.canonical_amount().min(8),
        floor: crate::parse_floor(cell.floor.as_str())?,
    })
}

fn slime_piston_launch_entry(
    structure: &schema::Structure,
    layout: &crate::Layout,
) -> Result<Value, String> {
    let half_width = 0.25 / 2.0;
    let step = 0.5;
    let extra = 0.01;
    let slime_block_x = structure
        .launch_config
        .as_ref()
        .and_then(|config| config.slime_block_x)
        .unwrap_or(-1.0);
    let waterway_start_x = slime_block_x + 2.0;
    let display_origin_x = structure.origin_x + waterway_start_x;
    let face_start_x = slime_block_x + 1.0;
    let raw = json!({
        "x": structure.start.x,
        "y": structure.start.y,
        "vx": structure.start.vx,
        "vy": structure.start.vy,
        "startOnGround": structure.start.start_on_ground.unwrap_or(true),
        "entityIdMod4": structure.start.entity_id_mod4,
        "initialTickCount": structure.start.initial_tick_count,
    });
    let mut state = LaunchState {
        x: structure.start.x,
        y: structure.start.y,
        vx: structure.start.vx,
        vy: structure.start.vy,
        on_ground: structure.start.start_on_ground.unwrap_or(true),
        entity_id_mod4: structure.start.entity_id_mod4,
        initial_tick_count: structure.start.initial_tick_count,
    };
    let mut timeline_samples = vec![json!({
        "gt": 0,
        "x": state.x,
        "y": state.y,
        "vx": state.vx,
        "vy": state.vy,
        "onGround": state.on_ground,
        "pistonCollision": false,
    })];
    let mut piston_steps = Vec::new();
    let piston_ticks = 2usize;

    for step_index in 0..piston_ticks {
        let collision_gt = step_index + 1;
        state = launch_item_tick(layout, &state, waterway_start_x, collision_gt);
        let sweep_min = face_start_x + step_index as f64 * step;
        let sweep_max = sweep_min + step;
        let item_min = state.x - half_width;
        let item_max = state.x + half_width;
        let mut piston_step = json!({
            "gt": collision_gt,
            "sweepMinX": sweep_min,
            "sweepMaxX": sweep_max,
            "itemMinX": item_min,
            "itemMaxX": item_max,
            "movement": 0.0,
            "xBefore": state.x,
            "xAfter": state.x,
            "collided": false,
        });
        if !(sweep_max >= item_min && item_max >= sweep_min) {
            piston_steps.push(piston_step);
            timeline_samples.push(json!({
                "gt": collision_gt,
                "x": state.x,
                "y": state.y,
                "vx": state.vx,
                "vy": state.vy,
                "onGround": state.on_ground,
                "pistonCollision": false,
            }));
            continue;
        }
        let movement = (sweep_max - item_min).clamp(0.0, step) + extra;
        let effective_absolute_x = state.x + movement;
        state.x = effective_absolute_x;
        state.vx = 1.0;
        state.on_ground = false;
        piston_step["movement"] = json!(movement);
        piston_step["xAfter"] = json!(effective_absolute_x);
        piston_step["collided"] = json!(true);
        piston_steps.push(piston_step);
        timeline_samples.push(json!({
            "gt": collision_gt,
            "x": state.x,
            "y": state.y,
            "vx": state.vx,
            "vy": state.vy,
            "onGround": state.on_ground,
            "pistonCollision": true,
        }));
        let effective_start = json!({
            "x": state.x,
            "y": state.y,
            "vx": state.vx,
            "vy": state.vy,
            "startOnGround": state.on_ground,
            "entityIdMod4": state.entity_id_mod4,
            "initialTickCount": structure.start.initial_tick_count + collision_gt,
        });
        let effective_local_start = json!({
            "x": state.x - waterway_start_x,
            "y": state.y,
            "vx": state.vx,
            "vy": state.vy,
            "startOnGround": state.on_ground,
            "entityIdMod4": state.entity_id_mod4,
            "initialTickCount": structure.start.initial_tick_count + collision_gt,
        });
        let local_timeline_samples = timeline_samples
            .iter()
            .map(|sample| {
                let mut localized = sample.clone();
                if let Some(value) = localized.get("x").and_then(Value::as_f64) {
                    localized["x"] = json!(value - waterway_start_x);
                }
                localized
            })
            .collect::<Vec<_>>();
        return Ok(json!({
            "mode": "piston",
            "applied": true,
            "rawStart": raw,
            "effectiveStart": effective_start,
            "effectiveLocalStart": effective_local_start,
            "effectiveAbsoluteX": effective_absolute_x,
            "displayOriginX": display_origin_x,
            "waterwayStartX": waterway_start_x,
            "slimeBlockX": slime_block_x,
            "direction": "+X",
            "pistonTicks": piston_ticks,
            "collisionGt": collision_gt,
            "lastCollisionGt": collision_gt,
            "collisionCount": 1,
            "sweepMinX": sweep_min,
            "sweepMaxX": sweep_max,
            "pistonMovement": movement,
            "pistonMovementTotal": movement,
            "timelineOffsetGt": collision_gt,
            "timelineSamples": local_timeline_samples,
            "timelineAbsoluteSamples": timeline_samples,
            "pistonSteps": piston_steps,
        }));
    }

    let effective_start = json!({
        "x": state.x,
        "y": state.y,
        "vx": state.vx,
        "vy": state.vy,
        "startOnGround": state.on_ground,
        "entityIdMod4": state.entity_id_mod4,
        "initialTickCount": structure.start.initial_tick_count + piston_ticks,
    });
    let effective_local_start = json!({
        "x": state.x - waterway_start_x,
        "y": state.y,
        "vx": state.vx,
        "vy": state.vy,
        "startOnGround": state.on_ground,
        "entityIdMod4": state.entity_id_mod4,
        "initialTickCount": structure.start.initial_tick_count + piston_ticks,
    });
    let local_timeline_samples = timeline_samples
        .iter()
        .map(|sample| {
            let mut localized = sample.clone();
            if let Some(value) = localized.get("x").and_then(Value::as_f64) {
                localized["x"] = json!(value - waterway_start_x);
            }
            localized
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "mode": "piston",
        "applied": false,
        "rawStart": raw,
        "effectiveStart": effective_start,
        "effectiveLocalStart": effective_local_start,
        "effectiveAbsoluteX": state.x,
        "displayOriginX": display_origin_x,
        "waterwayStartX": waterway_start_x,
        "slimeBlockX": slime_block_x,
        "direction": "+X",
        "pistonTicks": piston_ticks,
        "collisionGt": Value::Null,
        "lastCollisionGt": Value::Null,
        "collisionCount": 0,
        "pistonMovement": 0.0,
        "pistonMovementTotal": 0.0,
        "timelineOffsetGt": piston_ticks,
        "timelineSamples": local_timeline_samples,
        "timelineAbsoluteSamples": timeline_samples,
        "pistonSteps": piston_steps,
        "reason": "no_collision",
    }))
}

#[derive(Clone, Copy)]
struct LaunchState {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    on_ground: bool,
    entity_id_mod4: usize,
    initial_tick_count: usize,
}

fn launch_item_tick(
    layout: &crate::Layout,
    state: &LaunchState,
    waterway_start_x: f64,
    tick_offset: usize,
) -> LaunchState {
    let sim = crate::simulate(
        layout,
        &crate::SimConfig {
            ticks: 1,
            start_x: state.x - waterway_start_x,
            start_y: state.y,
            start_vx: state.vx,
            start_vy: state.vy,
            entity_id_mod4: state.entity_id_mod4,
            initial_tick_count: state.initial_tick_count + tick_offset.saturating_sub(1),
            start_on_ground: Some(state.on_ground),
        },
    );
    LaunchState {
        x: sim.xs[1] + waterway_start_x,
        y: sim.ys[1],
        vx: sim.vxs[1],
        vy: sim.vys[1],
        on_ground: sim.on_grounds[1] != 0,
        entity_id_mod4: state.entity_id_mod4,
        initial_tick_count: state.initial_tick_count,
    }
}

fn rewrite_model_engine(run: &mut schema::ViewerRun) {
    run.summary.model_engine = Some(BACKEND_NAME.to_string());
}

fn json_object_to_options(object: &serde_json::Map<String, Value>) -> ViewerRunOptions {
    object.iter().map(|(key, value)| (key.clone(), value.clone())).collect()
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_as_usize(value: &Value) -> Option<usize> {
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

fn value_as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn serve_block_asset(path_only: &str) -> HttpResponse {
    let Some(root) = resolve_block_asset_dir() else {
        return error_response(404, "not_found", "block asset directory is not configured");
    };
    let Some(name) = path_only.rsplit('/').next() else {
        return error_response(404, "not_found", "invalid block asset path");
    };
    if !name.ends_with(".png") || name.contains('\\') || name.contains("..") {
        return error_response(404, "not_found", "invalid block asset path");
    }
    let target = root.join(name);
    read_static_file(&root, &target)
        .map(|bytes| bytes_response(200, bytes, "image/png"))
        .unwrap_or_else(|error| error_response(404, "not_found", &error))
}

fn serve_static_path(path_only: &str) -> HttpResponse {
    let Some(root) = resolve_static_dir() else {
        return text_response(404, "static viewer directory is not configured", "text/plain; charset=utf-8");
    };
    let request_path = if path_only == "/" || path_only.is_empty() {
        "index.html"
    } else {
        path_only.trim_start_matches('/')
    };
    let target = root.join(request_path);
    let resolved = if target.is_dir() {
        target.join("index.html")
    } else {
        target
    };
    match read_static_file(&root, &resolved) {
        Ok(bytes) => bytes_response(200, bytes, content_type_for_path(&resolved)),
        Err(_) if !request_path.contains('.') => {
            let index = root.join("index.html");
            match read_static_file(&root, &index) {
                Ok(bytes) => bytes_response(200, bytes, "text/html; charset=utf-8"),
                Err(error) => error_response(404, "not_found", &error),
            }
        }
        Err(error) => error_response(404, "not_found", &error),
    }
}

fn read_static_file(root: &Path, target: &Path) -> Result<Vec<u8>, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve static root: {error}"))?;
    let resolved = target
        .canonicalize()
        .map_err(|error| format!("failed to resolve static path: {error}"))?;
    if !resolved.starts_with(&root) {
        return Err("requested path escapes the static root".to_string());
    }
    fs::read(&resolved).map_err(|error| format!("failed to read {}: {error}", resolved.display()))
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LitematicExportRequest {
    #[serde(default)]
    structure: Option<schema::Structure>,
    #[serde(default)]
    options: Option<LitematicExportOptions>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_store::RunStore;
    use crate::schema::{ViewerRun, ViewerRunSummary, ViewerRunsPayload};
    use std::collections::BTreeMap;
    use std::fs;
    use std::thread;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "item-waterway-solver-service-{}-{}",
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

        fn viewer_data_dir(&self) -> PathBuf {
            self.path.join("viewer_data")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn real_adapters(temp: &TestDir) -> Arc<ServiceAdapters> {
        Arc::new(ServiceAdapters::with_viewer_data_dir(temp.viewer_data_dir()))
    }

    fn assert_close(actual: f64, expected: f64, epsilon: f64, label: &str) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{label} mismatch: expected {expected}, got {actual}, epsilon {epsilon}"
        );
    }

    fn search_route(adapters: Arc<ServiceAdapters>, method: &str, path: &str, body: Vec<u8>) -> Value {
        let response = route_request(
            HttpRequest {
                method: method.to_string(),
                path: path.to_string(),
                body,
            },
            adapters,
        );
        assert_eq!(response.status, 200);
        serde_json::from_slice(&response.body).expect("search route json")
    }

    fn wait_for_task_status(
        adapters: Arc<ServiceAdapters>,
        task_id: &str,
        expected: &[&str],
    ) -> Value {
        let mut last_payload = None;
        for _ in 0..200 {
            let payload = search_route(
                Arc::clone(&adapters),
                "GET",
                &format!("/api/model/search/{task_id}"),
                Vec::new(),
            );
            last_payload = Some(payload.clone());
            let status = payload
                .get("task")
                .and_then(|task| task.get("status"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if expected.contains(&status) {
                return payload;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "timed out waiting for task {task_id} to reach one of {expected:?}; last payload: {}",
            last_payload.unwrap_or_else(|| json!({ "task_id": task_id, "status": "missing" }))
        );
    }

    #[test]
    fn status_payload_marks_rust_backend() {
        let payload = status_payload();
        assert!(payload.ok);
        assert_eq!(payload.backend, "rust");
        assert_eq!(payload.model_engine, BACKEND_NAME);
    }

    #[test]
    fn route_status_returns_json() {
        let request = HttpRequest {
            method: "GET".to_string(),
            path: "/api/status".to_string(),
            body: Vec::new(),
        };
        let response = route_request(request, Arc::new(ServiceAdapters::default()));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("status json");
        assert_eq!(payload.get("backend").and_then(Value::as_str), Some("rust"));
    }

    #[test]
    fn route_default_structure_returns_ok() {
        let request = HttpRequest {
            method: "GET".to_string(),
            path: "/api/model/default-structure".to_string(),
            body: Vec::new(),
        };
        let response = route_request(request, Arc::new(ServiceAdapters::default()));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("default structure json");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert!(payload.get("structure").is_some());
    }

    #[test]
    fn route_stub_search_start_keeps_frontend_shape() {
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/model/search".to_string(),
            body: br#"{"params":{"ticks":800}}"#.to_vec(),
        };
        let response = route_request(request, Arc::new(ServiceAdapters::default()));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("search json");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert!(payload.get("task").and_then(|task| task.get("task_id")).is_some());
        assert!(payload.get("task").and_then(|task| task.get("status")).is_some());
        assert!(payload.get("task").and_then(|task| task.get("progress")).is_some());
    }

    #[test]
    fn simulate_route_preserves_water_start_state_even_with_slime_config() {
        let temp = TestDir::new();
        let mut structure = schema::default_structure();
        structure.name = Some("water-structure".to_string());
        structure.start.x = 0.125;
        structure.start.y = 0.0;
        structure.start.vx = 0.0;
        structure.start.vy = 0.0;
        structure.start.start_on_ground = Some(true);
        structure.start.initial_tick_count = 0;
        structure.launch_config = Some(schema::LaunchConfig {
            mode: None,
            slime_block_x: Some(-1.0),
            extra: BTreeMap::new(),
        });

        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/model/simulate".to_string(),
            body: serde_json::to_vec(&json!({
                "structure": structure,
                "options": {
                    "ticks": 40,
                    "launchMode": "water",
                    "label": "service-water-sim"
                }
            }))
            .expect("serialize simulate request"),
        };

        let response = route_request(request, real_adapters(&temp));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("simulate json");
        let run = payload.get("run").expect("simulate run");
        assert_eq!(
            run.get("summary")
                .and_then(|summary| summary.get("launch_mode"))
                .and_then(Value::as_str),
            Some("water")
        );
        assert_eq!(
            run.get("summary")
                .and_then(|summary| summary.get("requested_launch_mode"))
                .and_then(Value::as_str),
            Some("water")
        );
        let start = run
            .get("structure")
            .and_then(|structure| structure.get("start"))
            .expect("run structure start");
        assert!((start.get("x").and_then(Value::as_f64).unwrap_or_default() - 0.125).abs() < 1.0e-12);
        assert!((start.get("vx").and_then(Value::as_f64).unwrap_or_default() - 0.0).abs() < 1.0e-12);
        assert_eq!(
            run.get("structure")
                .and_then(|structure| structure.get("launch"))
                .and_then(Value::as_object),
            None
        );
    }

    #[test]
    fn simulate_route_matches_user_logged_piston_early_ticks() {
        let temp = TestDir::new();
        let mut structure = schema::default_structure();
        structure.name = Some("piston-early-log".to_string());
        structure.start.x = 0.75;
        structure.start.y = 0.0;
        structure.start.vx = 0.0;
        structure.start.vy = 0.0;
        structure.start.start_on_ground = Some(true);
        structure.start.initial_tick_count = 0;
        structure.launch_config = Some(schema::LaunchConfig {
            mode: Some("piston".to_string()),
            slime_block_x: Some(-1.0),
            extra: BTreeMap::new(),
        });
        structure.prefix = vec![
            schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            schema::make_cell(Some(7.0 / 9.0), -1, "glass", None, Some(7)),
            schema::make_cell(Some(8.0 / 9.0), -1, "glass", None, Some(8)),
            schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
        ];
        structure.cycle = vec![
            schema::make_cell(Some(8.0 / 9.0), 1, "packed_ice", None, Some(8)),
            schema::make_cell(Some(7.0 / 9.0), 1, "packed_ice", None, Some(7)),
            schema::make_cell(None, 0, "blue_ice", None, Some(0)),
            schema::make_cell(Some(8.0 / 9.0), 0, "blue_ice", None, Some(8)),
        ];

        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/model/simulate".to_string(),
            body: serde_json::to_vec(&json!({
                "structure": structure,
                "options": {
                    "ticks": 6,
                    "targetSpeed": 0.5,
                    "targetDwellTicks": 2,
                    "launchMode": "piston",
                    "label": "piston-log-regression"
                }
            }))
            .expect("serialize simulate request"),
        };

        let response = route_request(request, real_adapters(&temp));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("simulate json");
        let run = payload.get("run").expect("simulate run");
        let summary = run.get("summary").expect("run summary");
        let points = run
            .get("points")
            .and_then(Value::as_array)
            .expect("run points");

        assert_eq!(
            summary.get("launch_mode").and_then(Value::as_str),
            Some("piston")
        );
        assert_close(
            summary
                .get("launch_effective_start_x")
                .and_then(Value::as_f64)
                .expect("launch_effective_start_x"),
            1.135,
            1.0e-12,
            "launch_effective_start_x",
        );

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
            assert_close(
                points[index]
                    .get("x_raw")
                    .and_then(Value::as_f64)
                    .expect("point x_raw"),
                *expected,
                1.0e-9,
                &format!("point[{index}].x_raw"),
            );
        }
    }

    #[test]
    fn compare_route_reports_legacy_unavailable_in_rust_only_backend() {
        let temp = TestDir::new();
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/model/compare".to_string(),
            body: serde_json::to_vec(&json!({
                "structure": schema::default_structure(),
                "options": {
                    "ticks": 40,
                    "launchMode": "water"
                }
            }))
            .expect("serialize compare request"),
        };

        let response = route_request(request, real_adapters(&temp));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("compare json");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(false));
        assert_eq!(
            payload.get("error").and_then(Value::as_str),
            Some("legacy_compare_unavailable")
        );
        assert_eq!(
            payload.get("rust_engine").and_then(Value::as_str),
            Some(BACKEND_NAME)
        );
    }

    #[test]
    fn simulate_route_returns_rust_run_for_default_structure() {
        let temp = TestDir::new();

        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/model/simulate".to_string(),
            body: serde_json::to_vec(&json!({
                "structure": schema::default_structure(),
                "options": {
                    "ticks": 60,
                    "targetSpeed": 0.5,
                    "targetDwellTicks": 2,
                    "launchMode": "water",
                    "label": "default-structure-smoke"
                }
            }))
            .expect("serialize simulate request"),
        };

        let response = route_request(request, real_adapters(&temp));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("simulate json");
        let run = payload.get("run").expect("simulate run");
        let summary = run.get("summary").expect("run summary");

        assert_eq!(
            summary.get("model_engine").and_then(Value::as_str),
            Some(BACKEND_NAME)
        );
        assert_eq!(
            summary.get("launch_mode").and_then(Value::as_str),
            Some("water")
        );
        assert_eq!(
            summary
                .get("requested_launch_mode")
                .and_then(Value::as_str),
            Some("water")
        );
        assert_eq!(
            summary.get("sample_count").and_then(Value::as_u64),
            Some(61)
        );
        assert_eq!(
            summary.get("duration_gt").and_then(Value::as_u64),
            Some(60)
        );
        assert!(summary.get("avg_derived_speed").and_then(Value::as_f64).is_some());
        assert!(summary.get("end_x_raw").and_then(Value::as_f64).is_some());
        assert!(run.get("points").and_then(Value::as_array).is_some());
    }

    #[test]
    fn preprocess_search_request_uses_dry_piston_placeholder_start_state() {
        let params = serde_json::from_str::<Value>(
            r#"{
                "ticks": 800,
                "launchMode": "piston",
                "slimeBlockX": -1,
                "startX": 0.125,
                "startY": 0.0,
                "startVX": 0.0,
                "startVY": 0.0
            }"#,
        )
        .expect("search params json");
        let params = params.as_object().expect("params object");

        let execution = preprocess_search_request(params).expect("preprocess piston search");

        assert!((execution.start_x - (-0.365)).abs() < 1.0e-12);
        assert!((execution.start_y - 0.0).abs() < 1.0e-12);
        assert!((execution.start_vx - 1.0).abs() < 1.0e-12);
        assert!((execution.start_vy - (-0.04)).abs() < 1.0e-12);
        assert!(!execution.start_on_ground);
        assert!((execution.origin_x - 0.0).abs() < 1.0e-12);
        assert_eq!(
            execution
                .launch
                .as_ref()
                .and_then(|launch| launch.get("collisionGt"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            execution
                .launch
                .as_ref()
                .and_then(|launch| launch.get("effectiveLocalStart"))
                .and_then(Value::as_object)
                .and_then(|start| start.get("initialTickCount"))
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn preprocess_structure_for_launch_recomputes_piston_from_raw_start() {
        let mut structure = schema::default_structure();
        structure.prefix = vec![
            schema::make_cell(None, 0, "packed_ice", None, Some(0)),
            schema::make_cell(Some(7.0 / 9.0), -1, "glass", None, Some(7)),
            schema::make_cell(Some(8.0 / 9.0), -1, "glass", None, Some(8)),
        ];
        structure.cycle = vec![schema::make_cell(None, 0, "packed_ice", None, Some(0))];
        structure.start.x = 1.135;
        structure.start.y = 0.0;
        structure.start.vx = 1.0;
        structure.start.vy = -0.08;
        structure.start.start_on_ground = Some(false);
        structure.start.initial_tick_count = 2;
        structure.launch_config = Some(schema::LaunchConfig {
            mode: Some("piston".to_string()),
            slime_block_x: Some(-1.0),
            extra: BTreeMap::new(),
        });
        structure.extra.insert(
            "launch".to_string(),
            json!({
                "mode": "piston",
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
                    "x": 99.0,
                    "y": 0.0,
                    "vx": 0.25,
                    "vy": 0.0,
                    "startOnGround": true,
                    "entityIdMod4": 0,
                    "initialTickCount": 99
                }
            }),
        );

        let updated =
            preprocess_structure_for_launch(&structure, "piston").expect("recompute piston launch");

        assert!((updated.start.x - 0.135).abs() < 1.0e-12);
        assert!((updated.start.vx - 1.0).abs() < 1.0e-12);
        assert!((updated.start.vy - (-0.08)).abs() < 1.0e-12);
        assert_eq!(updated.start.initial_tick_count, 2);
        assert!((updated.origin_x - 0.0).abs() < 1.0e-12);
        let launch = updated
            .extra
            .get("launch")
            .and_then(Value::as_object)
            .expect("updated launch");
        assert_eq!(
            launch
                .get("rawStart")
                .and_then(Value::as_object)
                .and_then(|raw| raw.get("x"))
                .and_then(Value::as_f64),
            Some(0.75)
        );
        assert_eq!(
            launch
                .get("effectiveStart")
                .and_then(Value::as_object)
                .and_then(|effective| effective.get("x"))
                .and_then(Value::as_f64),
            Some(1.135)
        );
        assert_eq!(
            launch
                .get("effectiveLocalStart")
                .and_then(Value::as_object)
                .and_then(|effective| effective.get("x"))
                .and_then(Value::as_f64),
            Some(0.135)
        );
        assert_eq!(
            launch
                .get("displayOriginX")
                .and_then(Value::as_f64),
            Some(1.0)
        );
    }

    #[test]
    fn build_search_structure_for_piston_search_matches_user_logged_early_ticks() {
        let params = serde_json::from_str::<Value>(
            r#"{
                "ticks": 6,
                "launchMode": "piston",
                "slimeBlockX": -1,
                "startX": 0.75,
                "startY": 0.0,
                "startVX": 0.0,
                "startVY": 0.0,
                "startOnGround": true,
                "entityIdMod4": 0,
                "initialTickCount": 0
            }"#,
        )
        .expect("search params json");
        let execution = preprocess_search_request(params.as_object().expect("params object"))
            .expect("preprocess piston search");
        let row = crate::reachable_candidates::ServiceCandidateRow {
            id: "test-row".to_string(),
            mode: "launch-fast".to_string(),
            cycle: "F2-I_D1-B_S1-B_D1-I_F2-I_D1-B_S1-B".to_string(),
            prefix_label: "DI-R2N-DI-F2I-D2B".to_string(),
            score: 1.0,
            entity_id_mod4: 0,
            initial_tick_mod4: 2,
            strict_hit_rate: Some(1.0),
            target_hit_rate: Some(1.0),
            raw_short_hit_rate: Some(1.0),
            average_speed: 0.5,
            dwell_window: json!({
                "mode": "cycle",
                "minBlock": 8,
                "maxBlock": 12,
                "minStartTick": 0,
                "includeFinalGroup": false
            }),
            prefix_cells: vec![
                crate::CellDescription { index: 0, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "packed_ice".to_string(), code: "D-I".to_string() },
                crate::CellDescription { index: 1, surface: Some(7.0 / 9.0), flow: -1, derived_flow_hint: None, amount: 7, floor: "normal".to_string(), code: "R7-N".to_string() },
                crate::CellDescription { index: 2, surface: Some(8.0 / 9.0), flow: -1, derived_flow_hint: None, amount: 8, floor: "normal".to_string(), code: "R8-N".to_string() },
                crate::CellDescription { index: 3, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "packed_ice".to_string(), code: "D-I".to_string() },
                crate::CellDescription { index: 4, surface: Some(8.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 8, floor: "packed_ice".to_string(), code: "F8-I".to_string() },
                crate::CellDescription { index: 5, surface: Some(7.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 7, floor: "packed_ice".to_string(), code: "F7-I".to_string() },
                crate::CellDescription { index: 6, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "blue_ice".to_string(), code: "D-B".to_string() },
                crate::CellDescription { index: 7, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "blue_ice".to_string(), code: "D-B".to_string() },
            ],
            cycle_cells: vec![
                crate::CellDescription { index: 0, surface: Some(8.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 8, floor: "packed_ice".to_string(), code: "F8-I".to_string() },
                crate::CellDescription { index: 1, surface: Some(7.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 7, floor: "packed_ice".to_string(), code: "F7-I".to_string() },
                crate::CellDescription { index: 2, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "blue_ice".to_string(), code: "D-B".to_string() },
                crate::CellDescription { index: 3, surface: Some(8.0 / 9.0), flow: 0, derived_flow_hint: None, amount: 8, floor: "blue_ice".to_string(), code: "S8-B".to_string() },
                crate::CellDescription { index: 4, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "packed_ice".to_string(), code: "D-I".to_string() },
                crate::CellDescription { index: 5, surface: Some(8.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 8, floor: "packed_ice".to_string(), code: "F8-I".to_string() },
                crate::CellDescription { index: 6, surface: Some(7.0 / 9.0), flow: 1, derived_flow_hint: None, amount: 7, floor: "packed_ice".to_string(), code: "F7-I".to_string() },
                crate::CellDescription { index: 7, surface: None, flow: 0, derived_flow_hint: None, amount: 0, floor: "blue_ice".to_string(), code: "D-B".to_string() },
                crate::CellDescription { index: 8, surface: Some(8.0 / 9.0), flow: 0, derived_flow_hint: None, amount: 8, floor: "blue_ice".to_string(), code: "S8-B".to_string() },
            ],
        };

        let structure =
            build_search_structure(&row, &execution).expect("build piston search structure");
        assert!((structure.start.x - 0.135).abs() < 1.0e-12);
        let launch = structure
            .extra
            .get("launch")
            .and_then(Value::as_object)
            .expect("piston launch");
        assert_eq!(
            launch
                .get("effectiveLocalStart")
                .and_then(Value::as_object)
                .and_then(|value| value.get("x"))
                .and_then(Value::as_f64),
            Some(0.135)
        );

        let points = viewer_runs::simulate_viewer_points_for_requested_duration(&structure, 6)
            .expect("simulate search structure");
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
            assert_close(
                points[index].x_raw.expect("x_raw"),
                *expected,
                1.0e-12,
                &format!("point[{index}].x_raw"),
            );
        }
    }

    #[test]
    fn route_search_start_uses_real_unique_task_ids() {
        let adapters = Arc::new(ServiceAdapters::default());
        let first = search_route(
            Arc::clone(&adapters),
            "POST",
            "/api/model/search",
            br#"{"params":{"ticks":800}}"#.to_vec(),
        );
        let second = search_route(
            Arc::clone(&adapters),
            "POST",
            "/api/model/search",
            br#"{"params":{"ticks":801}}"#.to_vec(),
        );

        let first_id = first
            .get("task")
            .and_then(|task| task.get("task_id"))
            .and_then(Value::as_str)
            .expect("first task id");
        let second_id = second
            .get("task")
            .and_then(|task| task.get("task_id"))
            .and_then(Value::as_str)
            .expect("second task id");

        assert_ne!(first_id, "stub-search-task");
        assert_ne!(second_id, "stub-search-task");
        assert_ne!(first_id, second_id);
    }

    #[test]
    fn route_search_get_and_cancel_use_real_task_lifecycle() {
        let adapters = Arc::new(ServiceAdapters::default());
        let started = search_route(
            Arc::clone(&adapters),
            "POST",
            "/api/model/search",
            br#"{"params":{"ticks":800}}"#.to_vec(),
        );
        let task_id = started
            .get("task")
            .and_then(|task| task.get("task_id"))
            .and_then(Value::as_str)
            .expect("started task id")
            .to_string();

        let observed = wait_for_task_status(
            Arc::clone(&adapters),
            &task_id,
            &["queued", "running", "completed"],
        );
        assert_eq!(observed.get("ok").and_then(Value::as_bool), Some(true));

        let cancelled = search_route(
            Arc::clone(&adapters),
            "POST",
            &format!("/api/model/search/{task_id}/cancel"),
            Vec::new(),
        );
        let cancelled_status = cancelled
            .get("task")
            .and_then(|task| task.get("status"))
            .and_then(Value::as_str)
            .expect("cancelled task status");
        assert!(matches!(cancelled_status, "running" | "cancelled" | "completed"));

        let final_payload = wait_for_task_status(
            Arc::clone(&adapters),
            &task_id,
            &["cancelled", "completed"],
        );
        let final_task = final_payload.get("task").expect("final task");
        let final_status = final_task
            .get("status")
            .and_then(Value::as_str)
            .expect("final status");
        assert!(matches!(final_status, "cancelled" | "completed"));
        assert_eq!(
            final_task
                .get("task_id")
                .and_then(Value::as_str)
                .expect("final task id"),
            task_id
        );
    }

    #[test]
    fn route_runs_uses_real_run_store_adapter() {
        let temp = TestDir::new();
        let store = RunStore::new(temp.viewer_data_dir());
        store
            .save_runs(&ViewerRunsPayload {
                updated_at: None,
                latest_run_id: None,
                run_count: 0,
                runs: vec![ViewerRun {
                    run_id: Some(920001),
                    label: Some("real-run".to_string()),
                    display_label: Some("Real Run".to_string()),
                    summary: ViewerRunSummary {
                        source: Some("service-test".to_string()),
                        deleted: false,
                        ..ViewerRunSummary::default()
                    },
                    points: Vec::new(),
                    structure: Some(schema::default_structure()),
                    extra: BTreeMap::new(),
                }],
            })
            .expect("seed runs store");

        let request = HttpRequest {
            method: "GET".to_string(),
            path: "/api/runs".to_string(),
            body: Vec::new(),
        };
        let response = route_request(request, real_adapters(&temp));
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("runs json");
        assert_eq!(payload.get("run_count").and_then(Value::as_u64), Some(1));
        assert_eq!(
            payload
                .get("runs")
                .and_then(Value::as_array)
                .and_then(|runs| runs.first())
                .and_then(|run| run.get("label"))
                .and_then(Value::as_str),
            Some("real-run")
        );
    }

    #[test]
    fn completed_search_results_are_visible_via_runs_endpoint() {
        let temp = TestDir::new();
        let adapters = real_adapters(&temp);
        let started = search_route(
            Arc::clone(&adapters),
            "POST",
            "/api/model/search",
            br#"{"params":{"mode":"early","ticks":17,"keep":1,"maxPrefixCells":2,"minHitRate":0,"startX":0.875,"startVX":1.0}}"#.to_vec(),
        );
        let task_id = started
            .get("task")
            .and_then(|task| task.get("task_id"))
            .and_then(Value::as_str)
            .expect("search task id")
            .to_string();

        let completed = wait_for_task_status(Arc::clone(&adapters), &task_id, &["completed"]);
        let task = completed.get("task").expect("completed task");
        let result = task.get("result").expect("task result");
        let created = result
            .get("created")
            .and_then(Value::as_array)
            .expect("created array");
        assert!(!created.is_empty(), "search should write at least one run");

        let runs_response = route_request(
            HttpRequest {
                method: "GET".to_string(),
                path: "/api/runs".to_string(),
                body: Vec::new(),
            },
            adapters,
        );
        assert_eq!(runs_response.status, 200);
        let runs_payload: Value =
            serde_json::from_slice(&runs_response.body).expect("runs response json");
        let runs = runs_payload
            .get("runs")
            .and_then(Value::as_array)
            .expect("runs array");
        assert_eq!(runs.len(), created.len());
        let first = runs.first().expect("first search run");
        assert_eq!(
            first.get("summary")
                .and_then(|summary| summary.get("source"))
                .and_then(Value::as_str),
            Some("reachability-search")
        );
        assert!(first.get("display_label").and_then(Value::as_str).is_some());
        assert!(first.get("structure").is_some());
    }

    #[test]
    fn route_litematic_endpoints_use_real_rust_adapter() {
        let temp = TestDir::new();
        let export_request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/litematic/export".to_string(),
            body: serde_json::to_vec(&json!({
                "structure": schema::default_structure(),
                "options": {
                    "cycleRepeat": 2,
                    "name": "service-test"
                }
            }))
            .expect("serialize export request"),
        };
        let export_response = route_request(export_request, real_adapters(&temp));
        assert_eq!(export_response.status, 200);
        assert_eq!(export_response.content_type, "application/octet-stream");
        assert!(!export_response.body.is_empty());

        let import_request = HttpRequest {
            method: "POST".to_string(),
            path: "/api/litematic/import?floorY=0&fluidY=1&z=0".to_string(),
            body: export_response.body,
        };
        let import_response = route_request(import_request, real_adapters(&temp));
        assert_eq!(import_response.status, 200);
        let payload: Value = serde_json::from_slice(&import_response.body).expect("import json");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert!(payload.get("structure").is_some());
        assert!(payload.get("region").is_some());
        assert!(payload.get("unknownBlocks").is_some());
    }
}
